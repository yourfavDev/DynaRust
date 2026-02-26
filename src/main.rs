mod storage;
mod network;
mod tokenizer;
mod security;
use openssl::ssl::{SslAcceptor, SslFiletype, SslMethod, SslVerifyMode};

use actix_web::{web, App, HttpServer};
use once_cell::sync::OnceCell;
use reqwest;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::process;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use storage::engine::{
    AppState, ClusterData, delete_value, get_value, get_table_store, join_cluster, put_value,
    get_all_keys, get_multiple_keys, NodeInfo, NodeStatus, current_timestamp,
    VersionedValue, get_global_store
};
use storage::persistance::{cold_save, load_all_tables};
use network::broadcaster::{membership_sync, heartbeat, get_membership, update_membership};
use crate::storage::engine::put_value_internal;
use crate::storage::snapshoting::start_snapshot_task;
use crate::storage::statistics::{get_stats, MetricsCollector, MetricsMiddleware};
use crate::storage::subscription::SubscriptionManager;

/// Declare APP_STATE globally so that it’s available throughout the module.
static APP_STATE: OnceCell<web::Data<AppState>> = OnceCell::new();

/// Merge the remote state for a given table into the local state.
fn merge_table_state(
    local: &mut HashMap<String, VersionedValue>,
    remote: HashMap<String, VersionedValue>,
) {
    for (key, remote_val) in remote {
        local
            .entry(key)
            .and_modify(|local_val| {
                if remote_val.version > local_val.version {
                    *local_val = remote_val.clone();
                }
            })
            .or_insert(remote_val);
    }
}

/// Merge the entire global remote store into the local store.
fn merge_global_store(
    local: &mut HashMap<String, HashMap<String, VersionedValue>>,
    remote: HashMap<String, HashMap<String, VersionedValue>>,
) {
    for (table, remote_table) in remote {
        local
            .entry(table.clone())
            .and_modify(|local_table| {
                merge_table_state(local_table, remote_table.clone());
            })
            .or_insert(remote_table);
    }
}


#[actix_web::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().unwrap();
    // Usage: <program> <current_node_address> [join_node_address]
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <current_node_address> [join_node_address]", args[0]);
        process::exit(1);
    }
    let current_node = args[1].clone();
    // Use the current node address for binding.
    let bind_addr = current_node.clone();
    match env::var("JWT_SECRET") {
        Ok(secret) => secret,
        Err(_) => {
            eprintln!("JWT_SECRET environment variable not set");
            std::process::exit(1);
        }
    };

    // Initialize the local in‑memory multi‑table key–value store.
    let state = web::Data::new(AppState {
        base_dir: "./data",
        store: RwLock::new(HashMap::new()),
    });
    let _ = APP_STATE.set(state.clone());

    // Load cold storage (snapshot + WAL replay) from disk.
    match load_all_tables(&state).await {
        Ok(_) => println!("Cold storage loaded"),
        Err(e) => eprintln!("Error loading cold storage: {}", e),
    }

    // (Optionally) Ensure the default table exists in memory.
    {
        let mut store = state.store.write().await;
        store.entry("default".to_string()).or_insert(HashMap::new());
    }
    start_snapshot_task(state.clone());

    // If a join node is provided, join its cluster and merge its global state.
    if args.len() >= 3 {
        let join_node = args[2].clone();

        // Send join request.
        let client = reqwest::Client::new();
        let join_url = format!("http://{}/join", join_node);
        match client
            .post(&join_url)
            .json(&json!({
                "node": current_node,
                "secret": env::var("CLUSTER_SECRET").unwrap_or_default()
            }))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(nodes) = response.json::<HashMap<String, NodeInfo>>().await {
                        println!("Joined cluster: {:?}", nodes);
                    }
                } else {
                    println!("Failed to join cluster (status = {})", response.status());
                }
            }
            Err(e) => println!("Error joining cluster: {}", e),
        }


        // Pull the remote state for all tables.
        let store_url = format!("http://{}/store", join_node);
        match client.get(&store_url).header("x-api-key", std::env::var("CLUSTER_SECRET").unwrap_or_default()).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    let remote_store = resp
                        .json::<HashMap<String, HashMap<String, VersionedValue>>>()
                        .await
                        .unwrap_or_else(|e| {
                            eprintln!("Failed to parse global cold storage: {}", e);
                            HashMap::new()
                        });
                    {
                        let mut store = state.store.write().await;
                        merge_global_store(&mut store, remote_store);
                        println!("Merged global cold storage: {:?}", *store);
                    }
                } else {
                    eprintln!(
                        "Failed to get global cold storage from {}: {:?}",
                        join_node,
                        resp.status()
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "Error fetching global cold storage from {}: {}",
                    join_node, e
                );
            }
        }
    }
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();

    // 2) Build a semaphore to cap at 20 concurrent replications
    let sem = Arc::new(Semaphore::new(20));
    // Spawn the periodic cold save task.
    tokio::spawn(cold_save(state.clone(), 30));

    // Initialize cluster data with dynamic membership.
    let mut initial_nodes = HashMap::new();
    initial_nodes.insert(
        current_node.clone(),
        NodeInfo {
            status: NodeStatus::Active,
            last_heartbeat: current_timestamp(),
        },
    );
    let cluster_data = web::Data::new(ClusterData {
        nodes: Arc::new(RwLock::new(initial_nodes)),
    });

    // Spawn the membership synchronization (heartbeat) task.
    let cluster_clone = cluster_data.clone();
    let current_clone = current_node.clone();
    tokio::spawn(membership_sync(cluster_clone, current_clone, 60));
    let subscription_manager = web::Data::new(SubscriptionManager::new());

    // Initialize metrics collector and middleware
    let metrics_collector = web::Data::new(MetricsCollector::new());
    // extract a clonable handle for the middleware
    let mw_col = metrics_collector.get_ref().clone();

    let use_https = match env::var("DYNA_MODE").unwrap_or_default().as_str() {
        "https" => true,
        _ => false
    };
    match use_https {
        true => {
            let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
            builder
                .set_private_key_file("./cert/server.key", SslFiletype::PEM)
                .unwrap();
            builder
                .set_certificate_chain_file("./cert/server.crt")
                .unwrap();
            builder
                .set_ca_file("./cert/ca.crt")
                .unwrap();
            builder.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            println!("Starting distributed DB engine at https://{}", current_node);

            // Build and run the HTTP server.
            HttpServer::new(move || {
                App::new()
                    .wrap(MetricsMiddleware::new(mw_col.clone()))
                    .app_data(metrics_collector.clone())
                    .app_data(subscription_manager.clone())
                    .app_data(state.clone())
                    .app_data(web::Data::new(http_client.clone()))
                    .app_data(web::Data::new(sem.clone()))
                    .app_data(cluster_data.clone())
                    .app_data(web::Data::new(current_node.clone()))
                    // Cluster management endpoints.
                    .route("/join", web::post().to(join_cluster))
                    .route("/membership", web::get().to(get_membership))
                    .route("/update_membership", web::post().to(update_membership))
                    .route("/heartbeat", web::get().to(heartbeat))
                    .route(
                        "/internal/{table}/key/{key}",
                        web::put().to(put_value_internal),
                    )
                    // Key–value endpoints with multi‑table support.
                    .route("/{table}/key/{key}", web::get().to(get_value))
                    .route("/{table}/key/{key}", web::put().to(put_value))
                    .route("/{table}/key/{key}", web::delete().to(delete_value))
                    // Endpoint to fetch a table’s entire in‑memory store.
                    .route("/{table}/store", web::get().to(get_table_store))
                    // Global endpoint returning the entire in‑memory store.
                    .route("/store", web::get().to(get_global_store))
                    // Endpoints to get keys from a table.
                    .route("/{table}/keys", web::get().to(get_all_keys))
                    .route("/{table}/keys", web::post().to(get_multiple_keys))
                    .route("/{table}/subscribe/{key}", web::get().to(
                        storage::subscription::subscribe_to_key
                    ))
                    .route("/auth/{user}", web::post().to(security::authentication::access))
                    .service(get_stats)
            })
                .bind_openssl(bind_addr.as_str(), builder)?
                .run()
                .await
        }
        false => {
            println!("Starting distributed DB engine at http://{}", current_node);

            // Build and run the HTTP server.
            HttpServer::new(move || {
                App::new()
                    .wrap(MetricsMiddleware::new(mw_col.clone()))
                    .app_data(metrics_collector.clone())
                    .app_data(subscription_manager.clone())
                    .app_data(state.clone())
                    .app_data(web::Data::new(http_client.clone()))
                    .app_data(web::Data::new(sem.clone()))
                    .app_data(cluster_data.clone())
                    .app_data(web::Data::new(current_node.clone()))
                    // Cluster management endpoints.
                    .route("/join", web::post().to(join_cluster))
                    .route("/membership", web::get().to(get_membership))
                    .route("/update_membership", web::post().to(update_membership))
                    .route("/heartbeat", web::get().to(heartbeat))
                    .route(
                        "/internal/{table}/key/{key}",
                        web::put().to(put_value_internal),
                    )
                    // Key–value endpoints with multi‑table support.
                    .route("/{table}/key/{key}", web::get().to(get_value))
                    .route("/{table}/key/{key}", web::put().to(put_value))
                    .route("/{table}/key/{key}", web::delete().to(delete_value))
                    // Endpoint to fetch a table’s entire in‑memory store.
                    .route("/{table}/store", web::get().to(get_table_store))
                    // Global endpoint returning the entire in‑memory store.
                    .route("/store", web::get().to(get_global_store))
                    // Endpoints to get keys from a table.
                    .route("/{table}/keys", web::get().to(get_all_keys))
                    .route("/{table}/keys", web::post().to(get_multiple_keys))
                    .route("/{table}/subscribe/{key}", web::get().to(
                        storage::subscription::subscribe_to_key
                    ))
                    .route("/auth/{user}", web::post().to(security::authentication::access))
                    .service(get_stats)
            })
                .bind(bind_addr.as_str())?
                .run()
                .await
        }
    }

}