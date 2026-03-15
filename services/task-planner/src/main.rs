use axum::{routing::{get, post}, Json, Router};
use std::{env, net::SocketAddr};
use task_model::{expand_command, CommandRequest};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("task-planner");

    let port = env::var("PORT").unwrap_or_else(|_| "8081".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/internal/plan", post(plan))
        .layer(TraceLayer::new_for_http());

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "task-planner", "status": "ok" }))
}

async fn plan(Json(request): Json<CommandRequest>) -> Json<serde_json::Value> {
    let tasks = expand_command(&request);

    Json(serde_json::json!({
        "task_count": tasks.len(),
        "tasks": tasks,
        "publish_topic": task_model::TASKS_TOPIC
    }))
}
