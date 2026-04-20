use crate::Arc;
use crate::network::broadcaster::gossip_membership;
use crate::storage::subscription::{KeyEvent, SubscriptionManager};
use actix_web::{HttpRequest, HttpResponse, Responder, web};
use futures::future::BoxFuture;
use jsonwebtoken::{DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, process};
use tokio::sync::RwLock;
use tokio::sync::Semaphore;
use tokio::time;
use web::Json;
//
// CONSTANTS & TYPES
//

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

use dashmap::DashMap;

//
// CORE DATA STRUCTURES
//

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedValue {
    pub value: HashMap<String, Value>,
    pub vector_clock: HashMap<String, u64>,
    pub timestamp: u128,
    pub owner: String,
}

impl VersionedValue {
    pub fn new(value: HashMap<String, Value>, owner: String, node_id: String) -> Self {
        let ts = current_timestamp();
        let mut vector_clock = HashMap::new();
        vector_clock.insert(node_id, 1);
        Self {
            value,
            vector_clock,
            timestamp: ts,
            owner,
        }
    }

    pub fn update(&mut self, value: HashMap<String, Value>, node_id: String) {
        self.value = value;
        let counter = self.vector_clock.entry(node_id).or_insert(0);
        *counter += 1;
        self.timestamp = current_timestamp();
    }

    pub fn patch(&mut self, updates: HashMap<String, Value>, node_id: String) {
        for (k, v) in updates {
            self.value.insert(k, v);
        }
        let counter = self.vector_clock.entry(node_id).or_insert(0);
        *counter += 1;
        self.timestamp = current_timestamp();
    }

    pub fn dominates(&self, other: &Self) -> bool {
        let mut strictly_newer = false;
        for (node, other_v) in &other.vector_clock {
            let self_v = self.vector_clock.get(node).copied().unwrap_or(0);
            if self_v < *other_v {
                return false;
            } else if self_v > *other_v {
                strictly_newer = true;
            }
        }
        for (node, self_v) in &self.vector_clock {
            if !other.vector_clock.contains_key(node) && *self_v > 0 {
                strictly_newer = true;
            }
        }
        strictly_newer
    }
}

/// The local in-memory database state.
pub struct AppState {
    pub base_dir: &'static str,
    /// Maps table names to the table's key-value store.
    pub store: DashMap<String, DashMap<String, VersionedValue>>,
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

#[derive(Deserialize, Serialize)]
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
pub fn get_replication_nodes(
    key: &str,
    nodes: &[String],
    replication_factor: usize,
) -> Vec<String> {
    if nodes.is_empty() {
        return Vec::new();
    }

    let replication_factor = replication_factor.min(nodes.len());
    let mut ring = std::collections::BTreeMap::new();
    let virtual_nodes = 10;

    for node in nodes {
        for v in 0..virtual_nodes {
            let mut hasher = DefaultHasher::new();
            format!("{}#{}", node, v).hash(&mut hasher);
            ring.insert(hasher.finish(), node.clone());
        }
    }

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let key_hash = hasher.finish();

    let mut targets = Vec::with_capacity(replication_factor);

    // Chain iterators to wrap around the ring
    let iter1 = ring.range(key_hash..).map(|(_, v)| v);
    let iter2 = ring.range(..key_hash).map(|(_, v)| v);

    for node in iter1.chain(iter2) {
        if !targets.contains(node) {
            targets.push(node.clone());
            if targets.len() == replication_factor {
                break;
            }
        }
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
    client: web::Data<reqwest::Client>,
    sem: web::Data<Arc<Semaphore>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();
    let cluster_secret =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    // 1️⃣ Internal Replication Request - Verify Secret
    if let Some(internal_req) = req.headers().get("X-Internal-Request") {
        if internal_req.to_str().unwrap_or("") == cluster_secret {
            let result = state
                .store
                .get(&table_name)
                .and_then(|table| table.get(&key_val).map(|v| v.clone()));
            return HttpResponse::Ok().json(result);
        }
    }

    // 2️⃣ Authenticate external user via JWT
    let user = match extract_user_from_token(&req) {
        Ok(u) => u,
        Err(_) => return HttpResponse::Unauthorized().body("You are not owning this record"),
    };

    // 3️⃣ Request replicas (including local node) to find the latest version
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    let mut futures_vec: Vec<BoxFuture<'static, Result<(String, Option<VersionedValue>), String>>> =
        Vec::new();

    for target in targets.into_iter() {
        let target_clone = target.clone();
        if target == *current_addr.get_ref() {
            let state_clone = state.clone();
            let table_name_clone = table_name.clone();
            let key_clone = key_val.clone();

            let fut: BoxFuture<'static, Result<(String, Option<VersionedValue>), String>> =
                Box::pin(async move {
                    let result = state_clone
                        .store
                        .get(&table_name_clone)
                        .and_then(|table| table.get(&key_clone).map(|v| v.clone()));
                    Ok((target_clone, result))
                });
            futures_vec.push(fut);
        } else {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_inner = client.get_ref().clone();
            let secret_clone = cluster_secret.clone();

            let fut: BoxFuture<'static, Result<(String, Option<VersionedValue>), String>> =
                Box::pin(async move {
                    match client_inner
                        .get(&url)
                        // FIX: Send the actual secret instead of "true"
                        .header("X-Internal-Request", &secret_clone)
                        .timeout(std::time::Duration::from_secs(3))
                        .send()
                        .await
                    {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                match resp.json::<Option<VersionedValue>>().await {
                                    Ok(v) => Ok((target_clone, v)),
                                    Err(e) => Err(format!(
                                        "Error parsing JSON from {}: {}",
                                        target_clone, e
                                    )),
                                }
                            } else {
                                Err(format!("Error from {}: {}", target_clone, resp.status()))
                            }
                        }
                        Err(e) => Err(format!("Request error to {}: {}", target_clone, e)),
                    }
                });
            futures_vec.push(fut);
        }
    }

    // Wait for all cluster responses
    let results = futures_util::future::join_all(futures_vec).await;
    let mut valid_results = Vec::new();
    let mut all_responses = Vec::new();

    for res in results {
        match res {
            Ok((addr, Some(val))) => {
                valid_results.push(val.clone());
                all_responses.push((addr, Some(val)));
            }
            Ok((addr, None)) => {
                all_responses.push((addr, None));
            }
            Err(err) => {
                println!("Error during read: {}", err);
            }
        }
    }

    // 4️⃣ Resolve the latest version
    if valid_results.is_empty() {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": "Key not found on any replica"
        }));
    }

    let latest = valid_results
        .into_iter()
        .max_by(|a, b| {
            if a.dominates(b) {
                std::cmp::Ordering::Greater
            } else if b.dominates(a) {
                std::cmp::Ordering::Less
            } else {
                a.timestamp.cmp(&b.timestamp)
            }
        })
        .unwrap();

    // 5️⃣ Read Repair: Update out-of-date or missing nodes
    for (addr, opt_v) in all_responses {
        let needs_repair = match opt_v {
            None => true,
            Some(v) => !v.dominates(&latest) && v.timestamp < latest.timestamp,
        };

        if needs_repair {
            let cli = client.get_ref().clone();
            let latest_repair = latest.clone();
            let table_repair = table_name.clone();
            let key_repair = key_val.clone();
            let secret_repair = cluster_secret.clone();
            let target = addr.clone();
            let permit = <Arc<Semaphore> as Clone>::clone(&sem)
                .acquire_owned()
                .await
                .unwrap();

            tokio::spawn(async move {
                let _permit = permit;
                let url = format!(
                    "http://{}/internal/{}/key/{}",
                    target, table_repair, key_repair
                );
                let _ = cli
                    .put(&url)
                    // FIX: Send the actual secret instead of "true"
                    .header("X-Internal-Request", &secret_repair)
                    .json(&latest_repair)
                    .send()
                    .await;
            });
        }
    }

    // 6️⃣ Enforce Ownership!
    if latest.owner != user {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Not authorized to view record"
        }));
    }

    HttpResponse::Ok().json(latest)
}

// external PATCH handler
#[allow(clippy::too_many_arguments)]
pub async fn patch_value(
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
        let table = state.store.entry(table_name.clone()).or_default();

        if let Some(mut existing) = table.get_mut(&key_val) {
            if existing.owner != user {
                return HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Not authorized to update this record"
                }));
            }
            existing.patch(body.0.clone(), current_addr.get_ref().clone());
            existing.clone()
        } else {
            return HttpResponse::NotFound().json(serde_json::json!({
                "error": "Record not found"
            }));
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
    let cluster_secret =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    for target in targets {
        if target == *current_addr.get_ref() {
            continue;
        }
        // acquire a permit (will await if > 20 in flight)
        let permit = <Arc<Semaphore> as Clone>::clone(&sem)
            .acquire_owned()
            .await
            .unwrap();
        let cli = client.clone();
        let payload = new_value.clone();
        let url = format!("http://{}/internal/{}/key/{}", target, table_name, key_val);

        let secret_clone = cluster_secret.clone();

        tokio::spawn(async move {
            let _permit = permit; // held until this future ends
            let mut retries = 3;
            let mut backoff = std::time::Duration::from_millis(100);

            while retries > 0 {
                match cli
                    .put(&url)
                    // FIX: Send the actual secret instead of "true"
                    .header("X-Internal-Request", &secret_clone)
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => break,
                    _ => {
                        retries -= 1;
                        if retries > 0 {
                            tokio::time::sleep(backoff).await;
                            backoff *= 2;
                        }
                    }
                }
            }
        });
    }

    HttpResponse::Ok().json(new_value)
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
        let table = state.store.entry(table_name.clone()).or_default();

        if let Some(mut existing) = table.get_mut(&key_val) {
            if existing.owner != user {
                return HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Not authorized to update this record"
                }));
            }
            existing.update(body.0.clone(), current_addr.get_ref().clone());
            existing.clone()
        } else {
            let v =
                VersionedValue::new(body.0.clone(), user.clone(), current_addr.get_ref().clone());
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
    let cluster_secret =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    for target in targets {
        // acquire a permit (will await if > 20 in flight)
        let permit = <Arc<Semaphore> as Clone>::clone(&sem)
            .acquire_owned()
            .await
            .unwrap();
        let cli = client.clone();
        let payload = new_value.clone();
        let url = format!("http://{}/internal/{}/key/{}", target, table_name, key_val);

        let secret_clone = cluster_secret.clone();

        tokio::spawn(async move {
            let _permit = permit; // held until this future ends
            let mut retries = 3;
            let mut backoff = std::time::Duration::from_millis(100);

            while retries > 0 {
                match cli
                    .put(&url)
                    // FIX: Send the actual secret instead of "true"
                    .header("X-Internal-Request", &secret_clone)
                    .json(&payload)
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        break;
                    }
                    // 2. Add a catch-all for requests that connect, but return a bad HTTP status code
                    Ok(resp) => {
                        println!("Replication failed with status: {}", resp.status());
                        retries -= 1;
                        if retries > 0 {
                            tokio::time::sleep(backoff).await;
                            backoff *= 2;
                        }
                    }
                    Err(e) => {
                        println!("{:?}", e);
                        retries -= 1;
                        if retries > 0 {
                            tokio::time::sleep(backoff).await;
                            backoff *= 2;
                        }
                    }
                }
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
    let cluster_secret =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());
    let provided_secret = req
        .headers()
        .get("X-Internal-Request")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if provided_secret != cluster_secret {
        return HttpResponse::Unauthorized().finish();
    }

    // 2️⃣ Merge the incoming VersionedValue by version number
    let incoming = body.into_inner();
    let table = state.store.entry(table_name.clone()).or_default();

    let cached = table
        .entry(key_val.clone())
        .and_modify(|existing| {
            if incoming.dominates(existing)
                || (!existing.dominates(&incoming) && incoming.timestamp > existing.timestamp)
            {
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
    client: web::Data<reqwest::Client>,
    sem: web::Data<Arc<Semaphore>>,
) -> impl Responder {
    let (table_name, key_val) = path.into_inner();
    let cluster_secret =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());

    // FIX: Internal requests bypass auth ONLY if the secret matches.
    if let Some(internal_req) = req.headers().get("X-Internal-Request") {
        if internal_req.to_str().unwrap_or("") == cluster_secret {
            if let Some(table) = state.store.get(&table_name) {
                table.remove(&key_val);
            }
            sub_manager
                .notify(&table_name, &key_val, KeyEvent::Deleted)
                .await;
            return HttpResponse::Ok().json(json!({"message": "Deleted"}));
        }
    }

    // External requests require a valid JWT token.
    let user = match extract_user_from_token(&req) {
        Ok(u) => u,
        Err(err_resp) => return err_resp,
    };

    // Check if the requester is the owner.
    let authorized = state
        .store
        .get(&table_name)
        .and_then(|table| table.get(&key_val).map(|v| v.owner == user))
        .unwrap_or(false);

    if !authorized {
        return HttpResponse::Unauthorized().json(json!({
            "error": "Not authorized to delete this record or record not found"
        }));
    }

    let local_status = if let Some(table) = state.store.get(&table_name) {
        if table.remove(&key_val).is_some() {
            "Deleted locally"
        } else {
            "Key not found locally"
        }
    } else {
        "Table not found locally"
    }
    .to_string();

    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Deleted)
        .await;

    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    for target in targets {
        if target != *current_addr.get_ref() {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_clone = client.clone();
            let secret_clone = cluster_secret.clone();

            // Acquire permit to cap concurrent outgoing replication tasks
            let permit = <Arc<Semaphore> as Clone>::clone(&sem)
                .acquire_owned()
                .await
                .unwrap();

            tokio::spawn(async move {
                let _permit = permit; // Permit is held until the deletion request completes
                let mut retries = 3;
                let mut backoff = std::time::Duration::from_millis(100);

                while retries > 0 {
                    match client_clone
                        .delete(&url)
                        // FIX: Send the actual secret instead of "true"
                        .header("X-Internal-Request", &secret_clone)
                        .timeout(std::time::Duration::from_secs(3))
                        .send()
                        .await
                    {
                        Ok(resp) if resp.status().is_success() => break,
                        _ => {
                            retries -= 1;
                            if retries > 0 {
                                tokio::time::sleep(backoff).await;
                                backoff *= 2;
                            }
                        }
                    }
                }
            });
        }
    }

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
    if let Some(table_db) = state.store.get(&table_name) {
        HttpResponse::Ok().json(&*table_db)
    } else {
        HttpResponse::NotFound().json(json!({"error": "Table not found"}))
    }
}

pub async fn get_all_keys(table: web::Path<String>, state: web::Data<AppState>) -> impl Responder {
    let table_name = table.into_inner();
    if let Some(table_db) = state.store.get(&table_name) {
        let keys: Vec<String> = table_db.iter().map(|kv| kv.key().clone()).collect();
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

    if let Some(table_data) = state.store.get(&table_name) {
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
    // ADDED: Shared client and semaphore
    client: web::Data<reqwest::Client>,
    sem: web::Data<Arc<Semaphore>>,
) -> impl Responder {
    let cluster_secret =
        env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());

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

        // Safely bound the background sleep/gossip task.
        // If the system is maxed out, we skip the background gossip rather than queuing unbound tasks.
        if let Ok(permit) = <Arc<Semaphore> as Clone>::clone(&sem).try_acquire_owned() {
            tokio::spawn(async move {
                let _permit = permit; // Permit held during the sleep and gossip phase
                time::sleep(std::time::Duration::from_secs(5)).await;

                let args: Vec<String> = env::args().collect();
                if args.len() > 1 {
                    let current_node = args[1].clone();
                    gossip_membership(&cluster2, &client, &current_node).await;
                }
            });
        } else {
            eprintln!(
                "Warning: Dropping gossip broadcast for new node {} due to high load",
                new_node
            );
        }
    }

    HttpResponse::Ok().json(nodes_guard.clone())
}
pub async fn get_global_store(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    let valid_api_key = env::var("CLUSTER_SECRET").unwrap_or_else(|_| "default_secret".to_string());
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
    HttpResponse::Ok().json(&state.store)
}
