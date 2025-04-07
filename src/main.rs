mod storage;
mod logger;

use actix_web::{web, App, HttpServer};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::sync::Mutex;
use crate::storage::persistance::cold_save;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize shared state with an empty HashMap.
    let state = web::Data::new(storage::engine::AppState {
        store: Mutex::new(HashMap::new()),
    });
    let mut file = OpenOptions::new()
        .write(true)
        // If the file does not exist, create it.
        .create(true)
        .read(true)
        // Optionally, use `append(true)` if you want to add content without truncating.
        .open("storage.db")?;
    match storage::persistance::load_db(&mut file, &state) {
        Ok(_) => engine_log!("Cold storage loaded"),
        Err(e) => println!("Error loading DB: {}", e),
    }
    cold_save(state.clone(), 10).await;
    log!("Starting single-instance DB engine at http://127.0.0.1:8080");
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .route("/key/{key}", web::get().to(storage::engine::get_value))
            .route("/key/{key}", web::put().to(storage::engine::put_value))
            .route("/key/{key}", web::delete().to(storage::engine::delete_value))
    })
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
