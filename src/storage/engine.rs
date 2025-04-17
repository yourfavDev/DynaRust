use actix_web::{web, HttpRequest, HttpResponse, Responder};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use web::Json;
use crate::storage::subscription::{KeyEvent, SubscriptionManager};
use jsonwebtoken::{decode, DecodingKey, Validation};

//
// CONSTANTS & TYPES
//

// In production, do not hardcode your secret but load it from a secure source.
const JWT_SECRET: &[u8] = b"kajdOsndmalskfi";

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

//
// CORE DATA STRUCTURES
//

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedValue {
    pub value: HashMap<String, Value>,
    pub version: u64,
    pub timestamp: u128,
    pub owner: String,
}

impl VersionedValue {
    pub fn new(value: HashMap<String, Value>, owner: String) -> Self {
        let ts = current_timestamp();
        Self {
            value,
            version: 1,
            timestamp: ts,
            owner,
        }
    }

    pub fn update(&mut self, value: HashMap<String, Value>) {
        self.value = value;
        self.version += 1;
        self.timestamp = current_timestamp();
    }
}

/// The local in-memory database state.
pub struct AppState {
    pub base_dir: &'static str,
    /// Maps table names to the table's key-value store.
    pub store: RwLock<HashMap<String, HashMap<String, VersionedValue>>>,
}

/// Shared cluster data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeStatus {
    Active,
    Suspect,
    Down,
    Leaving,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub status: NodeStatus,
    pub last_heartbeat: u128,
}

#[derive(Clone)]
pub struct ClusterData {
    // Mapping: node address -> NodeInfo
    pub nodes: std::sync::Arc<RwLock<HashMap<String, NodeInfo>>>,
}

#[derive(Deserialize)]
pub struct KeysRequest {
    pub keys: Vec<String>,
}

#[derive(Deserialize)]
pub struct JoinRequest {
    node: String,
    secret: String,
}

//
// UTILITY FUNCTIONS
//

/// Get current timestamp in milliseconds.
pub fn current_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

/// Extract active nodes from the cluster membership map.
fn get_active_nodes(nodes: &HashMap<String, NodeInfo>) -> Vec<String> {
    nodes
        .iter()
        .filter(|(_, info)| info.status == NodeStatus::Active)
        .map(|(k, _)| k.clone())
        .collect()
}

/// Computes replication targets for a given key.
fn get_replication_nodes(key: &str, nodes: &[String], replication_factor: usize) -> Vec<String> {
    if nodes.is_empty() {
        return Vec::new();
    }

    // Hash the key to determine ordering
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let total_nodes = nodes.len();
    let primary_index = (hash % (total_nodes as u64)) as usize;

    // Select targets up to replication factor
    let mut targets = Vec::with_capacity(replication_factor.min(total_nodes));
    targets.push(nodes[primary_index].clone());
    let mut i = 1;
    while targets.len() < replication_factor && i < total_nodes {
        let idx = (primary_index + i) % total_nodes;
        targets.push(nodes[idx].clone());
        i += 1;
    }
    targets
}

/// Helper function to extract the user (subject) from the JWT token in the Authorization header.
fn extract_user_from_token(req: &HttpRequest) -> Result<String, HttpResponse> {
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = auth_str.trim_start_matches("Bearer ").trim();
                match decode::<Claims>(
                    token,
                    &DecodingKey::from_secret(JWT_SECRET),
                    &Validation::default(),
                ) {
                    Ok(token_data) => Ok(token_data.claims.sub),
                    Err(_) => Err(HttpResponse::Unauthorized().json(json!({
                        "error": "Invalid JWT token"
                    }))),
                }
            } else {
                Err(HttpResponse::Unauthorized().json(json!({
                    "error": "Invalid authorization header format"
                })))
            }
        } else {
            Err(HttpResponse::Unauthorized().json(json!({
                "error": "Invalid authorization header"
            })))
        }
    } else {
        Err(HttpResponse::Unauthorized().json(json!({
            "error": "Missing Authorization header"
        })))
    }
}

//
// DATA OPERATION HANDLERS
//

/// GET handler for reading a key's value with replication.
pub async fn get_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
) -> impl Responder {
    // For internal replication requests, do local lookup.
    if req.headers().contains_key("X-Internal-Request") {
        let (table_name, key_val) = path.into_inner();
        let store = state.store.read().await;
        let result = store
            .get(&table_name)
            .and_then(|table| table.get(&key_val))
            .cloned();
        return HttpResponse::Ok().json(result);
    }

    let (table_name, key_val) = path.into_inner();

    // For external requests, request replicas.
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    let client = reqwest::Client::new();
    let mut futures_vec: Vec<BoxFuture<'static, Result<Option<VersionedValue>, String>>> =
        Vec::new();

    for target in targets.into_iter() {
        if target == *current_addr.get_ref() {
            let state_clone = state.clone();
            let table_name_clone = table_name.clone();
            let key_clone = key_val.clone();
            let fut: BoxFuture<'static, Result<Option<VersionedValue>, String>> =
                Box::pin(async move {
                    let store = state_clone.store.read().await;
                    let result = store
                        .get(&table_name_clone)
                        .and_then(|table| table.get(&key_clone))
                        .cloned();
                    Ok(result)
                });
            futures_vec.push(fut);
        } else {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_inner = client.clone();
            let fut: BoxFuture<'static, Result<Option<VersionedValue>, String>> =
                Box::pin(async move {
                    match client_inner
                        .get(&url)
                        .header("X-Internal-Request", "true")
                        .timeout(std::time::Duration::from_secs(3))
                        .send()
                        .await
                    {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                match resp.json::<Option<VersionedValue>>().await {
                                    Ok(v) => Ok(v),
                                    Err(e) => Err(format!(
                                        "Error parsing JSON from {}: {}",
                                        target, e
                                    )),
                                }
                            } else {
                                Err(format!("Error from {}: {}", target, resp.status()))
                            }
                        }
                        Err(e) => Err(format!("Request error to {}: {}", target, e)),
                    }
                });
            futures_vec.push(fut);
        }
    }

    let results = futures_util::future::join_all(futures_vec).await;
    let mut valid_results = Vec::new();
    for res in results {
        if let Ok(Some(val)) = res {
            valid_results.push(val);
        } else if let Err(err) = res {
            println!("Error during read: {}", err);
        }
    }

    if valid_results.is_empty() {
        return HttpResponse::NotFound().json(json!({
            "error": "Key not found on any replica"
        }));
    }

    let latest = valid_results
        .into_iter()
        .max_by(|a, b| a.version.cmp(&b.version))
        .unwrap();
    HttpResponse::Ok().json(latest)
}

/// PUT handler: Inserts or updates a key–value pair with ownership verification.
/// For external requests, a valid JWT token in the Authorization header is required.
pub async fn put_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
    body: web::Json<HashMap<String, Value>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // Internal replication requests bypass authentication.
    if req.headers().contains_key("X-Internal-Request") {
        let new_value = {
            let mut store = state.store.write().await;
            let table = store.entry(table_name.clone()).or_insert_with(HashMap::new);
            if let Some(existing) = table.get_mut(&key_val) {
                existing.update(body.0.clone());
                existing.clone()
            } else {
                // For internal requests, set owner as "internal"
                let v = VersionedValue::new(body.0.clone(), "internal".to_string());
                table.insert(key_val.clone(), v.clone());
                v
            }
        };
        sub_manager
            .notify(&table_name, &key_val, KeyEvent::Updated(new_value.clone()))
            .await;
        return HttpResponse::Created().json(new_value);
    }

    // External requests require a valid JWT token.
    let user = match extract_user_from_token(&req) {
        Ok(u) => u,
        Err(err_resp) => return err_resp,
    };

    let new_value = {
        let mut store = state.store.write().await;
        let table = store.entry(table_name.clone()).or_insert_with(HashMap::new);
        if let Some(existing) = table.get_mut(&key_val) {
            // Only allow the owner to update this record.
            if existing.owner != user {
                return HttpResponse::Unauthorized().json(json!({
                    "error": "Not authorized to update this record"
                }));
            }
            existing.update(body.0.clone());
            existing.clone()
        } else {
            let v = VersionedValue::new(body.0.clone(), user.clone());
            table.insert(key_val.clone(), v.clone());
            v
        }
    };

    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Updated(new_value.clone()))
        .await;

    // Replicate to other nodes.
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    let client = reqwest::Client::new();
    let mut replication_futures = Vec::new();
    for target in targets {
        if target != *current_addr.get_ref() {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_clone = client.clone();
            let value_clone = new_value.clone();
            let fut = async move {
                let result = client_clone
                    .put(&url)
                    .header("X-Internal-Request", "true")
                    .json(&value_clone)
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await;
                if let Err(e) = result {
                    println!("Replication error to {}: {}", target, e);
                }
            };
            replication_futures.push(fut);
        }
    }
    tokio::spawn(async move {
        futures_util::future::join_all(replication_futures).await;
    });
    HttpResponse::Created().json(new_value)
}

/// DELETE handler: Removes a key with an ownership check.
pub async fn delete_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // Internal requests bypass auth.
    if req.headers().contains_key("X-Internal-Request") {
        let mut store = state.store.write().await;
        if let Some(table) = store.get_mut(&table_name) {
            table.remove(&key_val);
        }
        sub_manager
            .notify(&table_name, &key_val, KeyEvent::Deleted)
            .await;
        return HttpResponse::Ok().json(json!({"message": "Deleted"}));
    }

    // External requests require a valid JWT token.
    let user = match extract_user_from_token(&req) {
        Ok(u) => u,
        Err(err_resp) => return err_resp,
    };

    // Check if the requester is the owner.
    let authorized = {
        let store = state.store.read().await;
        if let Some(table) = store.get(&table_name) {
            if let Some(existing) = table.get(&key_val) {
                existing.owner == user
            } else {
                false
            }
        } else {
            false
        }
    };

    if !authorized {
        return HttpResponse::Unauthorized().json(json!({
            "error": "Not authorized to delete this record or record not found"
        }));
    }

    let local_status = {
        let mut store = state.store.write().await;
        if let Some(table) = store.get_mut(&table_name) {
            if table.remove(&key_val).is_some() {
                "Deleted locally"
            } else {
                "Key not found locally"
            }
        } else {
            "Table not found locally"
        }
            .to_string()
    };

    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Deleted)
        .await;

    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    let client = reqwest::Client::new();
    let mut replication_futures = Vec::new();
    for target in targets {
        if target != *current_addr.get_ref() {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_clone = client.clone();
            let fut = async move {
                let result = client_clone
                    .delete(&url)
                    .header("X-Internal-Request", "true")
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await;
                if let Err(e) = result {
                    println!("Deletion replication error to {}: {}", target, e);
                }
            };
            replication_futures.push(fut);
        }
    }
    tokio::spawn(async move {
        futures_util::future::join_all(replication_futures).await;
    });
    HttpResponse::Ok().json(json!({"message": local_status}))
}

//
// TABLE MANAGEMENT HANDLERS
//

pub async fn get_table_store(
    table: web::Path<String>,
    state: web::Data<AppState>,
) -> impl Responder {
    let table_name = table.into_inner();
    let store = state.store.read().await;
    if let Some(table_db) = store.get(&table_name) {
        HttpResponse::Ok().json(table_db)
    } else {
        HttpResponse::NotFound().json(json!({"error": "Table not found"}))
    }
}

pub async fn get_all_keys(
    table: web::Path<String>,
    state: web::Data<AppState>,
) -> impl Responder {
    let table_name = table.into_inner();
    let store = state.store.read().await;
    if let Some(table_db) = store.get(&table_name) {
        let keys: Vec<String> = table_db.keys().cloned().collect();
        HttpResponse::Ok().json(keys)
    } else {
        HttpResponse::NotFound().json(json!({"error": "Table not found"}))
    }
}

pub async fn get_multiple_keys(
    table: web::Path<String>,
    keys_request: Json<KeysRequest>,
    state: web::Data<AppState>,
) -> impl Responder {
    let table_name = table.into_inner();
    let requested_keys = keys_request.into_inner().keys;
    let store = state.store.read().await;

    if let Some(table_data) = store.get(&table_name) {
        let mut result: HashMap<String, VersionedValue> = HashMap::new();
        for key in requested_keys {
            if let Some(value) = table_data.get(&key) {
                result.insert(key, value.clone());
            }
        }
        HttpResponse::Ok().json(result)
    } else {
        HttpResponse::NotFound().json(json!({"error": "Table not found"}))
    }
}

//
// CLUSTER MANAGEMENT HANDLERS
//

pub async fn join_cluster(
    cluster: web::Data<ClusterData>,
    request: Json<JoinRequest>,
) -> impl Responder {
    let cluster_secret = env::var("CLUSTER_SECRET")
        .unwrap_or_else(|_| "default_secret".to_string());

    if request.secret != cluster_secret {
        return HttpResponse::Unauthorized().body("Invalid cluster secret");
    }

    let new_node = request.node.clone();
    let mut nodes_guard = cluster.nodes.write().await;
    if !nodes_guard.contains_key(&new_node) {
        nodes_guard.insert(
            new_node.clone(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
    }
    HttpResponse::Ok().json(nodes_guard.clone())
}

pub async fn get_global_store(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> impl Responder {
    let valid_api_key = env::var("CLUSTER_SECRET")
        .unwrap_or_else(|_| "default_secret".to_string());
    match req.headers().get("x-api-key") {
        Some(header_value) => {
            if let Ok(api_key) = header_value.to_str() {
                if api_key != valid_api_key {
                    return HttpResponse::Unauthorized().body("Invalid API key provided");
                }
            } else {
                return HttpResponse::BadRequest().body("Malformed API key header");
            }
        }
        None => {
            return HttpResponse::Unauthorized().body("Missing API key");
        }
    };
    let store = state.store.read().await;
    HttpResponse::Ok().json(store.clone())
}

//
// UNIT TESTS
//
#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, web};
    use actix_web::dev::ServiceResponse;
    use serde_json::json;
    use std::collections::HashMap;
    use std::env;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // For testing purposes, implement PartialEq and Eq for VersionedValue.
    impl PartialEq for VersionedValue {
        fn eq(&self, other: &Self) -> bool {
            self.value == other.value
                && self.version == other.version
                && self.timestamp == other.timestamp
                && self.owner == other.owner
        }
    }
    impl Eq for VersionedValue {}

    #[actix_web::test]
    async fn test_current_timestamp() {
        let ts = current_timestamp();
        assert!(ts > 0);
    }

    #[actix_web::test]
    async fn test_get_active_nodes() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: 100,
            },
        );
        nodes.insert(
            "b".to_string(),
            NodeInfo {
                status: NodeStatus::Down,
                last_heartbeat: 200,
            },
        );
        let active = get_active_nodes(&nodes);
        assert_eq!(active, vec!["a".to_string()]);
    }

    #[actix_web::test]
    async fn test_get_replication_nodes() {
        let nodes = vec![
            "n1".to_string(),
            "n2".to_string(),
            "n3".to_string(),
        ];
        let result = get_replication_nodes("test_key", &nodes, 2);
        assert_eq!(result.len(), 2);
        for node in result {
            assert!(nodes.contains(&node));
        }
    }

    #[actix_web::test]
    async fn test_versioned_value_new_and_update() {
        let mut value_map = HashMap::new();
        value_map.insert("field".to_string(), json!("initial"));
        let mut v = VersionedValue::new(value_map.clone(), "user1".to_string());
        assert_eq!(v.version, 1);
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        let mut new_map = HashMap::new();
        new_map.insert("field".to_string(), json!("updated"));
        v.update(new_map.clone());
        assert_eq!(v.version, 2);
        assert_eq!(v.value, new_map);
    }

    #[actix_web::test]
    async fn test_get_table_store() {
        let mut value_map = HashMap::new();
        value_map.insert("foo".to_string(), json!("bar"));
        let versioned = VersionedValue::new(value_map, "user1".to_string());
        let mut table = HashMap::new();
        table.insert("key1".to_string(), versioned.clone());
        let mut store_map = HashMap::new();
        store_map.insert("table1".to_string(), table);
        let state = web::Data::new(AppState {
            base_dir: "/tmp",
            store: RwLock::new(store_map),
        });
        let path = web::Path::from("table1".to_string());
        let response = get_table_store(path, state).await;
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        let service_resp = ServiceResponse::new(dummy_req, http_resp);
        assert_eq!(service_resp.response().status(), 200);
        let table_out: HashMap<String, VersionedValue> =
            test::read_body_json(service_resp).await;
        assert!(table_out.contains_key("key1"));
    }

    #[actix_web::test]
    async fn test_get_all_keys() {
        let mut table = HashMap::new();
        table.insert(
            "key1".to_string(),
            VersionedValue::new(
                HashMap::from([("k".to_string(), json!("v"))]),
                "user1".to_string(),
            ),
        );
        table.insert("key2".to_string(), VersionedValue::new(HashMap::new(), "user1".to_string()));
        let mut store_map = HashMap::new();
        store_map.insert("table1".to_string(), table);
        let state = web::Data::new(AppState {
            base_dir: "/tmp",
            store: RwLock::new(store_map),
        });
        let path = web::Path::from("table1".to_string());
        let response = get_all_keys(path, state).await;
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        let service_resp = ServiceResponse::new(dummy_req, http_resp);
        assert_eq!(service_resp.response().status(), 200);
        let keys: Vec<String> = test::read_body_json(service_resp).await;
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
    }

    #[actix_web::test]
    async fn test_get_multiple_keys() {
        let versioned1 =
            VersionedValue::new(HashMap::from([("a".to_string(), json!("1"))]), "user1".to_string());
        let versioned2 =
            VersionedValue::new(HashMap::from([("b".to_string(), json!("2"))]), "user1".to_string());
        let mut table = HashMap::new();
        table.insert("k1".to_string(), versioned1.clone());
        table.insert("k2".to_string(), versioned2.clone());
        let mut store_map = HashMap::new();
        store_map.insert("table1".to_string(), table);
        let state = web::Data::new(AppState {
            base_dir: "/tmp",
            store: RwLock::new(store_map),
        });
        let keys_request = KeysRequest {
            keys: vec!["k1".to_string(), "k3".to_string()],
        };
        let json_req = web::Json(keys_request);
        let path = web::Path::from("table1".to_string());
        let response = get_multiple_keys(path, json_req, state).await;
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        let service_resp = ServiceResponse::new(dummy_req, http_resp);
        assert_eq!(service_resp.response().status(), 200);
        let result: HashMap<String, VersionedValue> =
            test::read_body_json(service_resp).await;
        assert!(result.contains_key("k1"));
        assert!(!result.contains_key("k3"));
    }

    #[actix_web::test]
    async fn test_put_value_internal() {
        let state = web::Data::new(AppState {
            base_dir: "/tmp",
            store: RwLock::new(HashMap::new()),
        });
        let cluster_data = web::Data::new(ClusterData {
            nodes: Arc::new(RwLock::new(HashMap::new())),
        });
        let current_addr = web::Data::new("127.0.0.1:8080".to_string());
        let sub_manager = web::Data::new(SubscriptionManager { channels: Default::default() });
        let mut body_map = HashMap::new();
        body_map.insert("key".to_string(), json!("value"));
        let body = web::Json(body_map.clone());
        let req = test::TestRequest::default()
            .insert_header(("X-Internal-Request", "true"))
            .to_http_request();
        let path = web::Path::from(("test".to_string(), "k1".to_string()));
        let response = put_value(
            req,
            path,
            state.clone(),
            cluster_data,
            current_addr,
            sub_manager,
            body,
        )
            .await;
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        let service_resp = ServiceResponse::new(dummy_req, http_resp);
        assert_eq!(service_resp.response().status(), 201);
        let body_resp: VersionedValue = test::read_body_json(service_resp).await;
        assert_eq!(body_resp.value, body_map);
        let store_read = state.store.read().await;
        let table = store_read.get("test").unwrap();
        let v = table.get("k1").unwrap();
        assert_eq!(v.value, body_map);
        assert_eq!(v.version, 1);
    }

    // Additional tests for external endpoints with JWT tokens can be added as needed.
}
