use axum::{extract::Path, routing::get, Json, Router};
use std::{env, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("rate-limit-service");

    let port = env::var("PORT").unwrap_or_else(|_| "8083".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/limits/{route}", get(limit_for_route))
        .layer(TraceLayer::new_for_http());

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "rate-limit-service", "status": "ok" }))
}

async fn limit_for_route(Path(route): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "global_key": rate_limit::GLOBAL_KEY,
        "route_key": rate_limit::route_key(&route),
        "remaining": 0,
        "coordination": "redis"
    }))
}
