use actix_web::{web, HttpResponse, Responder};
use actix_web::web::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use hex;
use chrono::{Utc, Duration};
use std::collections::HashMap;
use jsonwebtoken::{encode, Header, EncodingKey};

use crate::storage::engine::{AppState, VersionedValue};

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
    user: web::Path<String>,
    secret_req: Json<AccessToken>,
    state: web::Data<AppState>,
) -> impl Responder {
    let username = user.into_inner();
    let provided_secret = secret_req.into_inner().secret;

    // Acquire a write lock so we can create "auth" if missing.
    let mut store = state.store.write().await;

    // Lazily create the auth table if this is the first call.
    let auth_table: &mut HashMap<String, VersionedValue> =
        store.entry("auth".to_string()).or_insert_with(HashMap::new);

    // Look up the user record.
    let record = auth_table.get(&username);
    if record.is_none() {
        // User not found ⇒ registration step.
        // Hash the provided secret.
        let mut hasher = Sha256::new();
        hasher.update(&provided_secret);
        let hashed = hex::encode(hasher.finalize());

        // Build a small map { "secret": "<hex>" } and store it.
        let mut value_map = HashMap::new();
        value_map.insert("secret".to_string(), json!(hashed));

        let new_rec = VersionedValue::new(value_map, username.clone());
        auth_table.insert(username.clone(), new_rec.clone());

        return HttpResponse::Ok().json(json!({
            "status": "User created"
        }));
    }

    // Otherwise this is a login attempt.
    let user_record = record.unwrap();

    // Extract stored hash.
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

    // Hash the provided secret.
    let mut hasher = Sha256::new();
    hasher.update(&provided_secret);
    let provided_hash = hex::encode(hasher.finalize());

    if stored_hash != provided_hash {
        return HttpResponse::Unauthorized()
            .json(json!({"error":"Invalid credentials"}));
    }

    // Generate a JWT token, 1h expiry.
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

    HttpResponse::Ok().json(json!({ "token": token }))
}
