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
#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, web};
    use serde_json::json;
    use std::collections::HashMap;

    // Note: VersionedValue is imported from our engine module.
    // For testing purposes we assume its equality (PartialEq/Eq) works as intended.
    // If needed, add a manual impl of PartialEq/Eq in tests, but here we assume it’s available.

    #[actix_web::test]
    async fn test_subscription_manager_new() {
        let sm = SubscriptionManager::new();
        let channels = sm.channels.read().await;
        assert!(channels.is_empty(), "Expected no channels on a new manager");
    }

    #[actix_web::test]
    async fn test_subscribe_and_notify_updated_event() {
        let sm = SubscriptionManager::new();
        let table = "mytable";
        let key = "mykey";
        // Subscribe to the key; this returns a broadcast receiver.
        let mut receiver = sm.subscribe(table, key).await;
        // Create a dummy VersionedValue.
        let vv = crate::storage::engine::VersionedValue::new(HashMap::new());
        let event = KeyEvent::Updated(vv.clone());
        // Notify subscribers.
        sm.notify(table, key, event.clone()).await;
        // The receiver should get the event.
        let received = receiver.recv().await.expect("Should receive an event");
        match received {
            KeyEvent::Updated(ref v) => assert_eq!(v, &vv),
            _ => panic!("Expected an Updated event"),
        }
    }

    #[actix_web::test]
    async fn test_multiple_subscribers_receive_same_event() {
        let sm = SubscriptionManager::new();
        let table = "testTable";
        let key = "testKey";
        // Create two subscribers for the same key.
        let mut receiver1 = sm.subscribe(table, key).await;
        let mut receiver2 = sm.subscribe(table, key).await;
        // Create an event containing a non-empty VersionedValue.
        let vv = crate::storage::engine::VersionedValue::new(
            HashMap::from([("foo".to_string(), json!("bar"))])
        );
        let event = KeyEvent::Updated(vv.clone());
        // Notify subscribers.
        sm.notify(table, key, event.clone()).await;
        // Both receivers should receive the event.
        let r1 = receiver1.recv().await.expect("Receiver1 should get event");
        let r2 = receiver2.recv().await.expect("Receiver2 should get event");
        match r1 {
            KeyEvent::Updated(ref v) => assert_eq!(v, &vv),
            _ => panic!("Expected an Updated event in receiver1"),
        }
        match r2 {
            KeyEvent::Updated(ref v) => assert_eq!(v, &vv),
            _ => panic!("Expected an Updated event in receiver2"),
        }
    }

    #[actix_web::test]
    async fn test_notify_without_subscribers_does_not_panic() {
        let sm = SubscriptionManager::new();
        // Use a key that has not been subscribed to.
        let table = "nonexistent";
        let key = "nonexistent";
        let event = KeyEvent::Deleted;
        // Calling notify should complete without panicking.
        sm.notify(table, key, event).await;
    }

    #[actix_web::test]
    async fn test_subscribe_to_key_endpoint_header() {
        // Create a SubscriptionManager instance and wrap it in actix_web::Data.
        let sm = SubscriptionManager::new();
        let data = web::Data::new(sm);
        // Use an arbitrary table and key.
        let path = web::Path::from(("mytable".to_string(), "mykey".to_string()));
        let response = subscribe_to_key(path, data).await;
        // Convert the returned responder into an HttpResponse.
        let dummy_req = test::TestRequest::default().to_http_request();
        let http_resp = response.respond_to(&dummy_req).map_into_boxed_body();
        // Optionally, wrap into a ServiceResponse if needed by test::read_body_json.
        // Here, we only check the header.
        assert_eq!(
            http_resp.headers().get("Content-Type").unwrap(),
            "text/event-stream"
        );
    }
}
