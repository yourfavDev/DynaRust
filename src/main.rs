mod storage;
mod logger;

use actix_web::{web, App, HttpServer};
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::process;
use std::sync::Mutex;
use crate::storage::persistance::{cold_save, load_db};
use crate::storage::engine::{AppState, ClusterData}; // Import both from the same module

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize logging.

    // Parse command-line arguments to obtain the current node address and the cluster list.
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: {} <current_node_address> <comma_separated_cluster_addresses>",
            args[0]
        );
        process::exit(1);
    }
    let current_node = args[1].clone();
    let cluster_list: Vec<String> = args[2]
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Initialize shared state (in-memory key-value store).
    let state = web::Data::new(AppState {
        store: Mutex::new(HashMap::new()),
    });

    // Open (or create if not present) the database file.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("storage.db")?;

    // Load the existing cold storage data into memory.
    match load_db(&mut file, &state) {
        Ok(_) => println!("Cold storage loaded"),
        Err(e) => eprintln!("Error loading DB: {}", e),
    }

    // Start a background task to periodically persist state to disk.
    tokio::spawn({
        let state = state.clone();
        async move {
            cold_save(state, 10).await;
        }
    });
    println!("Cluster nodes: {:?}", cluster_list);

    println!("Starting distributed DB engine at http://{}", current_node);

    // Create shared cluster data and current address objects.
    let cluster_data = web::Data::new(ClusterData { nodes: cluster_list });
    let current_addr = web::Data::new(current_node.clone());

    // Clone the bind address so we can use it when binding the server.
    let bind_addr = current_addr.get_ref().clone();

    // Start the Actix Web HTTP server.
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(cluster_data.clone())
            .app_data(current_addr.clone())
            .route("/key/{key}", web::get().to(storage::engine::get_value))
            .route("/key/{key}", web::put().to(storage::engine::put_value))
            .route("/key/{key}", web::delete().to(storage::engine::delete_value))
    })
        .bind(bind_addr.as_str())?
        .run()
        .await
}
