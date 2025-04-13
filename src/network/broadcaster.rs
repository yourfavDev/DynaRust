use std::collections::HashMap;
use std::time::Duration;
use reqwest;
use actix_web::{web, HttpResponse, Responder};
use serde_json::json;
use crate::storage::engine::{current_timestamp, ClusterData, NodeInfo, NodeStatus};

pub async fn membership_sync(
    cluster_data: web::Data<ClusterData>,
    current_addr: String,
    interval_sec: u64,
) {
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(Duration::from_secs(interval_sec)).await;
        {
            let mut nodes_guard = cluster_data.nodes.write().await;
            for (node, info) in nodes_guard.iter_mut() {
                if node == &current_addr {
                    continue;
                }
                let url = format!("http://{}/heartbeat", node);
                match client.get(&url).timeout(Duration::from_secs(2)).send().await {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            info.last_heartbeat = current_timestamp();
                            info.status = NodeStatus::Active;
                        } else {
                            if info.status == NodeStatus::Active {
                                info.status = NodeStatus::Suspect;
                            }
                        }
                    }
                    Err(_) => {
                        if info.status == NodeStatus::Active {
                            info.status = NodeStatus::Suspect;
                        } else if info.status == NodeStatus::Suspect {
                            let now = current_timestamp();
                            if now - info.last_heartbeat > (interval_sec as u128 * 2000) {
                                info.status = NodeStatus::Down;
                            }
                        }
                    }
                }
            }
        }
        let nodes_snapshot = cluster_data.nodes.read().await;
        println!("Node {} membership: {:?}", current_addr, *nodes_snapshot);
        // Optional: implement gossip here.
    }
}

pub async fn heartbeat(_cluster_data: web::Data<ClusterData>) -> impl Responder {
    HttpResponse::Ok().json(json!({"status": "ok"}))
}

pub async fn update_membership(
    cluster_data: web::Data<ClusterData>,
    payload: web::Json<HashMap<String, NodeInfo>>,
) -> impl Responder {
    let mut local_guard = cluster_data.nodes.write().await;
    let incoming = payload.into_inner();
    let mut updated = false;
    for (node, info) in incoming.into_iter() {
        if !local_guard.contains_key(&node) {
            local_guard.insert(node, info);
            updated = true;
        } else {
            let existing = local_guard.get(&node).unwrap();
            if info.last_heartbeat > existing.last_heartbeat {
                local_guard.insert(node, info);
                updated = true;
            }
        }
    }
    if updated {
        println!("Updated membership: {:?}", *local_guard);
    }
    HttpResponse::Ok().finish()
}

pub async fn get_membership(
    cluster_data: web::Data<ClusterData>,
) -> impl Responder {
    let nodes = cluster_data.nodes.read().await;
    HttpResponse::Ok().json(nodes.clone())
}
