use actix_web::{web, HttpResponse, Responder};
use futures_util::future::join_all;
use std::collections::HashMap;
use std::sync::Mutex;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

/// The local in-memory database state.
pub struct AppState {
    pub(crate) store: Mutex<HashMap<String, String>>,
}

/// Shared cluster data (a list of node addresses and the desired replication factor).
#[derive(Clone)]
pub struct ClusterData {
    pub nodes: Vec<String>,
    pub replication_factor: usize,
}

/// Calculate the "responsible" node for a given key using a simple hash modulo algorithm.
fn get_responsible_node(key: &str, nodes: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let index = (hash % (nodes.len() as u64)) as usize;
    nodes[index].clone()
}

/// Helper that computes replication targets for a given key.
fn get_replication_nodes(key: &str, nodes: &[String], replication_factor: usize) -> Vec<String> {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    let total_nodes = nodes.len();
    let primary_index = (hash % (total_nodes as u64)) as usize;

    let mut targets = Vec::with_capacity(replication_factor);
    targets.push(nodes[primary_index].clone());

    // Choose additional targets (backups) in a circular fashion.
    let mut i = 1;
    while targets.len() < replication_factor && i < total_nodes {
        let idx = (primary_index + i) % total_nodes;
        targets.push(nodes[idx].clone());
        i += 1;
    }
    targets
}

/// GET handler: unchanged (for brevity)
pub async fn get_value(
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    key: web::Path<String>,
) -> impl Responder {
    let key_val = key.into_inner();
    let responsible = get_responsible_node(&key_val, &cluster_data.nodes);

    if responsible == *current_addr.get_ref() {
        let store = data.store.lock().unwrap();
        if let Some(value) = store.get(&key_val) {
            HttpResponse::Ok().body(value.clone())
        } else {
            HttpResponse::NotFound().body("Key not found")
        }
    } else {
        let url = format!("http://{}/key/{}", responsible, key_val);
        let client = reqwest::Client::new();
        match client.get(url).send().await {
            Ok(resp) => {
                let reqwest_status = resp.status();
                let text = resp.text().await.unwrap_or_else(|_| {
                    "Error reading response from responsible node".to_string()
                });
                let actix_status = actix_web::http::StatusCode::from_u16(reqwest_status.as_u16())
                    .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
                HttpResponse::build(actix_status).body(text)
            }
            Err(e) => HttpResponse::InternalServerError()
                .body(format!("Error forwarding GET request: {}", e)),
        }
    }
}

/// PUT handler: Inserts or updates a key–value pair with dynamic replication.
/// It calculates the target nodes using get_replication_nodes() and then
/// concurrently sends the write to all target nodes.
pub async fn put_value(
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    key: web::Path<String>,
    body: String,
) -> impl Responder {
    let key_val = key.into_inner();
    let replication_factor = cluster_data.replication_factor;
    let targets = get_replication_nodes(&key_val, &cluster_data.nodes, replication_factor);

    let client = reqwest::Client::new();
    let mut tasks = Vec::new();

    for target in targets {
        // Clone the key and body for use in each iteration.
        let key_clone = key_val.clone();
        let body_clone = body.clone();
        // Clone `data` so that each task gets its own handle.
        let data_clone = data.clone();

        if target == *current_addr.get_ref() {
            let local_task = async move {
                let mut store = data_clone.store.lock().unwrap();
                store.insert(key_clone, body_clone);
                Ok::<String, reqwest::Error>(format!("Local write on {}", target))
            };
            tasks.push(tokio::spawn(local_task));
        } else {
            let url = format!("http://{}/key/{}", target, key_val);
            let client_clone = client.clone();
            let forward_task = async move {
                let _resp = client_clone.put(&url).body(body_clone).send().await?;
                // For simplicity, just return the target name after forwarding.
                Ok(format!("Forwarded to {}", target))
            };
            tasks.push(tokio::spawn(forward_task));
        }
    }

    // Execute all spawned tasks concurrently and wait for them to complete.
    let _results = futures_util::future::join_all(tasks).await;

    HttpResponse::Created().body("Key stored with dynamic replication")
}

/// DELETE handler: Broadcasts deletion to all replicas holding the key.
/// The delete operation is sent to all nodes in the replication group.
pub async fn delete_value(
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    key: web::Path<String>,
) -> impl Responder {
    let key_val = key.into_inner();
    // Compute the replication group for the key.
    let replication_factor = cluster_data.replication_factor;
    let targets = get_replication_nodes(&key_val, &cluster_data.nodes, replication_factor);

    let client = reqwest::Client::new();
    // Explicitly type the tasks vector to satisfy the compiler.
    let mut tasks: Vec<tokio::task::JoinHandle<Result<String, reqwest::Error>>> = Vec::new();

    for target in targets {
        let key_clone = key_val.clone();
        let data_clone = data.clone();

        if target == *current_addr.get_ref() {
            let local_task = async move {
                let mut store = data_clone.store.lock().unwrap();
                let result = store.remove(&key_clone);
                match result {
                    Some(_) => Ok(format!("Local delete on {}", target)),
                    None => Ok(format!("Key not found on {}", target)),
                }
            };
            tasks.push(tokio::spawn(local_task));
        } else {
            let url = format!("http://{}/key/{}", target, key_val);
            let client_clone = client.clone();
            let forward_task = async move {
                let _resp = client_clone.delete(&url).send().await?;
                Ok(format!("Forwarded delete to {}", target))
            };
            tasks.push(tokio::spawn(forward_task));
        }
    }

    // Execute all spawned deletion tasks concurrently.
    let _results = join_all(tasks).await;
    HttpResponse::Ok().body("Key deleted from all replicas")
}