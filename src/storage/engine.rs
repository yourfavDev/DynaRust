use actix_web::{web, HttpResponse, Responder};
use std::collections::HashMap;
use std::sync::Mutex;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

/// The local in-memory database state.
pub struct AppState {
    pub(crate) store: Mutex<HashMap<String, String>>,
}

/// Shared cluster data (a list of node addresses).
#[derive(Clone)]
pub struct ClusterData {
    pub nodes: Vec<String>,
}

/// Calculate the "responsible" node for a given key using a simple hash modulo algorithm.
fn get_responsible_node(key: &str, nodes: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    // Choose a node based on the modulo of the hash.
    let index = (hash % (nodes.len() as u64)) as usize;
    nodes[index].clone()
}

/// GET handler: Fetches the value for a key.
/// The request is forwarded to the responsible node if needed.
pub async fn get_value(
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    key: web::Path<String>,
) -> impl Responder {
    let key_val = key.into_inner();
    let responsible = get_responsible_node(&key_val, &cluster_data.nodes);

    if responsible == *current_addr.get_ref() {
        // Process locally.
        let store = data.store.lock().unwrap();
        if let Some(value) = store.get(&key_val) {
            HttpResponse::Ok().body(value.clone())
        } else {
            HttpResponse::NotFound().body("Key not found")
        }
    } else {
        // Forward request to the responsible node.
        let url = format!("http://{}/key/{}", responsible, key_val);
        let client = reqwest::Client::new();
        match client.get(url).send().await {
            Ok(resp) => {
                let reqwest_status = resp.status();
                let text = resp.text().await.unwrap_or_else(|_| {
                    "Error reading response from responsible node".to_string()
                });
                // Convert reqwest status into actix_web HTTP status.
                let actix_status = actix_web::http::StatusCode::from_u16(reqwest_status.as_u16())
                    .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
                HttpResponse::build(actix_status).body(text)
            }
            Err(e) => HttpResponse::InternalServerError()
                .body(format!("Error forwarding GET request: {}", e)),
        }
    }
}

/// PUT handler: Inserts or updates a key–value pair.
/// Forwards the request if this node is not responsible.
pub async fn put_value(
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    key: web::Path<String>,
    body: String,
) -> impl Responder {
    let key_val = key.into_inner();
    let responsible = get_responsible_node(&key_val, &cluster_data.nodes);

    if responsible == *current_addr.get_ref() {
        // Process locally.
        let mut store = data.store.lock().unwrap();
        store.insert(key_val, body);
        HttpResponse::Created().body("Key stored")
    } else {
        // Forward request to the responsible node.
        let url = format!("http://{}/key/{}", responsible, key_val);
        let client = reqwest::Client::new();
        match client.put(&url).body(body).send().await {
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
                .body(format!("Error forwarding PUT request: {}", e)),
        }
    }
}

/// DELETE handler: Deletes a key.
/// Forwards the request if this node is not responsible.
pub async fn delete_value(
    data: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    key: web::Path<String>,
) -> impl Responder {
    let key_val = key.into_inner();
    let responsible = get_responsible_node(&key_val, &cluster_data.nodes);

    if responsible == *current_addr.get_ref() {
        // Process locally.
        let mut store = data.store.lock().unwrap();
        if store.remove(&key_val).is_some() {
            HttpResponse::Ok().body("Key deleted")
        } else {
            HttpResponse::NotFound().body("Key not found")
        }
    } else {
        // Forward request to the responsible node.
        let url = format!("http://{}/key/{}", responsible, key_val);
        let client = reqwest::Client::new();
        match client.delete(&url).send().await {
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
                .body(format!("Error forwarding DELETE request: {}", e)),
        }
    }
}
