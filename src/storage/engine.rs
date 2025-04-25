use tokio::sync::Semaphore;
use crate::Arc;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use std::{env, process};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use web::Json;
use crate::storage::subscription::{KeyEvent, SubscriptionManager};
use jsonwebtoken::{decode, DecodingKey, Validation};
use tokio::time;
use crate::network::broadcaster::{gossip_membership};
//
// CONSTANTS & TYPES
//

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

pub fn get_jwt_secret() -> String {
    env::var("JWT_SECRET").unwrap_or_else(|_| {
        eprintln!("error: JWT_SECRET environment variable not set");
        process::exit(1);
    })
}

/// Get current timestamp in milliseconds.
pub fn current_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

/// Extract active nodes from the cluster membership map.
pub fn get_active_nodes(nodes: &HashMap<String, NodeInfo>) -> Vec<String> {
    nodes
        .iter()
        .filter(|(_, info)| info.status == NodeStatus::Active)
        .map(|(k, _)| k.clone())
        .collect()
}

/// Computes replication targets for a given key.
pub fn get_replication_nodes(key: &str, nodes: &[String], replication_factor: usize) -> Vec<String> {
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
                    &DecodingKey::from_secret(get_jwt_secret().as_ref()),
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

// external PUT handler
#[allow(clippy::too_many_arguments)]
pub async fn put_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
    client: web::Data<reqwest::Client>,
    sem: web::Data<Arc<Semaphore>>,
    body: web::Json<HashMap<String, Value>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // 1️⃣ Authenticate external user via JWT
    let user = match extract_user_from_token(&req) {
        Ok(u) => u,
        Err(resp) => return resp,
    };

    // 2️⃣ Apply local update with ownership check
    let new_value = {
        let mut db = state.store.write().await;
        let table = db.entry(table_name.clone()).or_default();

        if let Some(existing) = table.get_mut(&key_val) {
            if existing.owner != user {
                return HttpResponse::Unauthorized().json(serde_json::json!({
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

    // 3️⃣ Notify any SSE subscriptions
    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Updated(new_value.clone()))
        .await;

    // 4️⃣ Replicate *full* VersionedValue to other nodes, but cap concurrency
    let cluster = cluster_data.nodes.read().await;
    let active = get_active_nodes(&*cluster);
    drop(cluster);

    let targets = get_replication_nodes(&key_val, &active, active.len());
    let secret = env::var("CLUSTER_SECRET").unwrap_or_default();

    for target in targets {
        if target == *current_addr.get_ref() {
            continue;
        }
        // acquire a permit (will await if > 20 in flight)
        let permit = <Arc<Semaphore> as Clone>::clone(&sem).acquire_owned().await.unwrap();
        let cli = client.clone();
        let payload = new_value.clone();
        let url = format!(
            "http://{}/internal/{}/key/{}",
            target, table_name, key_val
        );

        let secret_clone = secret.clone();

        tokio::spawn(async move {
            let _permit = permit; // held until this future ends
            if let Err(e) = cli
                .put(&url)
                .header("X-Internal-Request", "true")
                .header("SECRET", &secret_clone)
                .json(&payload)
                .send()
                .await
            {
                eprintln!("replication to {} failed: {}", target, e);
            }
        });
    }

    HttpResponse::Created().json(new_value)
}

// internal PUT handler (must be mounted at: /internal/{table}/key/{key})
pub async fn put_value_internal(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    sub_manager: web::Data<SubscriptionManager>,
    body: web::Json<VersionedValue>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // 1️⃣ Cluster‐secret check
    let cluster_secret = env::var("CLUSTER_SECRET").unwrap_or_default();
    match req.headers().get("SECRET") {
        Some(h) if h.to_str().unwrap_or("") == cluster_secret => {}
        _ => return HttpResponse::Unauthorized().finish(),
    }

    // 2️⃣ Merge the incoming VersionedValue by version number
    let incoming = body.into_inner();
    let mut db = state.store.write().await;
    let table = db.entry(table_name.clone()).or_default();

    let cached = table
        .entry(key_val.clone())
        .and_modify(|existing| {
            if incoming.version > existing.version {
                *existing = incoming.clone();
            }
        })
        .or_insert_with(|| incoming.clone())
        .clone();

    // 3️⃣ Notify subscribers
    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Updated(cached.clone()))
        .await;

    HttpResponse::Created().json(cached)
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
    let cluster2 = cluster.clone();
    let mut nodes_guard = cluster.nodes.write().await;
    if !nodes_guard.contains_key(&new_node) {
        nodes_guard.insert(
            new_node.clone(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );

        tokio::spawn(async move {
                time::sleep(std::time::Duration::from_secs(5)).await;
                let args: Vec<String> = env::args().collect();
                let current_node = args[1].clone();
                let client = reqwest::Client::new();
                gossip_membership(&cluster2, &client, &*current_node).await;
        });
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