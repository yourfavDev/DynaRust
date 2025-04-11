mod storage;
mod network;


use network::broadcaster::membership_sync;
use actix_web::{web, App, HttpServer};
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::process;
use std::sync::Mutex;
use storage::engine::{
    AppState, ClusterData, delete_value, get_value, join_cluster,
    put_value,
};
use storage::persistance::{cold_save, load_db};

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

    // Start with our own node as the initial membership.
    let initial_nodes = vec![current_node.clone()];
    // If a join node is provided, try joining the cluster.
    if args.len() >= 3 {
        let join_node = args[2].clone();
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
    }

    // Initialize the local in-memory key–value store.
    let state = web::Data::new(AppState {
        store: Mutex::new(HashMap::new()),
    });

    // Open a file for cold storage and try to load it.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("storage.db")?;
    match load_db(&mut file, &state) {
        Ok(_) => println!("Cold storage loaded"),
        Err(e) => eprintln!("Error loading DB: {}", e),
    }
    // Spawn the periodic cold save task.
    tokio::spawn({
        let state = state.clone();
        async move {
            cold_save(state, 6).await;
        }
    });

    // Initialize cluster data with dynamic membership.
    let cluster_data = web::Data::new(ClusterData {
        nodes: std::sync::Arc::new(Mutex::new(initial_nodes))
    });

    // Spawn the membership synchronization background task.
    let cluster_data_clone = cluster_data.clone();
    let current_node_clone = current_node.clone();
    tokio::spawn(membership_sync(
        cluster_data_clone,
        current_node_clone,
        60,
    ));

    println!("Starting distributed DB engine at http://{}", current_node);

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(cluster_data.clone())
            .app_data(web::Data::new(current_node.clone()))
            // Endpoints for joining and synchronizing membership.
            .route("/join", web::post().to(join_cluster))
            .route("/membership", web::get().to(network::broadcaster::get_membership))
            .route("/update_membership", web::post().to(network::broadcaster::update_membership))
            .route("/key/{key}", web::get().to(get_value))
            .route("/key/{key}", web::put().to(put_value))
            .route("/key/{key}", web::delete().to(delete_value))
    })
        .bind(bind_addr.as_str())?
        .run()
        .await
}
