use actix_web::{web, HttpRequest, HttpResponse, Responder};
use actix_web::web::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use hex;
use chrono::{Utc, Duration};
use std::collections::HashMap;
use std::env;
use jsonwebtoken::{encode, Header, EncodingKey};

use crate::storage::engine::{get_active_nodes, get_replication_nodes, AppState, ClusterData, VersionedValue};
use crate::storage::subscription::{KeyEvent, SubscriptionManager};

/// Incoming JSON payload.
#[derive(Debug, Deserialize)]
pub struct AccessToken {
    pub secret: String,
}

/// Claims for JWT.
#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

// Hard‐coded for demo; in prod load from ENV or config.
const JWT_SECRET: &[u8] = b"kajdOsndmalskfi";

pub async fn access(
    req: HttpRequest,
    user: web::Path<String>,
    secret_req: Json<AccessToken>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
) -> impl Responder {
    let username = user.into_inner();
    let provided_secret = secret_req.into_inner().secret;

    println!("Access request for user: {}", username);

    // Internal replication requests bypass authentication
    if req.headers().contains_key("X-Internal-Request") {
        println!("Processing internal replication request for {}", username);

        // Simplified internal replication handling
        let mut store = state.store.write().await;
        let auth_table: &mut HashMap<String, VersionedValue> =
            store.entry("auth".to_string()).or_insert_with(HashMap::new);

        // For internal requests, use the provided hash directly
        let mut value_map = HashMap::new();
        value_map.insert("secret".to_string(), json!(provided_secret));

        let user_header = req.headers().get("User")
            .and_then(|h| h.to_str().ok())
            .unwrap_or(&username);

        let new_rec = VersionedValue::new(value_map, String::from(user_header));
        auth_table.insert(username.clone(), new_rec.clone());

        println!("Successfully replicated user: {}", username);

        sub_manager
            .notify(&"auth".to_string(), &username, KeyEvent::Updated(new_rec.clone()))
            .await;

        return HttpResponse::Created().json(json!({
            "status": "User replicated"
        }));
    }

    // Regular authentication flow
    let is_new_user;
    let new_value = {
        let mut store = state.store.write().await;
        let auth_table: &mut HashMap<String, VersionedValue> =
            store.entry("auth".to_string()).or_insert_with(HashMap::new);

        let record = auth_table.get(&username);
        if record.is_none() {
            println!("New user registration: {}", username);
            // User registration
            let mut hasher = Sha256::new();
            hasher.update(&provided_secret);
            let hashed = hex::encode(hasher.finalize());

            let mut value_map = HashMap::new();
            value_map.insert("secret".to_string(), json!(hashed));

            let new_rec = VersionedValue::new(value_map, username.clone());
            auth_table.insert(username.clone(), new_rec.clone());
            is_new_user = true;
            new_rec.clone()
        } else {
            println!("Login attempt for existing user: {}", username);
            // Login attempt
            let user_record = record.unwrap();

            let stored_hash = match user_record
                .value
                .get("secret")
                .and_then(|v| v.as_str())
            {
                Some(h) => h,
                None => {
                    return HttpResponse::InternalServerError()
                        .json(json!({"error":"Corrupt user record"}));
                }
            };

            let mut hasher = Sha256::new();
            hasher.update(&provided_secret);
            let provided_hash = hex::encode(hasher.finalize());

            if stored_hash != provided_hash {
                return HttpResponse::Unauthorized()
                    .json(json!({"error":"Invalid credentials"}));
            }

            is_new_user = false;
            user_record.clone()
        }
    };

    // Notify subscribers about the update
    if is_new_user {
        sub_manager
            .notify(&"auth".to_string(), &username, KeyEvent::Updated(new_value.clone()))
            .await;
    }

    // Replicate new user to other nodes
    if is_new_user {
        println!("Starting replication for new user: {}", username);
        let cluster_guard = cluster_data.nodes.read().await;
        let active_nodes = get_active_nodes(&*cluster_guard);
        drop(cluster_guard);

        let replication_factor = active_nodes.len();
        let targets = get_replication_nodes(&username, &active_nodes, replication_factor);
        println!("Replication targets: {:?}", targets);

        let client = reqwest::Client::new();
        let mut replication_futures = Vec::new();

        // Get the hashed secret for replication
        let hashed_secret = new_value.value.get("secret").unwrap().as_str().unwrap();

        for target in targets {
            if target != *current_addr.get_ref() {
                // Make sure URL format matches your API route configuration
                let url = format!("http://{}/auth/{}", target, username);
                println!("Preparing replication to: {}", url);

                let client_clone = client.clone();
                let username_clone = username.clone();
                let secret_clone = hashed_secret.to_string();

                let fut = async move {
                    println!("Sending replication request to: {}", url);
                    // Create the payload with the same structure as AccessToken
                    let payload = json!({"secret": secret_clone});

                    let result = client_clone
                        .post(&url)
                        .header("X-Internal-Request", "true")
                        .header("User", &username_clone)
                        .json(&payload)
                        .send()
                        .await;

                    match result {
                        Ok(response) => {
                            println!("Replication response from {}: status={}",
                                     url, response.status());
                        },
                        Err(e) => {
                            println!("Replication error to {}: {}", url, e);
                        }
                    }
                };
                replication_futures.push(fut);
            }
        }

        // Use a separate clone for the tokio::spawn task
        let username_for_spawn = username.clone();
        tokio::spawn(async move {
            futures_util::future::join_all(replication_futures).await;
            println!("Replication completed for user: {}", username_for_spawn);
        });
    }

    // Generate JWT token for the authenticated user
    let exp = (Utc::now() + Duration::hours(1)).timestamp() as usize;
    let claims = Claims { sub: username.clone(), exp };

    let token = match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET),
    ) {
        Ok(t) => t,
        Err(e) => {
            log::error!("JWT encode error: {:?}", e);
            return HttpResponse::InternalServerError()
                .json(json!({"error":"Token creation failed"}));
        }
    };

    if is_new_user {
        println!("New user created and token generated for: {}", username);
        HttpResponse::Created().json(json!({ "token": token, "status": "User created" }))
    } else {
        println!("Token generated for existing user: {}", username);
        HttpResponse::Ok().json(json!({ "token": token }))
    }
}
