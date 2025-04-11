use std::time::Duration;
use reqwest;
use actix_web::{web, HttpResponse, Responder};
use crate::storage::engine::ClusterData;

/// Membership sync task.
/// Every `interval_sec` seconds, this task:
/// 1. Logs the current membership,
/// 2. Pulls each remote node’s membership (via GET /membership),
/// 3. Merges any new nodes into its local membership,
/// 4. Collects remote nodes that are unreachable and removes them,
/// 5. If any changes occurred (new nodes merged or unreachable nodes removed),
///    pushes the updated membership list to all remote nodes (via POST /update_membership).
pub async fn membership_sync(
    cluster_data: web::Data<ClusterData>,
    current_addr: String,
    interval_sec: u64,
) {
    let client = reqwest::Client::new();

    loop {
        // Wait for the defined interval.
        tokio::time::sleep(Duration::from_secs(interval_sec)).await;
        let mut local_updated = false;
        let mut unreachable: Vec<String> = Vec::new();

        // Clone the current membership for iteration.
        let local_nodes = { cluster_data.nodes.lock().unwrap().clone() };

        // Query each remote node (except our own) for its membership.
        for node in local_nodes.iter() {
            if node == &current_addr {
                continue;
            }

            let url = format!("http://{}/membership", node);
            match client.get(&url).send().await {
                Ok(resp) => {
                    if let Ok(remote_nodes) = resp.json::<Vec<String>>().await {
                        // Merge any new remote nodes into local membership.
                        let mut local_guard = cluster_data.nodes.lock().unwrap();
                        for r_node in remote_nodes {
                            if !local_guard.contains(&r_node) {
                                local_guard.push(r_node);
                                local_updated = true;
                            }
                        }
                    } else {
                        println!(
                            "Node {} - Failed to parse membership from {}",
                            current_addr, node
                        );
                    }
                }
                Err(e) => {
                    println!(
                        "Node {} - Error contacting {} for membership: {}",
                        current_addr, node, e
                    );
                    unreachable.push(node.clone());
                }
            }
        }

        // Remove unreachable nodes from the membership.
        if !unreachable.is_empty() {
            let mut local_guard = cluster_data.nodes.lock().unwrap();
            let before_removal = local_guard.clone();
            local_guard.retain(|n| !unreachable.contains(n));
            if local_guard.len() < before_removal.len() {
                local_updated = true;
                println!(
                    "Node {} - Removed unreachable nodes: {:?}",
                    current_addr,
                    unreachable
                );
            }
        }

        // Log the final membership after merging and removal.
        {
            let final_nodes = cluster_data.nodes.lock().unwrap().clone();
            println!(
                "Node {} - {:?}",
                current_addr, final_nodes
            );
        }

        // If membership changed (either new nodes added or unreachable removed),
        // push the updated membership list to all remote peers.
        if local_updated {
            let updated_membership = cluster_data.nodes.lock().unwrap().clone();
            for node in updated_membership.iter() {
                if node == &current_addr {
                    continue;
                }
                let update_url = format!("http://{}/update_membership", node);
                let push_payload = &updated_membership;

                match client.post(&update_url).json(&push_payload).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            println!(
                                "Node {} - Pushed membership update to {}",
                                current_addr, node
                            );
                        } else {
                            println!(
                                "Node {} - Push update to {} failed: {}",
                                current_addr,
                                node,
                                resp.status()
                            );
                        }
                    }
                    Err(e) => println!(
                        "Node {} - Error pushing update to {}: {}",
                        current_addr, node, e
                    ),
                }
            }
        }
    }
}

pub async fn update_membership(
    cluster_data: web::Data<ClusterData>,
    payload: web::Json<Vec<String>>,
) -> impl Responder {
    let mut local_guard = cluster_data.nodes.lock().unwrap();
    let mut updated = false;
    for node in payload.into_inner() {
        if !local_guard.contains(&node) {
            local_guard.push(node);
            updated = true;
        }
    }
    if updated {
        println!("Updated membership: {:?}", local_guard);
    }
    HttpResponse::Ok().finish()
}

/// GET membership handler: returns the current membership list as JSON.
pub async fn get_membership(cluster_data: web::Data<ClusterData>) -> impl Responder {
    let nodes = cluster_data.nodes.lock().unwrap().clone();
    HttpResponse::Ok().json(nodes)
}