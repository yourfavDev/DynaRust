use std::collections::HashMap;
use std::env;
use std::time::Duration;
use reqwest;
use actix_web::{web, HttpResponse, Responder};
use serde_json::json;
use crate::storage::engine::{
    current_timestamp, ClusterData, NodeInfo, NodeStatus,
};

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

/// Check health of all nodes and update their status
async fn check_node_health(
    cluster_data: &web::Data<ClusterData>,
    client: &reqwest::Client,
    current_addr: &str,
    interval_sec: u64,
) {
    let mut nodes_guard = cluster_data.nodes.write().await;
    // 2x interval in milliseconds
    let timeout_threshold = (interval_sec as u128) * 2000;

    for (node, info) in nodes_guard.iter_mut() {
        // Skip self-check
        if node == current_addr {
            continue;
        }
        let url = build_url(node, "heartbeat");

        match client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                // Successful heartbeat - mark node as active
                info.last_heartbeat = current_timestamp();
                info.status = NodeStatus::Active;
            }
            _ => {
                // Failed heartbeat - update status based on current state
                update_node_status(info, timeout_threshold);
            }
        }
    }

    println!("Node {} membership: {:?}", current_addr, *nodes_guard);
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

/// Send membership information to all other nodes
pub async fn gossip_membership(
    cluster_data: &web::Data<ClusterData>,
    client: &reqwest::Client,
    current_addr: &str,
) {
    // Get a snapshot of current membership
    let nodes_snapshot = {
        let guard = cluster_data.nodes.read().await;
        guard.clone()
    };

    // Send membership updates to each node
    for (node, _) in nodes_snapshot.iter() {
        if node == current_addr {
            continue;
        }

        let gossip_url = build_url(node, "update_membership");
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
    match client.post(url).json(membership).send().await {
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

/// Process membership updates received from other nodes
pub async fn update_membership(
    cluster_data: web::Data<ClusterData>,
    payload: web::Json<HashMap<String, NodeInfo>>,
) -> impl Responder {
    let mut local_guard = cluster_data.nodes.write().await;
    let incoming = payload.into_inner();
    let mut updated = false;

    for (node, info) in incoming.into_iter() {
        // For nodes not in our membership, add them
        if !local_guard.contains_key(&node) {
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
