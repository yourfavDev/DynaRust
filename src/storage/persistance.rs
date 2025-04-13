use std::collections::HashMap;
use std::fs::{create_dir_all, File, read_dir};
use std::io::{
    self, BufRead, BufReader, Read, Seek, SeekFrom, Write,
};
use std::path::Path;
use std::time::Duration;

use actix_web::web;
use tokio::time::sleep;
use serde_json::Value;
use base64::{engine::general_purpose, Engine as _};

use crate::storage::engine::AppState;

// A very simple constant key for XOR encryption.
const ENCRYPTION_KEY: u8 = 0xAA;

/// Encrypts the input string by XORing each byte with a fixed key,
/// then encoding the result in Base64.
fn encrypt(plain: &str) -> String {
    let encrypted: Vec<u8> = plain.bytes().map(|b| b ^ ENCRYPTION_KEY).collect();
    general_purpose::STANDARD.encode(&encrypted)
}

/// Decrypts the input string by first Base64-decoding it and then XORing
/// each byte with the fixed key. Returns the decrypted string.
fn decrypt(encrypted: &str) -> Result<String, Box<dyn std::error::Error>> {
    let decoded = general_purpose::STANDARD.decode(encrypted)?;
    let decrypted: Vec<u8> = decoded.into_iter().map(|b| b ^ ENCRYPTION_KEY).collect();
    Ok(String::from_utf8(decrypted)?)
}

/// Saves the current in-memory state on a per‑table basis.
/// For each table in the AppState, this function:
///   1. Creates (or reuses) a folder at `base_dir/<table_name>`.
///   2. Writes the table’s data into a file named "storage.db" inside that folder.
///
/// The file’s plain text (after decryption) is formatted as follows:
///
///   <partition_key>=[
///     <attribute> = <json_value>
///     <attribute2> = <json_value>
///   ]
///
/// JSON serialization is used so that on subsequent loads the values are not re‑wrapped.
fn save_to_cold(state: &web::Data<AppState>) -> io::Result<()> {
    // Lock the in-memory store.
    let store = state.store.lock().unwrap();

    // Iterate over every table.
    for (table_name, table_data) in store.iter() {
        // Build the folder path: base_dir/<table_name>
        let folder_path = Path::new(state.base_dir).join(table_name);
        create_dir_all(&folder_path)?; // Create it if it doesn't exist.

        // Construct the file path inside the table folder.
        let file_path = folder_path.join("storage.db");
        let mut file = File::create(&file_path)?;

        // Build the plain text representation.
        let mut plain_data = String::new();
        for (partition_key, attributes) in table_data.iter() {
            plain_data.push_str(&format!("{}=[\n", partition_key));
            for (attribute, value) in attributes.iter() {
                // Serialize the JSON value properly.
                let json_value = serde_json::to_string(value)
                    .unwrap_or_else(|_| "\"<serialization error>\"".to_string());
                plain_data.push_str(&format!("  {} = {}\n", attribute, json_value));
            }
            plain_data.push_str("]\n");
        }

        // Encrypt the plain representation.
        let encrypted_data = encrypt(&plain_data);
        write!(file, "{}", encrypted_data)?;
    }
    Ok(())
}

/// Spawns a background Tokio task which periodically saves the current state
/// to disk (every `interval` seconds).
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

/// Loads all the tables from disk by enumerating subdirectories in base_dir.
///
/// For each subdirectory (assumed to be a table folder) that contains a "storage.db"
/// file, the file is read, decrypted, and parsed into the table’s storage format.
/// The in‑memory store is then replaced with the newly loaded state.
pub fn load_all_tables(state: &web::Data<AppState>) -> io::Result<()> {
    let base_path = Path::new(state.base_dir);
    let mut new_store: HashMap<String, HashMap<String, HashMap<String, Value>>> =
        HashMap::new();

    if base_path.exists() && base_path.is_dir() {
        for entry in read_dir(base_path)? {
            let entry = entry?;
            let table_folder = entry.path();

            if table_folder.is_dir() {
                // The table name is the folder's name.
                let table_name = match table_folder.file_name() {
                    Some(name) => name.to_string_lossy().to_string(),
                    None => continue,
                };

                let file_path = table_folder.join("storage.db");
                if file_path.exists() {
                    let mut file = File::open(&file_path)?;
                    // Ensure reading from the beginning.
                    file.seek(SeekFrom::Start(0))?;
                    let mut encrypted_content = String::new();
                    file.read_to_string(&mut encrypted_content)?;

                    let decrypted_content = match decrypt(&encrypted_content) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("Decryption failed for table {}: {}", table_name, e);
                            continue;
                        }
                    };

                    println!("Decrypted data for table {}:\n{}", table_name, decrypted_content);

                    let reader = BufReader::new(decrypted_content.as_bytes());
                    let mut table_data: HashMap<String, HashMap<String, Value>> = HashMap::new();
                    let mut current_partition: Option<String> = None;
                    let mut current_attributes: HashMap<String, Value> = HashMap::new();

                    for line_result in reader.lines() {
                        let line = line_result?;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        // Lines ending with "=[" start a new partition.
                        if trimmed.ends_with("=[") {
                            if let Some(part_key) = current_partition.take() {
                                table_data.insert(part_key, current_attributes);
                                current_attributes = HashMap::new();
                            }
                            let part_key = trimmed.trim_end_matches("=[").trim().to_string();
                            current_partition = Some(part_key);
                        }
                        // A line containing only "]" finishes the current partition.
                        else if trimmed == "]" {
                            if let Some(part_key) = current_partition.take() {
                                table_data.insert(part_key, current_attributes);
                                current_attributes = HashMap::new();
                            } else {
                                eprintln!("Unexpected closing bracket in table {}", table_name);
                            }
                        }
                        // Otherwise, treat the line as an attribute line in the form "attribute = value".
                        else {
                            if let Some((attribute, raw_value)) = trimmed.split_once('=') {
                                let attribute = attribute.trim().to_string();
                                let raw_value = raw_value.trim();
                                let parsed_value = serde_json::from_str::<Value>(raw_value)
                                    .unwrap_or_else(|_| Value::String(raw_value.to_string()));
                                current_attributes.insert(attribute, parsed_value);
                            } else {
                                eprintln!("Invalid attribute line in table {}: '{}'", table_name, trimmed);
                            }
                        }
                    }

                    // Finalize any unclosed partition.
                    if let Some(part_key) = current_partition.take() {
                        table_data.insert(part_key, current_attributes);
                    }
                    new_store.insert(table_name, table_data);
                }
            }
        }
    }

    *state.store.lock().unwrap() = new_store;
    Ok(())
}
