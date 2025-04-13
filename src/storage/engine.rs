use actix_web::{web, HttpResponse, Responder};
use futures_util::future::join_all;
use reqwest;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The local in-memory database state.
/// The `store` maps table names to the table's key–value store.
/// Each table's store maps a key to a JSON object (a key–value map of attributes).
pub struct AppState {
    /// Base directory under which each table (folder) resides.
    /// Every table will be stored in a directory: `{base_dir}/{table}`.
    pub base_dir: &'static str,
    pub store: Mutex<HashMap<String, HashMap<String, HashMap<String, Value>>>>,
}

/// NEW: Handler to return the entire in-memory store for a table as JSON.
/// This endpoint is used by newly joined nodes to fetch the cold (persisted)
/// state from a peer.
///
/// The route is assumed to look like `GET /{table}/store`.
pub async fn get_table_store(
    table: web::Path<String>,
    state: web::Data<AppState>,
) -> impl Responder {
    let table_name = table.into_inner();
    let store = state.store.lock().unwrap();
    if let Some(table_db) = store.get(&table_name) {
        HttpResponse::Ok().json(table_db)
    } else {
        HttpResponse::NotFound().body("Table not found")
    }
}

/// Shared cluster data, including dynamic membership and the list of alive nodes.
#[derive(Clone)]
pub struct ClusterData {
    /// Dynamically maintained cluster membership list.
    pub nodes: Arc<Mutex<Vec<String>>>,
}

/// Calculates the "responsible" node for a given key using a simple hash modulo algorithm.
fn get_responsible_node(key: &str, nodes: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let index = (hash % (nodes.len() as u64)) as usize;
    nodes[index].clone()
}

/// Computes replication targets for a given key. The primary is chosen via a hash modulo,
/// and backup nodes are chosen in a circular fashion.
fn get_replication_nodes(
    key: &str,
    nodes: &[String],
    replication_factor: usize,
) -> Vec<String> {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let total_nodes = nodes.len();
    let primary_index = (hash % (total_nodes as u64)) as usize;

    let mut targets = Vec::with_capacity(replication_factor);
    targets.push(nodes[primary_index].clone());

    // Choose additional targets in a circular fashion.
    let mut i = 1;
    while targets.len() < replication_factor && i < total_nodes {
        let idx = (primary_index + i) % total_nodes;
        targets.push(nodes[idx].clone());
        i += 1;
    }
    targets
}

/// Handler for a node joining the cluster.
/// A POST to `/join` with a JSON body `{ "node": "address" }` adds the node
/// to the membership list (if not already present) and returns the updated list.
#[derive(Deserialize)]
pub struct JoinRequest {
    node: String,
}

pub async fn join_cluster(
    cluster: web::Data<ClusterData>,
    request: web::Json<JoinRequest>,
) -> impl Responder {
    let new_node = request.node.clone();
    let mut nodes_guard = cluster.nodes.lock().unwrap();
    if !nodes_guard.contains(&new_node) {
        nodes_guard.push(new_node);
    }
    HttpResponse::Ok().json(nodes_guard.clone())
}

/// GET handler for fetching a key's value from a table.
/// If the current node is responsible, it serves from local storage;
/// otherwise, it forwards the request to the responsible node.
///
/// The route is assumed to look like: `GET /{table}/key/{key}`
pub async fn get_value(
    path: web::Path<(String, String)>, // (table, key)
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();
    let nodes = {
        let guard = cluster_data.nodes.lock().unwrap();
        guard.clone()
    };
    let responsible = get_responsible_node(&key_val, &nodes);

    if responsible == *current_addr.get_ref() {
        let store = data.store.lock().unwrap();
        if let Some(table_db) = store.get(&table_name) {
            if let Some(value) = table_db.get(&key_val) {
                HttpResponse::Ok().json(value)
            } else {
                HttpResponse::NotFound().body("Key not found")
            }
        } else {
            HttpResponse::NotFound().body("Table not found")
        }
    } else {
        let url = format!("http://{}/{}/key/{}", responsible, table_name, key_val);
        let client = reqwest::Client::new();
        match client.get(url).send().await {
            Ok(resp) => {
                let reqwest_status = resp.status();
                let text = resp
                    .text()
                    .await
                    .unwrap_or_else(|_| "Error reading response".to_string());
                let actix_status = actix_web::http::StatusCode::from_u16(
                    reqwest_status.as_u16(),
                )
                    .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
                HttpResponse::build(actix_status).body(text)
            }
            Err(e) => HttpResponse::InternalServerError()
                .body(format!("Error forwarding GET request: {}", e)),
        }
    }
}

/// PUT handler: Inserts or updates a key–value pair with dynamic replication.
///
/// The endpoint expects a JSON object (a map of attributes) in the request body.
/// The route is assumed to look like: `PUT /{table}/key/{key}`
pub async fn put_value(
    path: web::Path<(String, String)>, // (table, key)
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    body: web::Json<HashMap<String, Value>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();
    let nodes = {
        let guard = cluster_data.nodes.lock().unwrap();
        guard.clone()
    };
    let replication_factor = nodes.len();
    let targets = get_replication_nodes(&key_val, &nodes, replication_factor);

    let client = reqwest::Client::new();
    let mut tasks = Vec::new();

    // Forward to all replication targets concurrently.
    for target in targets {
        let key_clone = key_val.clone();
        let table_clone = table_name.clone();
        let body_clone = body.0.clone();
        let data_clone = data.clone();

        if target == *current_addr.get_ref() {
            let local_task = async move {
                {
                    // Ensure the table exists (or create it).
                    let mut store = data_clone.store.lock().unwrap();
                    let table_db =
                        store.entry(table_clone.clone()).or_insert_with(HashMap::new);
                    table_db.insert(key_clone, body_clone);
                }
                // Create the folder for the table if it doesn't exist.
                let table_folder = Path::new(data_clone.base_dir).join(&table_clone);
                if let Err(e) = fs::create_dir_all(&table_folder) {
                    eprintln!(
                        "Warning: could not create directory {:?}: {}",
                        table_folder, e
                    );
                }
                Ok::<String, reqwest::Error>(format!("Local write on {}", target))
            };
            tasks.push(tokio::spawn(local_task));
        } else {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_clone = client.clone();
            let forward_task = async move {
                let _resp = client_clone.put(&url).json(&body_clone).send().await?;
                Ok(format!("Forwarded to {}", target))
            };
            tasks.push(tokio::spawn(forward_task));
        }
    }

    let _results = join_all(tasks).await;
    HttpResponse::Created().body("Key stored with dynamic replication")
}

/// DELETE handler: Broadcasts deletion to all replicas that hold the key.
/// The route is assumed to look like: `DELETE /{table}/key/{key}`
pub async fn delete_value(
    path: web::Path<(String, String)>, // (table, key)
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();
    let nodes = {
        let guard = cluster_data.nodes.lock().unwrap();
        guard.clone()
    };
    let replication_factor = nodes.len();
    let targets = get_replication_nodes(&key_val, &nodes, replication_factor);
    let client = reqwest::Client::new();
    let mut tasks =
        Vec::<tokio::task::JoinHandle<Result<String, reqwest::Error>>>::new();

    for target in targets {
        let key_clone = key_val.clone();
        let table_clone = table_name.clone();

        if target == *current_addr.get_ref() {
            let data_clone = data.clone();
            let local_task = async move {
                let mut store = data_clone.store.lock().unwrap();
                if let Some(table_db) = store.get_mut(&table_clone) {
                    let result = match table_db.remove(&key_clone) {
                        Some(_) => format!("Local delete on {}", target),
                        None => format!("Key not found on {}", target),
                    };
                    Ok(result)
                } else {
                    Ok(format!("Table not found on {}", target))
                }
            };
            tasks.push(tokio::spawn(local_task));
        } else {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_clone = client.clone();
            let forward_task = async move {
                let _resp = client_clone.delete(&url).send().await?;
                Ok(format!("Forwarded delete to {}", target))
            };
            tasks.push(tokio::spawn(forward_task));
        }
    }

    let _results = join_all(tasks).await;
    HttpResponse::Ok().body("Key deleted from all replicas")
}

/// GET /{table}/keys
/// Returns a JSON array with all key names in the given table.
pub async fn get_all_keys(table: web::Path<String>, state: web::Data<AppState>) -> impl Responder {
    let table_name = table.into_inner();
    let store = state.store.lock().unwrap();
    if let Some(table_data) = store.get(&table_name) {
        let keys: Vec<String> = table_data.keys().cloned().collect();
        HttpResponse::Ok().json(keys)
    } else {
        HttpResponse::NotFound().body("Table not found")
    }
}

/// Input for retrieving multiple keys.
#[derive(Deserialize)]
pub struct KeysRequest {
    /// List of keys requested.
    pub keys: Vec<String>,
}

/// POST /{table}/keys
/// With a JSON body { "keys": ["key1", "key2", ...] },
/// returns a JSON object mapping each found key to its stored value.
pub async fn get_multiple_keys(
    table: web::Path<String>,
    keys_request: web::Json<KeysRequest>,
    state: web::Data<AppState>,
) -> impl Responder {
    let table_name = table.into_inner();
    let requested_keys = keys_request.into_inner().keys;
    let store = state.store.lock().unwrap();
    if let Some(table_data) = store.get(&table_name) {
        let mut result = HashMap::new();
        for key in requested_keys {
            if let Some(value) = table_data.get(&key) {
                result.insert(key, value.clone());
            }
        }
        HttpResponse::Ok().json(result)
    } else {
        HttpResponse::NotFound().body("Table not found")
    }
}
