use std::collections::HashMap;
use std::fs::{create_dir_all, File, read_dir};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process;
use std::time::Duration;
use actix_web::web;
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::task;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rand_core::OsRng;
use rand_core::TryRngCore;

use crate::storage::engine::{AppState, VersionedValue};

const KEY: &[u8; 32] = include_bytes!(
    concat!(env!("CARGO_MANIFEST_DIR"), "/encryption.key")
);
pub fn encrypt(plain: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Initialize cipher with a 256‐bit key
    let cipher = Aes256Gcm::new(KEY.into());

    // 96‐bit nonce
    let mut nonce = [0u8; 12];
    let _ = OsRng.try_fill_bytes(&mut nonce);

    // Encrypt + authenticate - map the error manually
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Encryption error: {:?}", e),
            ))
        })?;

    // Prefix nonce for storage
    let mut out = nonce.to_vec();
    out.extend(ciphertext);

    // Base64‐encode for safe transport/storage
    Ok(general_purpose::STANDARD.encode(&out))
}

pub fn decrypt(encrypted: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Base64-decode
    let data = general_purpose::STANDARD
        .decode(encrypted)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Base64 decode error: {}", e),
            ))
        })?;

    // Ensure we have at least enough bytes for the nonce
    if data.len() < 12 {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Encrypted data too short",
        )));
    }

    // Split out the 96‐bit nonce
    let (nonce, ciphertext) = data.split_at(12);
    let cipher = Aes256Gcm::new(KEY.into());

    // Decrypt + verify - map the error manually
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Decryption error: {:?}", e),
            ))
        })?;

    // Convert bytes to UTF-8 string
    String::from_utf8(plaintext).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("UTF-8 conversion error: {}", e),
        ))
    })
}


#[derive(Serialize, Deserialize, Debug)]
pub struct WalEntry {
    op: String, // "put" or "delete"
    key: String,
    value: Option<VersionedValue>,
    timestamp: u128,
}

/// Appends a Write-Ahead Log (WAL) entry to the WAL file for the specified table.
// pub async fn append_to_wal(state: &AppState, table: &str, entry: &WalEntry) -> io::Result<()> {
//     let folder_path = Path::new(state.base_dir).join(table);
//     create_dir_all(&folder_path)?;
//     let wal_path = folder_path.join("wal.log");
//     let mut file = OpenOptions::new().create(true).append(true).open(&wal_path)?;
//     let line = serde_json::to_string(entry)?;
//     writeln!(file, "{}", line)?;
//     Ok(())
// }

/// Saves the current in‑memory state (snapshot) for each table to disk.
/// Also clears the WAL after snapshot.
pub async fn save_to_cold(state: web::Data<AppState>) -> io::Result<()> {
    // Clone the store so that we can release the lock.
    let store_snapshot = {
        let store = state.store.read().await;
        store.clone()
    };
    let state_clone = state.clone();
    task::spawn_blocking(move || {
        for (table_name, table_data) in store_snapshot.into_iter() {
            let folder_path = Path::new(state_clone.base_dir).join(&table_name);
            create_dir_all(&folder_path)?;
            let file_path = folder_path.join("storage.db");
            let mut file = File::create(&file_path)?;

            // build your plaintext
            let mut plain_data = String::new();
            for (key, versioned_value) in table_data.iter() {
                plain_data.push_str(&format!(
                    "{} = {}\n",
                    key,
                    serde_json::to_string(versioned_value)?
                ));
            }

            // ✂️ unwrap the Result<String,_> here and map to io::Error
            let encrypted_data = encrypt(&plain_data)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            // write the actual String, not a Result<_,_>
            file.write_all(encrypted_data.as_bytes())?;

            // Clear the WAL
            let wal_path = folder_path.join("wal.log");
            File::create(&wal_path)?;
        }
        Ok::<(), io::Error>(())
    })
        .await??;

    Ok(())
}

/// Loads all tables from disk by replaying the snapshot and WAL.
pub async fn load_all_tables(state: &web::Data<AppState>) -> io::Result<()> {
    let state_cloned = state.clone(); // clone to ensure 'static lifetime in blocking task
    let new_store = task::spawn_blocking(move || {
        let base_path = Path::new(state_cloned.base_dir);
        let mut store: HashMap<String, HashMap<String, VersionedValue>> = HashMap::new();
        if base_path.exists() && base_path.is_dir() {
            for entry in read_dir(base_path)? {
                let entry = entry?;
                let table_folder = entry.path();
                if table_folder.is_dir() {
                    let table_name = match table_folder.file_name() {
                        Some(name) => name.to_string_lossy().to_string(),
                        None => continue,
                    };

                    let mut table_data: HashMap<String, VersionedValue> = HashMap::new();
                    let snapshot_path = table_folder.join("storage.db");
                    if snapshot_path.exists() {
                        let mut file = File::open(&snapshot_path)?;
                        let mut encrypted_content = String::new();
                        file.read_to_string(&mut encrypted_content)?;
                        match decrypt(&encrypted_content) {
                            Ok(decrypted_content) => {
                                for line in decrypted_content.lines() {
                                    if let Some((key, json_str)) = line.split_once('=') {
                                        let key = key.trim().to_string();
                                        let json_str = json_str.trim();
                                        if let Ok(value) =
                                            serde_json::from_str::<VersionedValue>(json_str)
                                        {
                                            table_data.insert(key, value);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "Decryption failed for table {}: {}",
                                    table_name, e
                                );
                                process::exit(1);
                            }
                        }
                    }

                    // Replay WAL entries.
                    let wal_path = table_folder.join("wal.log");
                    if wal_path.exists() {
                        let file = File::open(&wal_path)?;
                        let reader = BufReader::new(file);
                        for line in reader.lines() {
                            let line = line?;
                            if let Ok(entry) =
                                serde_json::from_str::<WalEntry>(&line)
                            {
                                match entry.op.as_str() {
                                    "put" => {
                                        if let Some(val) = entry.value {
                                            table_data.insert(entry.key, val);
                                        }
                                    }
                                    "delete" => {
                                        table_data.remove(&entry.key);
                                    }
                                    _ => {
                                        eprintln!(
                                            "Unknown WAL op in table {}: {}",
                                            table_name, entry.op
                                        );
                                    }
                                }
                            } else {
                                eprintln!(
                                    "Failed to parse WAL entry in table {}",
                                    table_name
                                );
                            }
                        }
                    }
                    store.insert(table_name, table_data);
                }
            }
        }
        Ok::<HashMap<String, HashMap<String, VersionedValue>>, io::Error>(store)
    })
        .await??;
    let mut store_write = state.store.write().await;
    *store_write = new_store;
    Ok(())
}

/// Spawns a background Tokio task that periodically saves the current state to disk.
pub async fn cold_save(state: web::Data<AppState>, interval: usize) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(interval as u64)).await;
            if let Err(e) = save_to_cold(state.clone()).await {
                eprintln!("Error saving cold storage: {}", e);
            }
        }
    });
}
