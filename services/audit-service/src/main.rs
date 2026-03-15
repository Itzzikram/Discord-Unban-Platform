use axum::{routing::{get, post}, Json, Router};
use std::{env, net::SocketAddr};
use task_model::AuditEvent;
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("audit-service");

    let port = env::var("PORT").unwrap_or_else(|_| "8084".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/audit", post(record_audit))
        .layer(TraceLayer::new_for_http());

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "audit-service", "status": "ok" }))
}

async fn record_audit(Json(event): Json<AuditEvent>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "accepted": true,
        "event_id": event.event_id,
        "topic": task_model::AUDIT_TOPIC
    }))
}
