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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedValue {
    pub value: HashMap<String, Value>,
    pub version: u64,
    pub timestamp: u128,
}

impl VersionedValue {
    pub fn new(value: HashMap<String, Value>) -> Self {
        let ts = current_timestamp();
        Self {
            value,
            version: 1,
            timestamp: ts,
        }
    }

    pub fn update(&mut self, value: HashMap<String, Value>) {
        self.value = value;
        self.version += 1;
        self.timestamp = current_timestamp();
    }
}

pub fn current_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

/// The local in‑memory database state.
/// The `store` maps table names to the table’s key–value store.
/// Each table’s store maps a key to a VersionedValue.
pub struct AppState {
    /// Base directory under which each table (folder) resides.
    /// Every table will be stored in a directory: `{base_dir}/{table}`.
    pub base_dir: &'static str,
    pub store: RwLock<HashMap<String, HashMap<String, VersionedValue>>>,
    // The quorum thresholds still remain for consistency checking.
    pub write_quorum: usize,
    pub read_quorum: usize,
}

/// Shared cluster data, including dynamic membership and node statuses.
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

/// Utility: Extract active nodes from the cluster membership map.
fn get_active_nodes(nodes: &HashMap<String, NodeInfo>) -> Vec<String> {
    nodes
        .iter()
        .filter(|(_, info)| info.status == NodeStatus::Active)
        .map(|(k, _)| k.clone())
        .collect()
}

/// Calculates the “responsible” node for a given key using a simple hash modulo.
fn get_responsible_node(key: &str, nodes: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let index = (hash % (nodes.len() as u64)) as usize;
    nodes[index].clone()
}

/// Computes replication targets for a given key.
/// In this implementation we use all active nodes.
/// (For a more elaborate scheme you might rotate the order.)
fn get_replication_nodes(key: &str, nodes: &[String], replication_factor: usize) -> Vec<String> {
    // Here we use the primary index calculation to determine an ordering.
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let total_nodes = nodes.len();
    let primary_index = (hash % (total_nodes as u64)) as usize;

    let mut targets = Vec::with_capacity(replication_factor);
    targets.push(nodes[primary_index].clone());
    let mut i = 1;
    while targets.len() < replication_factor && i < total_nodes {
        let idx = (primary_index + i) % total_nodes;
        targets.push(nodes[idx].clone());
        i += 1;
    }
    targets
}

/// Handler to return the entire in‑memory store for a table as JSON.
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

/// Handler for a node joining the cluster.
#[derive(Deserialize)]
pub struct JoinRequest {
    node: String,
    secret: String,
}

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
    // Retrieve the valid API key from the environment variable.
    // In production, ensure this secret is securely managed.
    let valid_api_key =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    // Check for the API key in the "x-api-key" header.
    match req.headers().get("x-api-key") {
        Some(header_value) => {
            // Convert the header value to a string.
            if let Ok(api_key) = header_value.to_str() {
                if api_key != valid_api_key {
                    return HttpResponse::Unauthorized()
                        .body("Invalid API key provided");
                }
            } else {
                return HttpResponse::BadRequest().body("Malformed API key header");
            }
        }
        None => {
            return HttpResponse::Unauthorized().body("Missing API key");
        }
    };

    // If the API key is valid, retrieve the store.
    let store = state.store.read().await;
    HttpResponse::Ok().json(store.clone())
}


/// GET handler for quorum reading a key's value.
/// This function requests the key from all active nodes
/// and then selects the highest-versioned result.
pub async fn get_value(
    path: web::Path<(String, String)>, // (table, key)
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // Retrieve the active nodes.
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    // Use all active nodes as the replication factor.
    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    let client = reqwest::Client::new();
    let mut futures_vec: Vec<BoxFuture<'static, Result<Option<VersionedValue>, String>>> =
        Vec::new();

    // Iterate over targets by value.
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
                        .timeout(std::time::Duration::from_secs(3))
                        .send()
                        .await
                    {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                match resp.json::<VersionedValue>().await {
                                    Ok(v) => Ok(Some(v)),
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
            println!("Error during quorum read: {}", err);
        }
    }

    if valid_results.len() < state.read_quorum {
        return HttpResponse::InternalServerError()
            .json(json!({"error": "Read quorum not achieved"}));
    }

    // Pick the value with the highest version.
    let latest = valid_results
        .into_iter()
        .max_by(|a, b| a.version.cmp(&b.version))
        .unwrap();
    HttpResponse::Ok().json(latest)
}

/// PUT handler: Inserts or updates a key–value pair and replicates to all active nodes.
pub async fn put_value(
    path: web::Path<(String, String)>, // (table, key)
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    body: Json<HashMap<String, Value>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // Perform a local update.
    let new_value = {
        let mut store = state.store.write().await;
        let table = store.entry(table_name.clone()).or_insert_with(HashMap::new);
        if let Some(existing) = table.get_mut(&key_val) {
            existing.update(body.0.clone());
            existing.clone()
        } else {
            let v = VersionedValue::new(body.0.clone());
            table.insert(key_val.clone(), v.clone());
            v
        }
    };

    // Retrieve all active nodes.
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    // Use all active nodes as the replication factor.
    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    let client = reqwest::Client::new();
    let payload = serde_json::to_value(&new_value).unwrap();
    let mut ack_futures: Vec<BoxFuture<'static, Result<(), String>>> = Vec::new();

    // Iterate by value.
    for target in targets.clone().into_iter() {
        if target == *current_addr.get_ref() {
            ack_futures.push(Box::pin(async { Ok(()) }));
        } else {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_inner = client.clone();
            // Clone the payload for each iteration.
            let payload = payload.clone();
            let fut: BoxFuture<'static, Result<(), String>> = Box::pin(async move {
                match client_inner
                    .put(&url)
                    .json(&payload)
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            Ok(())
                        } else {
                            Err(format!("Failed at {}: {}", target, resp.status()))
                        }
                    }
                    Err(e) => Err(format!("Request error to {}: {}", target, e)),
                }
            });
            ack_futures.push(fut);
        }
    }

    let results = futures_util::future::join_all(ack_futures).await;
    let success_count = results.into_iter().filter(|res| res.is_ok()).count();
    let total_targets = targets.len();
    // Require that every target acknowledges.
    if success_count != total_targets {
        return HttpResponse::InternalServerError().json(json!({
            "error": format!("Write quorum not achieved, only got {} out of {} acks", success_count, total_targets)
        }));
    }
    HttpResponse::Created().json(new_value)
}

/// DELETE handler: Removes a key and replicates the deletion to all active nodes.
pub async fn delete_value(
    path: web::Path<(String, String)>, // (table, key)
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

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

    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);
    let client = reqwest::Client::new();
    let mut ack_futures: Vec<BoxFuture<'static, Result<(), String>>> = Vec::new();

    for target in targets.clone().into_iter() {
        if target == *current_addr.get_ref() {
            ack_futures.push(Box::pin(async { Ok(()) }));
        } else {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_inner = client.clone();
            let fut: BoxFuture<'static, Result<(), String>> = Box::pin(async move {
                match client_inner
                    .delete(&url)
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            Ok(())
                        } else {
                            Err(format!("Failed delete at {}: {}", target, resp.status()))
                        }
                    }
                    Err(e) => Err(format!("Request error to {}: {}", target, e)),
                }
            });
            ack_futures.push(fut);
        }
    }

    let results = futures_util::future::join_all(ack_futures).await;
    let success_count = results.into_iter().filter(|res| res.is_ok()).count();
    let total_targets = targets.len();
    if success_count != total_targets {
        return HttpResponse::InternalServerError().json(json!({
            "error": format!("Delete quorum not achieved, only got {} out of {} acks", success_count, total_targets)
        }));
    }
    HttpResponse::Ok().json(json!({ "message": local_status }))
}

/// GET /{table}/keys
/// Returns a JSON array with all key names in the given table.
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

/// Input for retrieving multiple keys.
#[derive(Deserialize)]
pub struct KeysRequest {
    /// List of keys requested.
    pub keys: Vec<String>,
}

/// POST /{table}/keys
/// Returns a JSON object mapping each found key to its stored VersionedValue.
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
