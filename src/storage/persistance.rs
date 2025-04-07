use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::time::Duration;
use actix_web::web;
use tokio::time::sleep;
use crate::storage::engine::AppState;
use std::io::Write;
use crate::{pretty_log};

/// Saves the current in-memory state to a "cold" storage file.
/// Each key-value pair is written as "key=value" on a new line.
fn save_to_cold(state: &web::Data<AppState>) -> io::Result<()> {
    let store = state.store.lock().unwrap();
    // Overwrite (or create) the cold storage file.
    let mut file = File::create("storage.db")?;
    for (key, value) in store.iter() {
        writeln!(file, "{}={}", key, value)?;
    }
    Ok(())
}

/// Spawns a background Tokio task which saves the current state to disk
/// every `interval` seconds.
pub async fn cold_save(state: web::Data<AppState>, interval: usize) {
    pretty_log!("ENGINE", "WORKER"; "Sync to cold storage every {} seconds", interval);
    // Spawn a background task that lives as long as the application.
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(interval as u64)).await;
            if let Err(e) = save_to_cold(&state) {
                eprintln!("Error saving cold storage: {}", e);
            }
        }
    });
}

/// Loads the database from a "cold" storage file into the in-memory store.
pub fn load_db(file: &mut File, state: &web::Data<AppState>) -> io::Result<()> {
    // Ensure we start reading from the beginning of the file.
    file.seek(SeekFrom::Start(0))?;
    let reader = BufReader::new(file);

    for line_result in reader.lines() {
        let line = line_result?;

        // Skip empty or whitespace-only lines.
        if line.trim().is_empty() {
            continue;
        }

        // Expect the format "key=value" on each line.
        if let Some((key, value)) = line.split_once('=') {
            state
                .store
                .lock()
                .unwrap()
                .insert(key.trim().to_string(), value.trim().to_string());
        } else {
            eprintln!("Invalid line format in storage.db: {}", line);
        }
    }

    Ok(())
}
