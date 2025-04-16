// storage/subscription.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// An event describing a key update (or deletion).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyEvent {
    Updated(VersionedValue),
    Deleted,
}

/// Manages broadcast channels for keys so that multiple subscribers can listen.
#[derive(Clone)]
pub struct SubscriptionManager {
    // The key is a combination of table name and key (e.g. "default/mykey")
    pub channels: Arc<RwLock<HashMap<String, broadcast::Sender<KeyEvent>>>>,
}

impl SubscriptionManager {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Subscribe to updates for a given table/key.
    pub async fn subscribe(&self, table: &str, key: &str) -> broadcast::Receiver<KeyEvent> {
        let identifier = format!("{}/{}", table, key);
        let mut channels = self.channels.write().await;
        if let Some(sender) = channels.get(&identifier) {
            sender.subscribe()
        } else {
            // Create a new channel if none exists; here we use a buffer of 16 events.
            let (tx, rx) = broadcast::channel(16);
            channels.insert(identifier, tx);
            rx
        }
    }

    /// Send a notification for the given table/key.
    pub async fn notify(&self, table: &str, key: &str, event: KeyEvent) {
        let identifier = format!("{}/{}", table, key);
        let channels = self.channels.read().await;
        if let Some(sender) = channels.get(&identifier) {
            // Ignore send error if there are no subscribers
            let _ = sender.send(event);
        }
    }
}
// storage/subscription.rs (continued)

use actix_web::{web, HttpResponse, Responder};
use futures::stream::StreamExt;
use crate::storage::engine::VersionedValue;

/// Subscribe to updates on a key. The URL contains the table and key.
pub async fn subscribe_to_key(
    path: web::Path<(String, String)>,
    sub_manager: web::Data<SubscriptionManager>,
) -> impl Responder {
    let (table, key) = path.into_inner();

    // Get a broadcast receiver for this key.
    let mut rx = sub_manager.subscribe(&table, &key).await;

    // Build an SSE stream. Each event is yielded as a chunk in the HTTP response.
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Serialize the event as JSON.
                    let data = serde_json::to_string(&event)
                        .unwrap_or_else(|_| "null".to_string());
                    // SSE events MUST be formatted as "data: <payload>\n\n".
                    yield Ok::<_, actix_web::Error>(
                        web::Bytes::from(format!("data: {}\n\n", data))
                    );
                }
                // If messages are dropped, simply continue.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                // Exit the loop when the sender is dropped.
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    HttpResponse::Ok()
        .append_header(("Content-Type", "text/event-stream"))
        .streaming(stream)
}
