use axum::{http::{header, HeaderValue, StatusCode}, response::IntoResponse, routing::get, Router};
use prometheus::{Encoder, IntGauge, Registry, TextEncoder};
use std::{env, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("metrics-service");

    let port = env::var("PORT").unwrap_or_else(|_| "8087".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .layer(TraceLayer::new_for_http());

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> impl IntoResponse {
    "ok"
}

async fn metrics() -> Result<impl IntoResponse, StatusCode> {
    let registry = Registry::new();

    let queue_size = IntGauge::new("queue_size", "Current queued tasks").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    queue_size.set(0);
    registry.register(Box::new(queue_size)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let worker_count = IntGauge::new("worker_count", "Current worker replicas").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    worker_count.set(3);
    registry.register(Box::new(worker_count)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let encoder = TextEncoder::new();
    let families = registry.gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let body = String::from_utf8(buffer).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(([(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4"))], body))
}