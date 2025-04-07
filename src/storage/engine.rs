use std::collections::HashMap;
use std::sync::Mutex;
use actix_web::{ web,  HttpResponse, Responder};

pub struct AppState {
    pub(crate) store: Mutex<HashMap<String, String>>,
}

/// Handler to get the value for a given key.
/// Use a GET request to `/key/<key>`.
pub async fn get_value(
    data: web::Data<AppState>,
    key: web::Path<String>,
) -> impl Responder {
    let store = data.store.lock().unwrap();
    if let Some(value) = store.get(&key.into_inner()) {
        HttpResponse::Ok().body(value.clone())
    } else {
        HttpResponse::NotFound().body("Key not found")
    }
}

/// Handler to insert or update a key–value pair.
/// Use a PUT request to `/key/<key>` with the value in the request body.
pub async fn put_value(
    data: web::Data<AppState>,
    key: web::Path<String>,
    body: String,
) -> impl Responder {
    let mut store = data.store.lock().unwrap();
    store.insert(key.into_inner(), body);
    HttpResponse::Created().body("Key stored")
}

/// Handler to delete a key.
/// Use a DELETE request to `/key/<key>`.
pub async fn delete_value(
    data: web::Data<AppState>,
    key: web::Path<String>,
) -> impl Responder {
    let mut store = data.store.lock().unwrap();
    if store.remove(&key.into_inner()).is_some() {
        HttpResponse::Ok().body("Key deleted")
    } else {
        HttpResponse::NotFound().body("Key not found")
    }
}
