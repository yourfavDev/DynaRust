mod storage;
mod network;

use actix_web::{web, App, HttpServer};
use once_cell::sync::OnceCell;
use reqwest;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::process;
use std::sync::Mutex;

use storage::engine::{
    AppState, ClusterData, delete_value, get_value, get_store, join_cluster,
    put_value,
};
use storage::persistance::{cold_save, load_db};

// Declare APP_STATE globally so that it’s available throughout the module.
static APP_STATE: OnceCell<web::Data<AppState>> = OnceCell::new();

/// Merge the remote state into the local state.
/// For each key from the remote state:
/// - If the key already exists, update its attributes.
/// - Otherwise, insert the new key with its attributes.
fn merge_state(
    local: &mut HashMap<String, HashMap<String, Value>>,
    remote: HashMap<String, HashMap<String, Value>>,
) {
    for (key, remote_partition) in remote {
        local.entry(key)
            .and_modify(|local_partition| {
                for (attr, value) in remote_partition.clone() {
                    local_partition.insert(attr, value);
                }
            })
            .or_insert(remote_partition);
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
    // Create a separate clone for the bind operation.
    let bind_addr = current_node.clone();

    // Initialize the local in-memory key–value store.
    let state = web::Data::new(AppState {
        store: Mutex::new(HashMap::new()),
    });
    // Set the global APP_STATE pointer.
    match APP_STATE.set(state.clone()) {
        Ok(x) => x,
        Err(_) => panic!("Failed to set APP_STATE"),
    };

    // Open and load local cold storage from disk.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("storage.db")?;
    match load_db(&mut file, &state) {
        Ok(_) => println!("Cold storage loaded"),
        Err(e) => eprintln!("Error loading DB: {}", e),
    }

    // If a join node is provided, join its cluster and pull its state.
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

        // Pull the remote state through the /store endpoint.
        let client = reqwest::Client::new();
        let store_url = format!("http://{}/store", join_node);
        match client.get(&store_url).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    // The remote data is a mapping from partition keys
                    // to JSON maps.
                    let remote_store: HashMap<String, HashMap<String, Value>> =
                        match resp.json().await {
                            Ok(store) => store,
                            Err(e) => {
                                eprintln!("Failed to parse cold storage: {}", e);
                                HashMap::new()
                            }
                        };
                    // Merge the remote store into the current local store.
                    {
                        let mut local_store = state.store.lock().unwrap();
                        merge_state(&mut local_store, remote_store);
                        println!(
                            "Merged cold storage from {}: {:?}",
                            join_node, *local_store
                        );
                    }
                } else {
                    eprintln!(
                        "Failed to get cold storage from {}: {:?}",
                        join_node,
                        resp.status()
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "Error fetching cold storage from {}: {}",
                    join_node, e,
                );
            }
        }
    }

    // Spawn the periodic cold save task.
    tokio::spawn({
        let state = state.clone();
        async move {
            cold_save(state, 60).await;
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

    // Build and run the HTTP server with the /store endpoint.
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(cluster_data.clone())
            .app_data(web::Data::new(current_node.clone()))
            .route("/join", web::post().to(join_cluster))
            .route(
                "/membership",
                web::get().to(network::broadcaster::get_membership),
            )
            .route(
                "/update_membership",
                web::post().to(network::broadcaster::update_membership),
            )
            .route("/key/{key}", web::get().to(get_value))
            .route("/key/{key}", web::put().to(put_value))
            .route("/key/{key}", web::delete().to(delete_value))
            .route("/store", web::get().to(get_store))
    })
        .bind(bind_addr.as_str())?
        .run()
        .await
}
