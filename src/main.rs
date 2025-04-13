mod storage;
mod network;
mod tokenizer;

use actix_web::{web, App, HttpServer};
use once_cell::sync::OnceCell;
use reqwest;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::process;
use std::sync::Mutex;

use storage::engine::{
    AppState, ClusterData, delete_value, get_value, get_table_store, join_cluster, put_value,
};
use storage::persistance::{cold_save, load_all_tables};

/// Declare APP_STATE globally so that it’s available throughout the module.
static APP_STATE: OnceCell<web::Data<AppState>> = OnceCell::new();

/// Merge the remote state for a given table into the local state.
/// For each key (i.e. partition) from the remote state:
/// - If the key already exists, update its attributes.
/// - Otherwise, insert the new key with its attributes.
fn merge_table_state(
    local: &mut HashMap<String, HashMap<String, Value>>,
    remote: HashMap<String, HashMap<String, Value>>,
) {
    for (key, remote_partition) in remote {
        local
            .entry(key)
            .and_modify(|local_partition| {
                for (attr, value) in remote_partition.iter() {
                    local_partition.insert(attr.clone(), value.clone());
                }
            })
            .or_insert(remote_partition);
    }
}

/// Merge the entire global remote store into the local store.
/// Both stores have the type:
/// HashMap<table_name, HashMap<partition_key, HashMap<attribute, Value>>>
fn merge_global_store(
    local: &mut HashMap<String, HashMap<String, HashMap<String, Value>>>,
    remote: HashMap<String, HashMap<String, HashMap<String, Value>>>,
) {
    for (table_name, remote_table) in remote {
        local
            .entry(table_name.clone())
            .and_modify(|local_table| {
                // Clone remote_table so we don't move it
                merge_table_state(local_table, remote_table.clone());
            })
            .or_insert(remote_table);
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Usage: <program> <current_node_address> [join_node_address]
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: {} <current_node_address> [join_node_address]",
            args[0]
        );
        process::exit(1);
    }
    let current_node = args[1].clone();
    // Use the current node address for binding.
    let bind_addr = current_node.clone();

    // Initialize the local in‑memory multi‑table key–value store.
    // Each table name maps to a key–value store:
    //    table_name -> (key -> attributes)
    let state = web::Data::new(AppState {
        // Specify a base directory for table folders.
        base_dir: "./data",
        store: Mutex::new(HashMap::new()),
    });
    // Set the global APP_STATE pointer.
    let _ = APP_STATE.set(state.clone());

    // Load local cold storage from disk by enumerating each table folder.
    match load_all_tables(&state) {
        Ok(_) => println!("Cold storage loaded"),
        Err(e) => eprintln!("Error loading cold storage: {}", e),
    }

    // (Optionally) Ensure the default table exists in memory.
    {
        let default_table = "default".to_string();
        let mut store = state.store.lock().unwrap();
        store.entry(default_table).or_insert(HashMap::new());
    }

    // If a join node is provided, join its cluster and pull its global state.
    if args.len() >= 3 {
        let join_node = args[2].clone();

        // Send join request.
        let client = reqwest::Client::new();
        let join_url = format!("http://{}/join", join_node);
        match client
            .post(&join_url)
            .json(&serde_json::json!({ "node": current_node }))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(nodes) = response.json::<Vec<String>>().await {
                        println!("Joined cluster: {:?}", nodes);
                    }
                } else {
                    println!(
                        "Failed to join cluster (status = {})",
                        response.status()
                    );
                }
            }
            Err(e) => println!("Error joining cluster: {}", e),
        }

        // Pull the remote state for all tables.
        // This requires that the remote node exposes a global state endpoint at GET /store.
        let store_url = format!("http://{}/store", join_node);
        let client = reqwest::Client::new();
        match client.get(&store_url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    let remote_store: HashMap<
                        String,
                        HashMap<String, HashMap<String, Value>>,
                    > = resp.json().await.unwrap_or_else(|e| {
                        eprintln!("Failed to parse global cold storage: {}", e);
                        HashMap::new()
                    });
                    {
                        let mut store = state.store.lock().unwrap();
                        merge_global_store(&mut store, remote_store);
                        println!(
                            "Merged global cold storage from {}: {:?}",
                            join_node, *store
                        );
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
                    join_node, e,
                );
            }
        }
    }

    // Spawn the periodic cold save task.
    tokio::spawn({
        let state = state.clone();
        async move {
            cold_save(state, 30).await;
        }
    });

    // Initialize cluster data with dynamic membership.
    let cluster_data = web::Data::new(ClusterData {
        nodes: std::sync::Arc::new(Mutex::new(vec![current_node.clone()])),
    });

    // Spawn the membership synchronization background task.
    let cluster_data_clone = cluster_data.clone();
    let current_node_clone = current_node.clone();
    tokio::spawn(network::broadcaster::membership_sync(
        cluster_data_clone,
        current_node_clone,
        60,
    ));

    println!("Starting distributed DB engine at http://{}", current_node);

    // Build and run the HTTP server.
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(cluster_data.clone())
            .app_data(web::Data::new(current_node.clone()))
            // Endpoint to join the cluster.
            .route("/join", web::post().to(join_cluster))
            // Endpoints for updating/fetching membership.
            .route(
                "/membership",
                web::get().to(network::broadcaster::get_membership),
            )
            .route(
                "/update_membership",
                web::post().to(network::broadcaster::update_membership),
            )
            // Key endpoints with multi‑table support: the table name is part of the URL.
            .route("/{table}/key/{key}", web::get().to(get_value))
            .route("/{table}/key/{key}", web::put().to(put_value))
            .route("/{table}/key/{key}", web::delete().to(delete_value))
            // Endpoint to fetch a table’s entire in‑memory store.
            .route("/{table}/store", web::get().to(get_table_store))
            // Global endpoint returning the entire in‑memory store.
            .route("/store", web::get().to(|state: web::Data<AppState>| async move {
                web::Json(state.store.lock().unwrap().clone())
            }))
    })
        .bind(bind_addr.as_str())?
        .run()
        .await
}
