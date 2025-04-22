// src/storage/statistics.rs

use actix_web::{
    dev::{ServiceRequest, ServiceResponse, Transform},
    get,
    web::Data,
    HttpResponse,
    Responder,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Instant,
};

use crate::storage::engine::{AppState, ClusterData};
use crate::storage::subscription::SubscriptionManager;

/// Holds global counters.
#[derive(Clone)]
pub struct MetricsCollector {
    pub total_requests: Arc<AtomicU64>,
    pub total_latency_ns: Arc<AtomicU64>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            total_requests: Arc::new(AtomicU64::new(0)),
            total_latency_ns: Arc::new(AtomicU64::new(0)),
        }
    }

    fn record(&self, latency_ns: u64) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_latency_ns.fetch_add(latency_ns, Ordering::Relaxed);
    }
}

/// Actix middleware that measures request latency.
pub struct MetricsMiddleware {
    collector: MetricsCollector,
}

impl MetricsMiddleware {
    pub fn new(collector: MetricsCollector) -> Self {
        MetricsMiddleware { collector }
    }
}

impl<S, B> Transform<S, ServiceRequest> for MetricsMiddleware
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>
    + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = MetricsMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(MetricsMiddlewareService {
            service,
            collector: self.collector.clone(),
        })
    }
}

pub struct MetricsMiddlewareService<S> {
    service: S,
    collector: MetricsCollector,
}

impl<S, B> actix_web::dev::Service<ServiceRequest> for MetricsMiddlewareService<S>
where
    S: actix_web::dev::Service<ServiceRequest, Response = ServiceResponse<B>, Error = actix_web::Error>
    + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let collector = self.collector.clone();
        let start = Instant::now();
        let fut = self.service.call(req);

        Box::pin(async move {
            let res = fut.await?;
            let latency = start.elapsed().as_nanos() as u64;
            collector.record(latency);
            Ok(res)
        })
    }
}

#[derive(Serialize)]
struct ClusterStats {
    status: String,
    last_heartbeat: u128,
}

#[derive(Serialize)]
struct StatsResponse {
    tables: HashMap<String, usize>,
    total_keys: usize,
    total_requests: u64,
    average_latency_ms: f64,
    active_sse_connections: usize,
    /// Current cluster membership with status and last heartbeat
    cluster: HashMap<String, ClusterStats>,
}

/// GET /stats
#[get("/stats")]
pub async fn get_stats(
    state: Data<AppState>,
    sub: Data<SubscriptionManager>,
    metrics: Data<MetricsCollector>,
    cluster_data: Data<ClusterData>,
) -> impl Responder {
    // Count keys per table.
    let store = state.store.read().await;
    let mut tables = HashMap::new();
    let mut total_keys = 0;
    for (table, map) in store.iter() {
        tables.insert(table.clone(), map.len());
        total_keys += map.len();
    }

    // Compute request stats.
    let total_requests = metrics.total_requests.load(Ordering::Relaxed);
    let total_latency_ns = metrics.total_latency_ns.load(Ordering::Relaxed);
    let average_latency_ms = if total_requests > 0 {
        (total_latency_ns as f64 / total_requests as f64) / 1e6
    } else {
        0.0
    };

    // Count active SSE connections.
    let channels = sub.channels.read().await;
    let active_sse_connections = channels.values().map(|tx| tx.receiver_count()).sum();

    // Build cluster membership map.
    let cluster = {
        let nodes = cluster_data.nodes.read().await;
        nodes
            .iter()
            .map(|(addr, info)| {
                let status_str = format!("{:?}", info.status);
                (
                    addr.clone(),
                    ClusterStats {
                        status: status_str,
                        last_heartbeat: info.last_heartbeat,
                    },
                )
            })
            .collect::<HashMap<_, _>>()
    };

    let resp = StatsResponse {
        tables,
        total_keys,
        total_requests,
        average_latency_ms,
        active_sse_connections,
        cluster,
    };

    HttpResponse::Ok().json(resp)
}
