use std::collections::HashMap;
use std::time::Duration;
use reqwest;
use actix_web::{web, HttpResponse, Responder};
use serde_json::json;
use crate::storage::engine::{current_timestamp, ClusterData, NodeInfo, NodeStatus};

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
        check_node_health(&cluster_data, &client, &current_addr, interval_sec).await;

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
    let timeout_threshold = interval_sec as u128 * 2000; // 2x interval in milliseconds

    for (node, info) in nodes_guard.iter_mut() {
        // Skip self-check
        if node == current_addr {
            continue;
        }
        let url = if node.starts_with("http://") || node.starts_with("https://") {
            // Remove any trailing slash for consistency, then append the endpoint.
            format!("{}/heartbeat", node.trim_end_matches('/'))
        } else {
            // If no scheme is present, use http as the default.
            format!("http://{}/heartbeat", node)
        };

        match client.get(&url).timeout(Duration::from_secs(2)).send().await {
            Ok(resp) if resp.status().is_success() => {
                // Successful heartbeat - mark node as active
                info.last_heartbeat = current_timestamp();
                info.status = NodeStatus::Active;
            },
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
        },
        NodeStatus::Suspect => {
            // Check if it's been down too long
            let now = current_timestamp();
            if now - info.last_heartbeat > timeout_threshold {
                info.status = NodeStatus::Down;
            }
        },
        _ => {} // For other states, do nothing
    }
}

/// Send membership information to all other nodes
async fn gossip_membership(
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

        let gossip_url = format!("http://{}/update_membership", node);
        let membership_clone = nodes_snapshot.clone();
        let client_clone = client.clone();
        let node_clone = node.clone();

        // Send update asynchronously
        tokio::spawn(async move {
            if let Err(e) = send_membership_update(&client_clone, &gossip_url, &membership_clone).await {
                eprintln!("Error gossiping membership to {}: {}", node_clone, e);
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
        },
        Err(e) => Err(e.to_string()),
    }
}

//
// MEMBERSHIP HTTP ENDPOINTS
//

/// Respond to heartbeat requests from other nodes
pub async fn heartbeat() -> impl Responder {
    HttpResponse::Ok().json(json!({"status": "ok", "timestamp": current_timestamp()}))
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
        "active_count": nodes.values().filter(|n| n.status == NodeStatus::Active).count(),
        "timestamp": current_timestamp()
    });

    HttpResponse::Ok().json(response)
}
#[cfg(test)]
mod tests {
    use super::*;
    use actix_test::start;
    use actix_web::{test, web, HttpResponse};
    use actix_web::dev::ServiceResponse;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    // Import types from engine as needed.
    use crate::storage::engine::{current_timestamp, ClusterData, NodeInfo, NodeStatus};

    // --- Test update_node_status ---
    #[actix_web::test]
    async fn test_update_node_status() {
        // Create a node initially Active.
        let mut info = NodeInfo {
            status: NodeStatus::Active,
            last_heartbeat: current_timestamp() - 100,
        };
        let threshold = 500;
        // Active node with a missed heartbeat becomes Suspect.
        update_node_status(&mut info, threshold);
        assert_eq!(info.status, NodeStatus::Suspect);

        // Now, a Suspect node becomes Down if too much time has passed.
        info.status = NodeStatus::Suspect;
        info.last_heartbeat = current_timestamp() - (threshold + 100);
        update_node_status(&mut info, threshold);
        assert_eq!(info.status, NodeStatus::Down);

        // A Down node remains Down.
        info.status = NodeStatus::Down;
        let old = info.last_heartbeat;
        update_node_status(&mut info, threshold);
        assert_eq!(info.status, NodeStatus::Down);
        assert_eq!(info.last_heartbeat, old);
    }

    // --- Test send_membership_update using a local server ---
    #[actix_web::test]
    async fn test_send_membership_update_success() {
        // Spawn a test HTTP server that returns 200 OK on POST.
        let srv = start(|| {
            actix_web::App::new().route("/", web::post().to(|| async {
                HttpResponse::Ok().finish()
            }))
        });
        let url = srv.url("/");
        // Create a dummy membership: one node.
        let mut membership: HashMap<String, NodeInfo> = HashMap::new();
        membership.insert(
            "node1".to_string(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
        let client = reqwest::Client::new();
        let result = send_membership_update(&client, &url, &membership).await;
        assert!(result.is_ok());
    }

    #[actix_web::test]
    async fn test_send_membership_update_failure() {
        // Use an address that is unlikely to be listening.
        let url = "http://127.0.0.1:12345/invalid";
        let mut membership: HashMap<String, NodeInfo> = HashMap::new();
        membership.insert(
            "node1".to_string(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
        let client = reqwest::Client::new();
        let result = send_membership_update(&client, url, &membership).await;
        assert!(result.is_err());
    }

    // --- Test HTTP endpoint: heartbeat ---
    #[actix_web::test]
    async fn test_heartbeat_endpoint() {
        let response = heartbeat().await;
        let dummy_req = test::TestRequest::default().to_http_request();
        // Convert the responder into an HttpResponse with a boxed body.
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        assert_eq!(http_resp.status(), 200);
        let service_resp = ServiceResponse::new(dummy_req, http_resp);
        let body: Value = test::read_body_json(service_resp).await;
        assert_eq!(body.get("status").unwrap(), "ok");
        assert!(body.get("timestamp").is_some());
    }

    // --- Test HTTP endpoint: update_membership ---
    #[actix_web::test]
    async fn test_update_membership_endpoint() {
        // Create an empty ClusterData.
        let cluster_data = web::Data::new(ClusterData {
            nodes: Arc::new(RwLock::new(HashMap::new())),
        });
        // Create a payload that adds one node.
        let mut payload: HashMap<String, NodeInfo> = HashMap::new();
        let node_info = NodeInfo {
            status: NodeStatus::Active,
            last_heartbeat: current_timestamp(),
        };
        payload.insert("node1".to_string(), node_info.clone());
        let payload = web::Json(payload);
        let response = update_membership(cluster_data.clone(), payload).await;
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        assert_eq!(http_resp.status(), 200);
        // Verify that the node was added.
        let guard = cluster_data.nodes.read().await;
        assert!(guard.contains_key("node1"));
    }

    // --- Test HTTP endpoint: get_membership ---
    #[actix_web::test]
    async fn test_get_membership_endpoint() {
        // Create a ClusterData with a preset node.
        let mut nodes = HashMap::new();
        nodes.insert(
            "node1".to_string(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
        let cluster_data = web::Data::new(ClusterData {
            nodes: Arc::new(RwLock::new(nodes.clone())),
        });
        let response = get_membership(cluster_data.clone()).await;
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        assert_eq!(http_resp.status(), 200);
        let service_resp = ServiceResponse::new(dummy_req, http_resp);
        let body: Value = test::read_body_json(service_resp).await;
        // Check that the JSON contains "nodes", "active_count", and "timestamp".
        assert!(body.get("nodes").is_some());
        assert!(body.get("active_count").is_some());
        assert!(body.get("timestamp").is_some());
        let active_count = nodes.values().filter(|n| n.status == NodeStatus::Active).count();
        assert_eq!(body.get("active_count").unwrap().as_u64().unwrap(), active_count as u64);
    }

    // --- Test check_node_health ---
    #[actix_web::test]
    async fn test_check_node_health() {
        // Spawn a test server that returns 200 OK for GET /heartbeat.
        let srv = start(|| {
            actix_web::App::new().route("/heartbeat", web::get().to(|| async {
                HttpResponse::Ok().json(json!({"status": "ok"}))
            }))
        });
        let heartbeat_addr = srv.url("");
        // Create ClusterData with two nodes: self and the external server.
        let current_addr = "self_address".to_string();
        let mut nodes = HashMap::new();
        nodes.insert(
            current_addr.clone(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
        nodes.insert(
            heartbeat_addr.clone(),
            NodeInfo {
                status: NodeStatus::Suspect,
                last_heartbeat: current_timestamp() - 10_000,
            },
        );
        let cluster_data = web::Data::new(ClusterData {
            nodes: Arc::new(RwLock::new(nodes)),
        });
        let client = reqwest::Client::new();
        // Call check_node_health; the external node should update to Active.
        check_node_health(&cluster_data, &client, &current_addr, 1).await;
        let guard = cluster_data.nodes.read().await;
        let external = guard.get(&heartbeat_addr).unwrap();
        assert_eq!(external.status, NodeStatus::Active);
    }

    // --- Test gossip_membership ---
    #[actix_web::test]
    async fn test_gossip_membership() {
        // Create ClusterData with two nodes: self and one external node.
        let current_addr = "self_address".to_string();
        let external_addr = "http://127.0.0.1:0".to_string(); // Use an invalid port to simulate failure.
        let mut nodes = HashMap::new();
        nodes.insert(
            current_addr.clone(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
        nodes.insert(
            external_addr.clone(),
            NodeInfo {
                status: NodeStatus::Active,
                last_heartbeat: current_timestamp(),
            },
        );
        let cluster_data = web::Data::new(ClusterData {
            nodes: Arc::new(RwLock::new(nodes)),
        });
        let client = reqwest::Client::new();
        // Call gossip_membership. It spawns tasks for external nodes.
        // Errors due to external_addr will be printed but should not panic.
        gossip_membership(&cluster_data, &client, &current_addr).await;
    }
}
