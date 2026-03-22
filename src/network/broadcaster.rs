use std::collections::HashMap;
use std::env;
use std::time::Duration;
use reqwest;
use actix_web::{HttpRequest, HttpResponse, Responder, web};
use serde_json::json;
use rand::seq::SliceRandom;
use crate::storage::engine::{
    current_timestamp, ClusterData, NodeInfo, NodeStatus,
};

#[derive(serde::Deserialize, serde::Serialize)]
pub struct IndirectPingRequest {
    pub target: String,
}

/// Respond to indirect ping requests by pinging the target node
pub async fn indirect_ping(
    req: web::Json<IndirectPingRequest>,
    client: web::Data<reqwest::Client>,
) -> impl Responder {
    let url = build_url(&req.target, "heartbeat");
    match client.get(&url).timeout(Duration::from_secs(2)).send().await {
        Ok(resp) if resp.status().is_success() => HttpResponse::Ok().json(json!({"status": "ok"})),
        _ => HttpResponse::ServiceUnavailable().json(json!({"status": "failed"})),
    }
}

/// Read DYNA_MODE and return either "https" or "http".
fn get_scheme() -> &'static str {
    match env::var("DYNA_MODE").map(|m| m.to_lowercase()).as_deref() {
        Ok("https") => "https",
        _ => "http",
    }
}

/// Strip any existing scheme/trailing slash from `node` and build a full URL.
fn build_url(node: &str, endpoint: &str) -> String {
    let scheme = get_scheme();
    let host = node
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let ep = endpoint.trim_start_matches('/');
    format!("{}://{}/{}", scheme, host, ep)
}

//
// MEMBERSHIP BACKGROUND TASK
//

pub async fn membership_sync(
    cluster_data: web::Data<ClusterData>,
    current_addr: String,
    interval_sec: u64,
) {
    let client = reqwest::Client::new();

    loop {
        // Wait for the specified interval
        tokio::time::sleep(Duration::from_secs(interval_sec)).await;

        // Run health checks and update node statuses
        check_node_health(
            &cluster_data,
            &client,
            &current_addr,
            interval_sec,
        )
            .await;

        // Distribute membership information to other nodes
        gossip_membership(&cluster_data, &client, &current_addr).await;
    }
}

/// Check health of nodes using SWIM-inspired protocol
async fn check_node_health(
    cluster_data: &web::Data<ClusterData>,
    client: &reqwest::Client,
    current_addr: &str,
    interval_sec: u64,
) {
    let nodes_to_check = {
        let guard = cluster_data.nodes.read().await;
        let mut potential_nodes: Vec<String> = guard
            .iter()
            .filter(|(node, info)| *node != current_addr && (info.status == NodeStatus::Active || info.status == NodeStatus::Suspect))
            .map(|(node, _)| node.clone())
            .collect();

        let mut rng = rand::thread_rng();
        potential_nodes.shuffle(&mut rng);
        potential_nodes.into_iter().take(3).collect::<Vec<_>>()
    };

    let timeout_threshold = (interval_sec as u128) * 2000;

    for target in nodes_to_check {
        let url = build_url(&target, "heartbeat");
        let mut success = false;

        // 1. Direct heartbeat
        if let Ok(resp) = client.get(&url).timeout(Duration::from_secs(2)).send().await {
            if resp.status().is_success() {
                success = true;
            }
        }

        // 2. Indirect ping via 2 other nodes if direct ping failed
        if !success {
            let helpers = {
                let guard = cluster_data.nodes.read().await;
                let mut h: Vec<String> = guard.keys()
                    .filter(|&addr| addr != current_addr && addr != &target && guard.get(addr).map(|i| i.status == NodeStatus::Active).unwrap_or(false))
                    .cloned()
                    .collect();
                let mut rng = rand::thread_rng();
                h.shuffle(&mut rng);
                h.into_iter().take(2).collect::<Vec<_>>()
            };

            for helper in helpers {
                let indirect_url = build_url(&helper, "indirect_ping");
                if let Ok(resp) = client.post(&indirect_url)
                    .json(&IndirectPingRequest { target: target.clone() })
                    .timeout(Duration::from_secs(3))
                    .send()
                    .await {
                    if resp.status().is_success() {
                        success = true;
                        break;
                    }
                }
            }
        }

        // Update node status
        let mut guard = cluster_data.nodes.write().await;
        if let Some(info) = guard.get_mut(&target) {
            if success {
                info.status = NodeStatus::Active;
                info.last_heartbeat = current_timestamp();
            } else {
                update_node_status(info, timeout_threshold);
            }
        }
    }

    let guard = cluster_data.nodes.read().await;
    println!("Node {} membership: {:?}", current_addr, *guard);
}

/// Update a node's status based on heartbeat failures
fn update_node_status(info: &mut NodeInfo, timeout_threshold: u128) {
    match info.status {
        NodeStatus::Active => {
            // First failure - mark as suspect
            info.status = NodeStatus::Suspect;
        }
        NodeStatus::Suspect => {
            // Check if it's been down too long
            let now = current_timestamp();
            if now - info.last_heartbeat > timeout_threshold {
                info.status = NodeStatus::Down;
            }
        }
        _ => {} // For other states, do nothing
    }
}

/// Send membership information to up to 3 random nodes
pub async fn gossip_membership(
    cluster_data: &web::Data<ClusterData>,
    client: &reqwest::Client,
    current_addr: &str,
) {
    // Get a snapshot and select random targets
    let (nodes_snapshot, targets) = {
        let guard = cluster_data.nodes.read().await;
        let mut potential_targets: Vec<String> = guard.keys()
            .filter(|&addr| addr != current_addr && guard.get(addr).map(|i| i.status == NodeStatus::Active).unwrap_or(false))
            .cloned()
            .collect();
        
        let mut rng = rand::thread_rng();
        potential_targets.shuffle(&mut rng);
        let targets = potential_targets.into_iter().take(3).collect::<Vec<_>>();
        (guard.clone(), targets)
    };

    // Send membership updates to selected random nodes
    for node in targets {
        let gossip_url = build_url(&node, "update_membership");
        let membership_clone = nodes_snapshot.clone();
        let client_clone = client.clone();
        let node_clone = node.clone();

        // Send update asynchronously
        tokio::spawn(async move {
            if let Err(e) =
                send_membership_update(&client_clone, &gossip_url, &membership_clone)
                    .await
            {
                eprintln!(
                    "Error gossiping membership to {}: {}",
                    node_clone, e
                );
            }
        });
    }
}

/// Send a membership update to a specific node
async fn send_membership_update(
    client: &reqwest::Client,
    url: &str,
    membership: &HashMap<String, NodeInfo>,
) -> Result<(), String> {
    let cluster_token = env::var("CLUSTER_SECRET")
        .unwrap_or_else(|_| "default_secret".to_string());

    match client.post(url)
        .header("Content-Type", "application/octet-stream")
        .header("X-Cluster-Token", &cluster_token)
        .body(bincode::serialize(membership).unwrap())
        .send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(format!("Failed with status: {}", resp.status()))
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

//
// MEMBERSHIP HTTP ENDPOINTS
//

/// Respond to heartbeat requests from other nodes
pub async fn heartbeat() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "status": "ok",
        "timestamp": current_timestamp(),
    }))
}

// Define a safe upper bound for your cluster size
const MAX_CLUSTER_SIZE: usize = 100; 

/// Process membership updates received from other nodes
pub async fn update_membership(
    req: HttpRequest, // Added to inspect headers
    cluster_data: web::Data<ClusterData>,
    body: web::Bytes,
) -> impl Responder {
    // 1. Authentication Check
    // In production, load this once at startup rather than on every request
    let expected_token = env::var("CLUSTER_SECRET")
        .unwrap_or_else(|_| "default_secret".to_string());
    
    let is_authenticated = req.headers()
        .get("X-Cluster-Token")
        .and_then(|h| h.to_str().ok())
        .map_or(false, |token| token == expected_token);

    if !is_authenticated {
        return HttpResponse::Unauthorized().json(json!({
            "error": "Unauthorized: Invalid or missing cluster token"
        }));
    }

    let mut local_guard = cluster_data.nodes.write().await;
    let incoming: HashMap<String, NodeInfo> = match bincode::deserialize(&body) {
        Ok(v) => v,
        Err(_) => return HttpResponse::BadRequest().finish(),
    };
    let mut updated = false;

    for (node, info) in incoming.into_iter() {
        // 2. Input Validation (ensure it's not empty and has no spaces)
        if node.is_empty() || node.contains(' ') {
            continue;
        }

        // For nodes not in our membership, add them safely
        if !local_guard.contains_key(&node) {
            // 3. Hard Limit Check
            if local_guard.len() >= MAX_CLUSTER_SIZE {
                // Silently ignore or log a warning. Avoid heavy logging to prevent log-flooding DoS.
                eprintln!("Warning: Cluster at max capacity ({}). Ignoring new node: {}", MAX_CLUSTER_SIZE, node);
                continue; 
            }

            local_guard.insert(node, info);
            updated = true;
            continue;
        }

        // For existing nodes, only update if the incoming data is newer
        let existing = local_guard.get(&node).unwrap();
        if info.last_heartbeat > existing.last_heartbeat {
            local_guard.insert(node, info);
            updated = true;
        }
    }

    if updated {
        println!("Updated membership: {:?}", *local_guard);
    }

    HttpResponse::Ok().finish()
}
/// Get the current cluster membership state
pub async fn get_membership(
    cluster_data: web::Data<ClusterData>,
) -> impl Responder {
    let nodes = cluster_data.nodes.read().await;

    // Create a response with additional metadata
    let response = json!({
        "nodes": nodes.clone(),
        "active_count": nodes
            .values()
            .filter(|n| n.status == NodeStatus::Active)
            .count(),
        "timestamp": current_timestamp(),
    });

    HttpResponse::Ok().json(response)
}
