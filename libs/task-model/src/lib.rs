use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const COMMANDS_TOPIC: &str = "platform.commands";
pub const TASKS_TOPIC: &str = "platform.tasks";
pub const RESULTS_TOPIC: &str = "platform.results";
pub const AUDIT_TOPIC: &str = "platform.audit";
pub const EVENTS_TOPIC: &str = "platform.events";
pub const METRICS_TOPIC: &str = "platform.metrics";
pub const DLQ_TOPIC: &str = "platform.dlq";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRequest {
    pub tenant_id: String,
    pub guild_id: String,
    pub action: String,
    pub requested_by: String,
    pub idempotency_key: String,
    pub dry_run: bool,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub command_id: Uuid,
    pub requested_at: DateTime<Utc>,
    pub approval_required: bool,
    pub request: CommandRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub command_id: Uuid,
    pub tenant_id: String,
    pub approved_by: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecisionRequest {
    pub approved_by: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalList {
    pub approvals: Vec<ApprovalRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEnvelope {
    pub task_id: Uuid,
    pub command_id: Uuid,
    pub guild_id: String,
    pub task_type: String,
    pub target_id: String,
    pub requested_by: String,
    pub idempotency_key: String,
    pub lease_region: Option<String>,
    pub task_context: serde_json::Value,
    pub attempt: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub command_id: Uuid,
    pub guild_id: String,
    pub success: bool,
    pub route: String,
    pub bucket_key: String,
    pub status_code: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub details: String,
    pub retryable: bool,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitObservation {
    pub command_id: Uuid,
    pub task_id: Uuid,
    pub guild_id: String,
    pub route: String,
    pub bucket_key: String,
    pub status_code: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub global: bool,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: Uuid,
    pub tenant_id: String,
    pub command_id: Uuid,
    pub task_id: Option<Uuid>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub fn expand_command(request: &CommandRequest) -> Vec<TaskEnvelope> {
    plan_tasks(Uuid::new_v4(), request)
}

pub fn plan_tasks(command_id: Uuid, request: &CommandRequest) -> Vec<TaskEnvelope> {
    let targets = request
        .payload
        .get("targets")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let lease_region = request
        .payload
        .get("region")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    let shared_task_context = request
        .payload
        .get("task_context")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let created_at = Utc::now();

    targets
        .into_iter()
        .filter_map(|target| target.as_str().map(ToOwned::to_owned))
        .map(|target_id| {
            let task_id = Uuid::new_v4();

            TaskEnvelope {
                task_id,
                command_id,
                guild_id: request.guild_id.clone(),
                task_type: request.action.clone(),
                target_id: target_id.clone(),
                requested_by: request.requested_by.clone(),
                idempotency_key: format!("{}:{command_id}:{target_id}", request.idempotency_key),
                lease_region: lease_region.clone(),
                task_context: shared_task_context.clone(),
                attempt: 0,
                created_at,
            }
        })
        .collect()
}

pub fn target_count(request: &CommandRequest) -> usize {
    request
        .payload
        .get("targets")
        .and_then(|value| value.as_array())
        .map(|targets| targets.len())
        .unwrap_or(0)
}
