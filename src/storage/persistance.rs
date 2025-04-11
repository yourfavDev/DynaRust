use std::collections::HashMap;
use std::fs::File;
use std::io::{
    self, BufRead, BufReader, Read, Seek, SeekFrom, Write,
};
use std::time::Duration;
use actix_web::web;
use tokio::time::sleep;
use serde_json::Value;
use base64::{engine::general_purpose, Engine as _}; // Updated import
use crate::storage::engine::AppState;

// A very simple constant key for XOR encryption.
const ENCRYPTION_KEY: u8 = 0xAA;

/// Encrypts the input string by XORing each byte with a fixed key,
/// then encoding the result in Base64.
fn encrypt(plain: &str) -> String {
    let encrypted: Vec<u8> = plain.bytes().map(|b| b ^ ENCRYPTION_KEY).collect();
    // Use the engine-based API.
    general_purpose::STANDARD.encode(&encrypted)
}

/// Decrypts the input string by first Base64-decoding it and then XORing
/// each byte with the fixed key. Returns the decrypted string.
fn decrypt(encrypted: &str) -> Result<String, Box<dyn std::error::Error>> {
    let decoded = general_purpose::STANDARD.decode(encrypted)?; // Updated API.
    let decrypted: Vec<u8> = decoded.into_iter().map(|b| b ^ ENCRYPTION_KEY).collect();
    Ok(String::from_utf8(decrypted)?)
}

/// Saves the current in-memory state to a "cold" storage file after encrypting
/// its content. The file format (after decryption) is as follows:
///
/// partition_key=[
///   attribute = value
///   attribute2 = value2
/// ]
fn save_to_cold(state: &web::Data<AppState>) -> io::Result<()> {
    // Acquire a lock on the store.
    let store = state.store.lock().unwrap();

    // Build the plain text representation of the store.
    let mut plain_data = String::new();
    for (partition_key, attributes) in store.iter() {
        plain_data.push_str(&format!("{}=[\n", partition_key));
        for (attribute, value) in attributes.iter() {
            plain_data.push_str(&format!("  {} = {}\n", attribute, value));
        }
        plain_data.push_str("]\n");
    }


    // Encrypt the whole string.
    let encrypted_data = encrypt(&plain_data);

    // Write the encrypted data to the storage file.
    let mut file = File::create("storage.db")?;
    write!(file, "{}", encrypted_data)?;
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
/// The file is first decrypted before the content is parsed.
pub fn load_db(file: &mut File, state: &web::Data<AppState>) -> io::Result<()> {
    // Ensure we start reading from the beginning of the file.
    file.seek(SeekFrom::Start(0))?;
    let mut encrypted_content = String::new();
    file.read_to_string(&mut encrypted_content)?;

    // Decrypt the file content.
    let decrypted_content = match decrypt(&encrypted_content) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Decryption failed: {}", e);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Decryption failed",
            ));
        }
    };

    println!("Decrypted data:\n{}", decrypted_content);

    let reader = BufReader::new(decrypted_content.as_bytes());

    // Create a new store which we'll swap into the state once loading is complete.
    let mut new_store: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut current_partition: Option<String> = None;
    let mut current_attributes: HashMap<String, Value> = HashMap::new();

    for line_result in reader.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Detect a partition header line (ends with "=[")
        if trimmed.ends_with("=[") {
            // Save the previous partition (if any) into the store.
            if let Some(part_key) = current_partition.take() {
                new_store.insert(part_key, current_attributes);
                current_attributes = HashMap::new();
            }
            // Extract the partition key.
            let partition_key = trimmed.trim_end_matches("=[").trim().to_string();
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
            // Expect an attribute line: "attribute = value"
            if let Some((attribute, raw_value)) = trimmed.split_once('=') {
                let attribute = attribute.trim().to_string();
                let raw_value = raw_value.trim();
                let parsed_value =
                    serde_json::from_str::<Value>(raw_value)
                        .unwrap_or_else(|_| Value::String(raw_value.to_string()));
                current_attributes.insert(attribute, parsed_value);
            } else {
                eprintln!("Invalid attribute line: '{}'", line);
            }
        }
    }

    // In case the file ended without a final closing bracket.
    if let Some(part_key) = current_partition {
        new_store.insert(part_key, current_attributes);
    }

    // Replace the entire in-memory store with the newly loaded store.
    *state.store.lock().unwrap() = new_store;
    Ok(())
}
