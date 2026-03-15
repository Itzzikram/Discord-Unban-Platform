use axum::{extract::{Path, State}, http::StatusCode, routing::{get, post}, Json, Router};
use kafka_runtime::{create_producer, publish_json, KafkaConfig};
use rdkafka::producer::FutureProducer;
use std::{env, net::SocketAddr, sync::Arc};
use task_model::{CommandEnvelope, TaskEnvelope};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    producer: FutureProducer,
    config: Arc<KafkaConfig>,
    store: Option<platform_store::PlatformStore>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("queue-service");

    let port = env::var("PORT").unwrap_or_else(|_| "8085".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let config = Arc::new(KafkaConfig::from_env("queue-service"));
    let producer = create_producer(&config)?;
    let state = AppState { producer, config, store: platform_store::PlatformStore::connect_from_env().await? };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/topics", get(topics))
        .route("/v1/topics/lag-targets", get(lag_targets))
        .route("/v1/commands", post(publish_command))
        .route("/v1/tasks", post(publish_task))
        .route("/v1/releases/{command_id}", post(release_command))
        .route("/v1/replay/{command_id}", post(replay_command))
        .layer(TraceLayer::new_for_http());
        
    let app = app.with_state(state);

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "service": "queue-service",
        "status": "ok",
        "backend": "apache-kafka",
        "brokers": state.config.brokers,
        "consumer_group": state.config.consumer_group,
    }))
}

async fn topics() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "backend": "apache-kafka",
        "topics": [
            task_model::COMMANDS_TOPIC,
            task_model::TASKS_TOPIC,
            task_model::RESULTS_TOPIC,
            task_model::AUDIT_TOPIC,
            task_model::DLQ_TOPIC,
            task_model::EVENTS_TOPIC,
            task_model::METRICS_TOPIC
        ]
    }))
}

async fn lag_targets(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "consumer_group": state.config.consumer_group,
        "scale_up_threshold": state.config.lag_scale_up_threshold,
        "scale_out_threshold": state.config.lag_scale_out_threshold
    }))
}

async fn publish_command(
    State(state): State<AppState>,
    Json(command): Json<CommandEnvelope>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = command.request.guild_id.clone();

    match publish_json(&state.producer, task_model::COMMANDS_TOPIC, &key, &command).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "topic": task_model::COMMANDS_TOPIC, "command_id": command.command_id })),
        ),
        Err(error) => {
            warn!(%error, "failed to publish command");
            (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": error.to_string() })))
        }
    }
}

async fn publish_task(
    State(state): State<AppState>,
    Json(task): Json<TaskEnvelope>,
) -> (StatusCode, Json<serde_json::Value>) {
    let key = task.guild_id.clone();

    match publish_json(&state.producer, task_model::TASKS_TOPIC, &key, &task).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "topic": task_model::TASKS_TOPIC, "task_id": task.task_id })),
        ),
        Err(error) => {
            warn!(%error, "failed to publish task");
            (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": error.to_string() })))
        }
    }
}

async fn release_command(State(state): State<AppState>, Path(command_id): Path<Uuid>) -> (StatusCode, Json<serde_json::Value>) {
    release_from_store(state, command_id, "queued").await
}

async fn replay_command(State(state): State<AppState>, Path(command_id): Path<Uuid>) -> (StatusCode, Json<serde_json::Value>) {
    release_from_store(state, command_id, "replayed").await
}

async fn release_from_store(state: AppState, command_id: Uuid, next_status: &str) -> (StatusCode, Json<serde_json::Value>) {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    let command = match store.fetch_command(command_id).await {
        Ok(Some(command)) => command,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "command not found" }))),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    let tasks = match store.list_tasks_for_command(command_id).await {
        Ok(tasks) => tasks,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    if let Err(error) = publish_json(&state.producer, task_model::COMMANDS_TOPIC, &command.request.guild_id, &command).await {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": error.to_string() })));
    }

    for task in &tasks {
        if let Err(error) = publish_json(&state.producer, task_model::TASKS_TOPIC, &task.guild_id, task).await {
            return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": error.to_string() })));
        }
    }

    if let Err(error) = store.mark_tasks_queued(command_id).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() })));
    }

    if let Err(error) = store.update_command_status(command_id, next_status).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() })));
    }

    (StatusCode::OK, Json(serde_json::json!({ "command_id": command_id, "released_tasks": tasks.len(), "status": next_status })))
}