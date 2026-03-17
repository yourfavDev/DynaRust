use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::storage::engine::{AppState, ClusterData, VersionedValue, get_active_nodes, get_replication_nodes};
use crate::storage::subscription::{KeyEvent, SubscriptionManager};
use jsonwebtoken::{encode, Header, EncodingKey, decode, DecodingKey, Validation};
use chrono::{Utc, Duration};

#[derive(Deserialize)]
pub struct AdminLoginRequest {
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct AdminClaims {
    sub: String,
    exp: usize,
    admin: bool,
}

pub fn get_jwt_secret() -> String {
    env::var("JWT_SECRET").unwrap_or_else(|_| "default_jwt_secret".to_string())
}

pub async fn admin_dashboard() -> impl Responder {
    let html = include_str!("admin.html");
    HttpResponse::Ok().content_type("text/html; charset=utf-8").body(html)
}

pub async fn admin_login(req: Json<AdminLoginRequest>) -> impl Responder {
    let master_password = env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin123".to_string());
    
    if req.password == master_password {
        let exp = (Utc::now() + Duration::hours(24)).timestamp() as usize;
        let claims = AdminClaims { 
            sub: "admin".to_string(), 
            exp,
            admin: true 
        };

        let token = match encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(get_jwt_secret().as_ref()),
        ) {
            Ok(t) => t,
            Err(_) => return HttpResponse::InternalServerError().finish(),
        };

        HttpResponse::Ok().json(json!({ "token": token }))
    } else {
        HttpResponse::Unauthorized().finish()
    }
}

fn validate_admin_token(req: &HttpRequest) -> bool {
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = auth_str.trim_start_matches("Bearer ").trim();
                return match decode::<AdminClaims>(
                    token,
                    &DecodingKey::from_secret(get_jwt_secret().as_ref()),
                    &Validation::default(),
                ) {
                    Ok(token_data) => token_data.claims.admin,
                    Err(_) => false,
                };
            }
        }
    }
    false
}

use actix_web::web::Json;

pub async fn admin_get_tables(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if !validate_admin_token(&req) { return HttpResponse::Unauthorized().finish(); }
    
    let store = state.store.read().await;
    let tables: Vec<String> = store.keys().cloned().collect();
    HttpResponse::Ok().json(tables)
}

pub async fn admin_get_keys(
    req: HttpRequest, 
    path: web::Path<String>, 
    state: web::Data<AppState>
) -> impl Responder {
    if !validate_admin_token(&req) { return HttpResponse::Unauthorized().finish(); }
    
    let table_name = path.into_inner();
    let store = state.store.read().await;
    if let Some(table) = store.get(&table_name) {
        let keys: Vec<String> = table.keys().cloned().collect();
        HttpResponse::Ok().json(keys)
    } else {
        HttpResponse::NotFound().finish()
    }
}

pub async fn admin_get_record(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>
) -> impl Responder {
    if !validate_admin_token(&req) { return HttpResponse::Unauthorized().finish(); }
    
    let (table_name, key) = path.into_inner();
    let store = state.store.read().await;
    if let Some(table) = store.get(&table_name) {
        if let Some(record) = table.get(&key) {
            return HttpResponse::Ok().json(record);
        }
    }
    HttpResponse::NotFound().finish()
}

pub async fn admin_put_record(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
    client: web::Data<reqwest::Client>,
    sem: web::Data<Arc<Semaphore>>,
    body: web::Json<HashMap<String, Value>>,
) -> impl Responder {
    if !validate_admin_token(&req) { return HttpResponse::Unauthorized().finish(); }
    
    let (table_name, key_val) = path.into_inner();
    
    // Admin PUT bypasses ownership check but updates version
    let new_value = {
        let mut db = state.store.write().await;
        let table = db.entry(table_name.clone()).or_default();

        if let Some(existing) = table.get_mut(&key_val) {
            existing.update(body.0.clone());
            existing.clone()
        } else {
            let v = VersionedValue::new(body.0.clone(), "admin".to_string());
            table.insert(key_val.clone(), v.clone());
            v
        }
    };

    // Notify subscriptions
    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Updated(new_value.clone()))
        .await;

    // Replicate
    replicate_admin_change(&table_name, &key_val, &new_value, &cluster_data, &current_addr, &client, &sem).await;

    HttpResponse::Ok().json(new_value)
}

pub async fn admin_delete_record(
    req: HttpRequest,
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
    cluster_data: web::Data<ClusterData>,
    current_addr: web::Data<String>,
    sub_manager: web::Data<SubscriptionManager>,
    client: web::Data<reqwest::Client>,
    sem: web::Data<Arc<Semaphore>>,
) -> impl Responder {
    if !validate_admin_token(&req) { return HttpResponse::Unauthorized().finish(); }
    
    let (table_name, key_val) = path.into_inner();
    
    let mut store = state.store.write().await;
    if let Some(table) = store.get_mut(&table_name) {
        table.remove(&key_val);
    }
    
    sub_manager
        .notify(&table_name, &key_val, KeyEvent::Deleted)
        .await;

    // Replicate deletion
    let cluster_guard = cluster_data.nodes.read().await;
    let active_nodes = get_active_nodes(&*cluster_guard);
    drop(cluster_guard);

    let replication_factor = active_nodes.len();
    let targets = get_replication_nodes(&key_val, &active_nodes, replication_factor);

    for target in targets {
        if target != *current_addr.get_ref() {
            let url = format!("http://{}/{}/key/{}", target, table_name, key_val);
            let client_clone = client.clone();
            let permit = <Arc<Semaphore> as Clone>::clone(&sem).acquire_owned().await.unwrap();

            tokio::spawn(async move {
                let _permit = permit;
                let _ = client_clone
                    .delete(&url)
                    .header("X-Internal-Request", "true")
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await;
            });
        }
    }

    HttpResponse::Ok().json(json!({"message": "Deleted by admin"}))
}

async fn replicate_admin_change(
    table_name: &str,
    key_val: &str,
    new_value: &VersionedValue,
    cluster_data: &ClusterData,
    current_addr: &String,
    client: &reqwest::Client,
    sem: &Arc<Semaphore>,
) {
    let cluster = cluster_data.nodes.read().await;
    let active = get_active_nodes(&*cluster);
    drop(cluster);

    let targets = get_replication_nodes(key_val, &active, active.len());
    let secret = env::var("CLUSTER_SECRET").unwrap_or_default();

    for target in targets {
        if target == *current_addr {
            continue;
        }
        let permit = <Arc<Semaphore> as Clone>::clone(sem).acquire_owned().await.unwrap();
        let cli = client.clone();
        let payload = new_value.clone();
        let url = format!(
            "http://{}/internal/{}/key/{}",
            target, table_name, key_val
        );
        let secret_clone = secret.clone();

        tokio::spawn(async move {
            let _permit = permit;
            let _ = cli
                .put(&url)
                .header("X-Internal-Request", "true")
                .header("SECRET", &secret_clone)
                .json(&payload)
                .send()
                .await;
        });
    }
}
