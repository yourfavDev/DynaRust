use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::time::Duration;
use actix_web::web;
use tokio::time::sleep;
use serde_json::Value; // Make sure to include serde_json in your Cargo.toml
use crate::storage::engine::AppState;

/// Saves the current in-memory state to a "cold" storage file.
/// The file format will be:
///
/// partition_key=[
///   attribute = value
///   attribute2 = value2
/// ]
fn save_to_cold(state: &web::Data<AppState>) -> io::Result<()> {
    // Acquire a lock on the store.
    let store = state.store.lock().unwrap();
    // Overwrite (or create) the cold storage file.
    let mut file = File::create("storage.db")?;
    let mut log = String::new();

    for (partition_key, attributes) in store.iter() {
        log += &format!("{}=[\n", partition_key);
        writeln!(file, "{}=[", partition_key)?;
        for (attribute, value) in attributes.iter() {
            // Here we assume that each Value is simply a string
            // You might need to adjust how you display or format the Value.
            log += &format!("  {} = {}\n", attribute, value);
            writeln!(file, "  {} = {}", attribute, value)?;
        }
        log += "]\n";
        writeln!(file, "]")?;
    }

    println!("{}", log);
    Ok(())
}

/// Spawns a background Tokio task which saves the current state to disk
/// every `interval` seconds.
pub async fn cold_save(state: web::Data<AppState>, interval: usize) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(interval as u64)).await;
            if let Err(e) = save_to_cold(&state) {
                eprintln!("Error saving cold storage: {}", e);
            }
        }
    });
}

/// Loads the database from the "cold" storage file into the in-memory store.
/// It expects the file format written by `save_to_cold`, e.g.:
///
/// partition_key=[
///   attribute = value
///   attribute2 = value2
/// ]
///
/// Lines that don't match the expected format are skipped (with a warning).
pub fn load_db(file: &mut File, state: &web::Data<AppState>) -> io::Result<()> {
    // Ensure we start reading from the beginning of the file.
    file.seek(SeekFrom::Start(0))?;
    let reader = BufReader::new(file);

    // Create a new store with the expected value type.
    let mut new_store: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut current_partition: Option<String> = None;
    let mut current_attributes: HashMap<String, Value> = HashMap::new();

    for line_result in reader.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Detect a partition header line: it should end with "=["
        if trimmed.ends_with("=[") {
            // Save the previous partition (if any) into the store.
            if let Some(part_key) = current_partition.take() {
                new_store.insert(part_key, current_attributes);
                current_attributes = HashMap::new();
            }
            // Extract the partition key by stripping the trailing "=["
            let partition_key =
                trimmed.trim_end_matches("=[").trim().to_string();
            current_partition = Some(partition_key);
        } else if trimmed == "]" {
            // End of the current partition block.
            if let Some(part_key) = current_partition.take() {
                new_store.insert(part_key, current_attributes);
                current_attributes = HashMap::new();
            } else {
                eprintln!(
                    "Unexpected closing bracket ']' without a matching partition header."
                );
            }
        } else {
            // Expect an attribute line in the format "attribute = value"
            if let Some((attribute, raw_value)) = trimmed.split_once('=') {
                let attribute = attribute.trim().to_string();
                let raw_value = raw_value.trim();
                // Try to parse the raw value as JSON. This handles values such as:
                //   "es"  -> Value::String("es")
                //   true  -> Value::Bool(true)
                let parsed_value = serde_json::from_str::<Value>(raw_value)
                    .unwrap_or_else(|_| Value::String(raw_value.to_string()));
                current_attributes.insert(attribute, parsed_value);
            } else {
                eprintln!("Invalid attribute line: '{}'", line);
            }
        }
    }

    // In case the file ended without a final closing bracket, save any remaining open partition.
    if let Some(part_key) = current_partition {
        new_store.insert(part_key, current_attributes);
    }

    // Replace the entire in‑memory store with the newly loaded store.
    *state.store.lock().unwrap() = new_store;
    Ok(())
}