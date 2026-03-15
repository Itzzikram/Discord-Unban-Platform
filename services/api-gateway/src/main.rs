use auth::{authorize_analytics_read, authorize_approval_decision, authorize_approval_read, authorize_command, JwtSecret, VerifiedClaims};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
    routing::{get, post},
    Json, Router,
};
use kafka_runtime::{create_producer, publish_json, KafkaConfig};
use rdkafka::producer::FutureProducer;
use serde::Deserialize;
use std::{env, net::SocketAddr, sync::Arc};
use task_model::{plan_tasks, ApprovalDecisionRequest, ApprovalList, CommandEnvelope, CommandRequest};
use tracing::{info, warn};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

#[derive(Deserialize)]
struct RateLimitAnalyticsQuery {
    tenant_id: Option<String>,
    route: Option<String>,
    limit: Option<i64>,
}

#[derive(Clone)]
struct AppState {
    service_name: Arc<str>,
    store: Option<platform_store::PlatformStore>,
    producer: FutureProducer,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("api-gateway");

    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;
    let producer = create_producer(&KafkaConfig::from_env("api-gateway"))?;
    let jwt_secret = Arc::from(env::var("JWT_SECRET").unwrap_or_else(|_| "development-secret".to_string()));
    let state = AppState {
        service_name: Arc::from("api-gateway"),
        store: platform_store::PlatformStore::connect_from_env().await?,
        producer,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/commands", post(create_command))
        .route("/v1/approvals", get(list_approvals))
        .route("/v1/approvals/{command_id}", get(get_approval).delete(delete_approval))
        .route("/v1/approvals/{command_id}/approve", post(approve_approval))
        .route("/v1/approvals/{command_id}/reject", post(reject_approval))
        .route("/v1/analytics/ratelimits", get(list_ratelimit_analytics))
        .layer(TraceLayer::new_for_http())
        .layer(Extension(JwtSecret(jwt_secret)))
        .with_state(state);

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({ "service": state.service_name.as_ref(), "status": "ok" }))
}

async fn create_command(
    State(state): State<AppState>,
    claims: VerifiedClaims,
    Json(request): Json<CommandRequest>,
) -> impl IntoResponse {
    let authorization = match authorize_command(&claims, &request) {
        Ok(outcome) => outcome,
        Err(error) => {
            return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": error.to_string() })));
        }
    };

    let command_id = Uuid::new_v4();
    let tasks = plan_tasks(command_id, &request);
    let planned_task_count = tasks.len();

    let envelope = CommandEnvelope {
        command_id,
        requested_at: chrono::Utc::now(),
        approval_required: authorization.approval_required,
        request,
    };

    if let Some(store) = &state.store {
        if let Err(error) = store.record_command(&envelope).await {
            warn!(%error, "failed to persist command envelope");
        }
        if let Err(error) = store.store_planned_tasks(&tasks).await {
            warn!(%error, command_id = %envelope.command_id, "failed to persist planned tasks");
        }
        if envelope.approval_required {
            if let Err(error) = store.create_pending_approval(&envelope).await {
                warn!(%error, command_id = %envelope.command_id, "failed to create pending approval record");
            }
        }
    }

    if !envelope.approval_required {
        if let Err(error) = release_command(&state, &envelope, &tasks).await {
            warn!(%error, command_id = %envelope.command_id, "automatic command release failed");
        }
    }

    info!(subject = %authorization.subject, action = %envelope.request.action, guild_id = %envelope.request.guild_id, approval_required = envelope.approval_required, planned_task_count, "validated command");

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "command_id": envelope.command_id,
            "status": if envelope.approval_required { "pending_approval" } else { "accepted" },
            "tenant_id": authorization.tenant_id,
            "planned_task_count": planned_task_count,
            "next": if envelope.approval_required {
                "approval workflow"
            } else {
                "publish to Kafka topic platform.commands"
            }
        })),
    )
}

async fn list_approvals(State(state): State<AppState>, claims: VerifiedClaims) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    match store.list_approvals().await {
        Ok(approvals) => {
            let filtered: Vec<_> = approvals
                .into_iter()
                .filter(|approval| authorize_approval_read(&claims, &approval.tenant_id).is_ok())
                .collect();

            (
            StatusCode::OK,
            Json(serde_json::to_value(ApprovalList { approvals: filtered }).unwrap_or_else(|_| serde_json::json!({ "approvals": [] }))),
        )
        },
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    }
}

async fn get_approval(State(state): State<AppState>, claims: VerifiedClaims, Path(command_id): Path<Uuid>) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    match store.get_approval(command_id).await {
        Ok(Some(approval)) => {
            if let Err(error) = authorize_approval_read(&claims, &approval.tenant_id) {
                return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": error.to_string() })));
            }

            (StatusCode::OK, Json(serde_json::to_value(approval).unwrap_or_else(|_| serde_json::json!({})) ))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    }
}

async fn approve_approval(
    State(state): State<AppState>,
    claims: VerifiedClaims,
    Path(command_id): Path<Uuid>,
    Json(decision): Json<ApprovalDecisionRequest>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    let current = match store.get_approval(command_id).await {
        Ok(Some(approval)) => approval,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    if let Err(error) = authorize_approval_decision(&claims, &current.tenant_id, &decision.approved_by) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": error.to_string() })));
    }

    let approval = match store.approve_command(command_id, &decision.approved_by, decision.reason.as_deref()).await {
        Ok(Some(approval)) => approval,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    let Some(command) = (match store.fetch_command(command_id).await {
        Ok(command) => command,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    }) else {
        return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "command not found" })));
    };

    let tasks = match store.list_tasks_for_command(command_id).await {
        Ok(tasks) => tasks,
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    if let Err(error) = release_command(&state, &command, &tasks).await {
        return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": error.to_string() })));
    }

    (StatusCode::OK, Json(serde_json::to_value(approval).unwrap_or_else(|_| serde_json::json!({})) ))
}

async fn reject_approval(
    State(state): State<AppState>,
    claims: VerifiedClaims,
    Path(command_id): Path<Uuid>,
    Json(decision): Json<ApprovalDecisionRequest>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    let current = match store.get_approval(command_id).await {
        Ok(Some(approval)) => approval,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    if let Err(error) = authorize_approval_decision(&claims, &current.tenant_id, &decision.approved_by) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": error.to_string() })));
    }

    match store.reject_command(command_id, &decision.approved_by, decision.reason.as_deref()).await {
        Ok(Some(approval)) => (StatusCode::OK, Json(serde_json::to_value(approval).unwrap_or_else(|_| serde_json::json!({})) )),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    }
}

async fn delete_approval(State(state): State<AppState>, claims: VerifiedClaims, Path(command_id): Path<Uuid>) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    let approval = match store.get_approval(command_id).await {
        Ok(Some(approval)) => approval,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    if let Err(error) = authorize_approval_decision(&claims, &approval.tenant_id, &claims.sub) {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": error.to_string() })));
    }

    match store.delete_approval(command_id).await {
        Ok(0) => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "approval not found" }))),
        Ok(_) => (StatusCode::NO_CONTENT, Json(serde_json::json!({}))),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    }
}

async fn list_ratelimit_analytics(
    State(state): State<AppState>,
    claims: VerifiedClaims,
    Query(query): Query<RateLimitAnalyticsQuery>,
) -> impl IntoResponse {
    let Some(store) = &state.store else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": "database unavailable" })));
    };

    let tenant_scope = match authorize_analytics_read(&claims, query.tenant_id.as_deref()) {
        Ok(scope) => scope,
        Err(error) => return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": error.to_string() }))),
    };

    match store
        .list_ratelimit_observations(tenant_scope.as_deref(), query.route.as_deref(), query.limit.unwrap_or(100).clamp(1, 500))
        .await
    {
        Ok(observations) => (StatusCode::OK, Json(serde_json::json!({ "observations": observations }))),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": error.to_string() }))),
    }
}

async fn release_command(state: &AppState, command: &CommandEnvelope, tasks: &[task_model::TaskEnvelope]) -> anyhow::Result<()> {
    publish_json(&state.producer, task_model::COMMANDS_TOPIC, &command.request.guild_id, command).await?;

    for task in tasks {
        publish_json(&state.producer, task_model::TASKS_TOPIC, &task.guild_id, task).await?;
    }

    if let Some(store) = &state.store {
        store.mark_tasks_queued(command.command_id).await?;
        store.update_command_status(command.command_id, "queued").await?;
    }

    Ok(())
}
