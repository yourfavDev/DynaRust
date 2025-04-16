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
//
// CORE DATA STRUCTURES
//

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

/// The local in-memory database state.
pub struct AppState {
    /// Base directory under which each table (folder) resides.
    pub base_dir: &'static str,
    /// Maps table names to the table's key-value store.
    pub store: RwLock<HashMap<String, HashMap<String, VersionedValue>>>,
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

/// Input for retrieving multiple keys.
#[derive(Deserialize)]
pub struct KeysRequest {
    /// List of keys requested.
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

/// Get current timestamp in milliseconds
pub fn current_timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis()
}

/// Extract active nodes from the cluster membership map
fn get_active_nodes(nodes: &HashMap<String, NodeInfo>) -> Vec<String> {
    nodes
        .iter()
        .filter(|(_, info)| info.status == NodeStatus::Active)
        .map(|(k, _)| k.clone())
        .collect()
}

/// Computes replication targets for a given key
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

//
// DATA OPERATION HANDLERS
//

/// GET handler for reading a key's value with replication
pub async fn get_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
) -> impl Responder {
    // Handle internal requests with local lookup only
    if req.headers().contains_key("X-Internal-Request") {
        let (table_name, key_val) = path.into_inner();
        let store = state.store.read().await;
        let result = store
            .get(&table_name)
            .and_then(|table| table.get(&key_val))
            .cloned();
        return HttpResponse::Ok().json(result);
    }

    // For external requests, fetch from all replicas
    let (table_name, key_val) = path.into_inner();

    // Get active nodes for replication
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    // Determine replication targets
    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    // Prepare request futures for all replicas
    let client = reqwest::Client::new();
    let mut futures_vec: Vec<BoxFuture<'static, Result<Option<VersionedValue>, String>>> = Vec::new();

    for target in targets.into_iter() {
        if target == *current_addr.get_ref() {
            // Local node case
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
            // Remote node case
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

    // Collect results
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
        return HttpResponse::NotFound().json(json!({"error": "Key not found on any replica"}));
    }

    // Return the highest version value
    let latest = valid_results
        .into_iter()
        .max_by(|a, b| a.version.cmp(&b.version))
        .unwrap();
    HttpResponse::Ok().json(latest)
}

/// PUT handler: Inserts or updates a key–value pair and replicates to active nodes.
/// Also sends a live update notification via the SubscriptionManager.
pub async fn put_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    // New dependency injection for the subscription manager.
    sub_manager: web::Data<SubscriptionManager>,
    body: web::Json<HashMap<String, Value>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // Handle internal replication request (skip notifications).
    if req.headers().contains_key("X-Internal-Request") {
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
        sub_manager.notify(&table_name, &key_val, KeyEvent::Updated(new_value.clone())).await;

        return HttpResponse::Created().json(new_value);
    }


    // External request: update local store.
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

    // Notify all subscribers that the key has been updated.
    sub_manager.notify(&table_name, &key_val, KeyEvent::Updated(new_value.clone())).await;


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

    // Fire replication requests asynchronously.
    tokio::spawn(async move {
        futures_util::future::join_all(replication_futures).await;
    });

    HttpResponse::Created().json(new_value)
}

/// DELETE handler: Removes a key and replicates the deletion to active nodes.
/// Also notifies subscribers of the deletion event.
pub async fn delete_value(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();

    // Internal replication request.
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

    // External request: remove the key locally.
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

    // Notify subscribers that the key has been deleted.
    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Deleted)
        .await;

    // Replicate deletion to other nodes.
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

/// Handler to return the entire in-memory store for a table as JSON
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

/// Returns a JSON array with all key names in the given table
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

/// Returns a JSON object mapping each found key to its stored VersionedValue
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

/// Handler for a node joining the cluster
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

/// Handler to access global store with authentication
pub async fn get_global_store(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> impl Responder {
    // Retrieve the valid API key from the environment variable
    let valid_api_key = env::var("CLUSTER_SECRET")
        .unwrap_or_else(|_| "default_secret".to_string());

    // Check for the API key in the "x-api-key" header
    match req.headers().get("x-api-key") {
        Some(header_value) => {
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

    // If the API key is valid, retrieve the store
    let store = state.store.read().await;
    HttpResponse::Ok().json(store.clone())
}
