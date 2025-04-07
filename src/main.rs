mod storage;
mod logger;

use actix_web::{web, App, HttpServer};
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::process;
use std::sync::Mutex;
use crate::storage::persistance::{cold_save, load_db};
use crate::storage::engine::{AppState, ClusterData};

#[actix_web::main]
async fn main() -> std::io::Result<()> {

    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: {} <current_node_address> <comma_separated_cluster_addresses>",
            args[0]
        );
        process::exit(1);
    }
    let current_node = args[1].clone();
    // Expect a full cluster list (all nodes) separated by commas.
    let cluster_list: Vec<String> = args[2]
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // You can also set the replication factor statically or as a command-line argument.
    let replication_factor = 3; // For example, 3 replicas

    let state = web::Data::new(AppState {
        store: Mutex::new(HashMap::new()),
    });

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open("storage.db")?;
    match load_db(&mut file, &state) {
        Ok(_) => println!("Cold storage loaded"),
        Err(e) => eprintln!("Error loading DB: {}", e),
    }

    tokio::spawn({
        let state = state.clone();
        async move {
            cold_save(state, 10).await;
        }
    });

    println!("Starting distributed DB engine at http://{}", current_node);

    let cluster_data = web::Data::new(ClusterData {
        nodes: cluster_list,
        replication_factor,
    });
    let current_addr = web::Data::new(current_node.clone());
    let bind_addr = current_addr.get_ref().clone();

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(cluster_data.clone())
            .app_data(current_addr.clone())
            // Use the dynamically replicated PUT handler.
            .route("/key/{key}", web::put().to(storage::engine::put_value))
            // You can similarly update GET and DELETE endpoints.
            .route("/key/{key}", web::get().to(storage::engine::get_value))
            .route("/key/{key}", web::delete().to(storage::engine::delete_value))
    })
        .bind(bind_addr.as_str())?
        .run()
        .await
}
