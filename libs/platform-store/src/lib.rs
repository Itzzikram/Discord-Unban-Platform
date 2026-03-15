use anyhow::Result;
use chrono::{TimeDelta, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::env;
use task_model::{ApprovalRecord, AuditEvent, CommandEnvelope, RateLimitObservation, TaskEnvelope, TaskResult};
use uuid::Uuid;

#[derive(Clone)]
pub struct PlatformStore {
    pool: PgPool,
}

impl PlatformStore {
    pub async fn connect_from_env() -> Result<Option<Self>> {
        let database_url = match env::var("DATABASE_URL") {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };

        let max_connections = env::var("DB_MAX_CONNECTIONS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(5);

        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(&database_url)
            .await?;

        Ok(Some(Self { pool }))
    }

    pub async fn record_command(&self, envelope: &CommandEnvelope) -> Result<()> {
        sqlx::query(
            "insert into commands (id, tenant_id, guild_id, action, requested_by, idempotency_key, dry_run, payload, status, approval_required, created_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             on conflict (idempotency_key) do nothing",
        )
        .bind(envelope.command_id)
        .bind(&envelope.request.tenant_id)
        .bind(&envelope.request.guild_id)
        .bind(&envelope.request.action)
        .bind(&envelope.request.requested_by)
        .bind(&envelope.request.idempotency_key)
        .bind(envelope.request.dry_run)
        .bind(envelope.request.payload.clone())
        .bind(if envelope.approval_required { "pending_approval" } else { "accepted" })
        .bind(envelope.approval_required)
        .bind(envelope.requested_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn create_pending_approval(&self, envelope: &CommandEnvelope) -> Result<()> {
        sqlx::query(
            "insert into approvals (command_id, tenant_id, status, reason)
             values ($1, $2, $3, $4)
             on conflict (command_id) do update set status = excluded.status, reason = excluded.reason, updated_at = now()",
        )
        .bind(envelope.command_id)
        .bind(&envelope.request.tenant_id)
        .bind("pending")
        .bind("bulk moderation approval required")
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn store_planned_tasks(&self, tasks: &[TaskEnvelope]) -> Result<()> {
        for task in tasks {
            sqlx::query(
                 "insert into tasks (id, command_id, guild_id, task_type, target_id, status, attempt_count, next_retry_at, created_at, requested_by, idempotency_key, lease_region, task_context)
                  values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                 on conflict (idempotency_key) do nothing",
            )
            .bind(task.task_id)
            .bind(task.command_id)
            .bind(&task.guild_id)
            .bind(&task.task_type)
            .bind(&task.target_id)
            .bind("planned")
            .bind(task.attempt as i32)
            .bind(Option::<chrono::DateTime<Utc>>::None)
            .bind(task.created_at)
            .bind(&task.requested_by)
            .bind(&task.idempotency_key)
            .bind(task.lease_region.clone())
            .bind(task.task_context.clone())
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn fetch_command(&self, command_id: Uuid) -> Result<Option<CommandEnvelope>> {
        let row: Option<(Uuid, chrono::DateTime<Utc>, bool, String, String, String, String, String, bool, sqlx::types::Json<serde_json::Value>)> = sqlx::query_as::<_, (Uuid, chrono::DateTime<Utc>, bool, String, String, String, String, String, bool, sqlx::types::Json<serde_json::Value>)>(
            "select id, created_at, approval_required, tenant_id, guild_id, action, requested_by, idempotency_key, dry_run, payload from commands where id = $1",
        )
        .bind(command_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| CommandEnvelope {
            command_id: row.0,
            requested_at: row.1,
            approval_required: row.2,
            request: task_model::CommandRequest {
                tenant_id: row.3,
                guild_id: row.4,
                action: row.5,
                requested_by: row.6,
                idempotency_key: row.7,
                dry_run: row.8,
                payload: row.9.0,
            },
        }))
    }

    pub async fn list_tasks_for_command(&self, command_id: Uuid) -> Result<Vec<TaskEnvelope>> {
        let rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, String, String, String, Option<String>, sqlx::types::Json<serde_json::Value>, i32, chrono::DateTime<Utc>)>(
            "select id, command_id, guild_id, task_type, target_id, requested_by, idempotency_key, lease_region, task_context, attempt_count, created_at from tasks where command_id = $1 order by created_at asc",
        )
        .bind(command_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| TaskEnvelope {
                task_id: row.0,
                command_id: row.1,
                guild_id: row.2,
                task_type: row.3,
                target_id: row.4,
                requested_by: row.5,
                idempotency_key: row.6,
                lease_region: row.7,
                task_context: row.8.0,
                attempt: row.9 as u32,
                created_at: row.10,
            })
            .collect())
    }

    pub async fn mark_tasks_queued(&self, command_id: Uuid) -> Result<u64> {
        let rows = sqlx::query("update tasks set status = 'queued', next_retry_at = null where command_id = $1")
            .bind(command_id)
            .execute(&self.pool)
            .await?
            .rows_affected();

        Ok(rows)
    }

    pub async fn update_command_status(&self, command_id: Uuid, status: &str) -> Result<()> {
        sqlx::query("update commands set status = $2 where id = $1")
            .bind(command_id)
            .bind(status)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn list_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        let rows = sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<String>, chrono::DateTime<Utc>, chrono::DateTime<Utc>)>(
            "select command_id, tenant_id, approved_by, status, reason, created_at, updated_at from approvals order by updated_at desc",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(map_approval).collect())
    }

    pub async fn get_approval(&self, command_id: Uuid) -> Result<Option<ApprovalRecord>> {
        let row = sqlx::query_as::<_, (Uuid, String, Option<String>, String, Option<String>, chrono::DateTime<Utc>, chrono::DateTime<Utc>)>(
            "select command_id, tenant_id, approved_by, status, reason, created_at, updated_at from approvals where command_id = $1",
        )
        .bind(command_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(map_approval))
    }

    pub async fn approve_command(&self, command_id: Uuid, approved_by: &str, reason: Option<&str>) -> Result<Option<ApprovalRecord>> {
        sqlx::query(
            "update approvals set status = 'approved', approved_by = $2, reason = $3, updated_at = now() where command_id = $1",
        )
        .bind(command_id)
        .bind(approved_by)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        self.update_command_status(command_id, "accepted").await?;
        self.get_approval(command_id).await
    }

    pub async fn reject_command(&self, command_id: Uuid, approved_by: &str, reason: Option<&str>) -> Result<Option<ApprovalRecord>> {
        sqlx::query(
            "update approvals set status = 'rejected', approved_by = $2, reason = $3, updated_at = now() where command_id = $1",
        )
        .bind(command_id)
        .bind(approved_by)
        .bind(reason)
        .execute(&self.pool)
        .await?;

        self.update_command_status(command_id, "rejected").await?;
        self.get_approval(command_id).await
    }

    pub async fn delete_approval(&self, command_id: Uuid) -> Result<u64> {
        let rows = sqlx::query("delete from approvals where command_id = $1")
            .bind(command_id)
            .execute(&self.pool)
            .await?
            .rows_affected();

        Ok(rows)
    }

    pub async fn record_task_result(&self, result: &TaskResult) -> Result<()> {
        sqlx::query(
            "insert into task_results (task_id, command_id, guild_id, route, bucket_key, success, status_code, retry_after_ms, details, retryable, completed_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(result.task_id)
        .bind(result.command_id)
        .bind(&result.guild_id)
        .bind(&result.route)
        .bind(&result.bucket_key)
        .bind(result.success)
        .bind(result.status_code.map(|value| value as i32))
        .bind(result.retry_after_ms.map(|value| value as i64))
        .bind(&result.details)
        .bind(result.retryable)
        .bind(result.completed_at)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "update tasks set status = $2, attempt_count = attempt_count + 1, next_retry_at = $3 where id = $1",
        )
        .bind(result.task_id)
        .bind(if result.success { "completed" } else { "failed" })
        .bind(if result.retryable {
            Some(Utc::now() + TimeDelta::seconds(30))
        } else {
            None
        })
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn record_ratelimit_observation(&self, observation: &RateLimitObservation) -> Result<()> {
        sqlx::query(
            "insert into route_ratelimit_observations (command_id, task_id, guild_id, route, bucket_key, status_code, retry_after_ms, is_global, observed_at)
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(observation.command_id)
        .bind(observation.task_id)
        .bind(&observation.guild_id)
        .bind(&observation.route)
        .bind(&observation.bucket_key)
        .bind(observation.status_code.map(|value| value as i32))
        .bind(observation.retry_after_ms.map(|value| value as i64))
        .bind(observation.global)
        .bind(observation.observed_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_ratelimit_observations(
        &self,
        tenant_id: Option<&str>,
        route: Option<&str>,
        limit: i64,
    ) -> Result<Vec<RateLimitObservation>> {
        let rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, String, Option<i32>, Option<i64>, bool, chrono::DateTime<Utc>)>(
            "select r.command_id, r.task_id, r.guild_id, r.route, r.bucket_key, r.status_code, r.retry_after_ms, r.is_global, r.observed_at
             from route_ratelimit_observations r
             join commands c on c.id = r.command_id
             where ($1::text is null or c.tenant_id = $1)
               and ($2::text is null or r.route = $2)
             order by r.observed_at desc
             limit $3",
        )
        .bind(tenant_id)
        .bind(route)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| RateLimitObservation {
                command_id: row.0,
                task_id: row.1,
                guild_id: row.2,
                route: row.3,
                bucket_key: row.4,
                status_code: row.5.map(|value| value as u16),
                retry_after_ms: row.6.map(|value| value as u64),
                global: row.7,
                observed_at: row.8,
            })
            .collect())
    }

    pub async fn record_audit(&self, event: &AuditEvent) -> Result<()> {
        sqlx::query(
            "insert into audit_events (id, tenant_id, command_id, task_id, event_type, payload, created_at)
             values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(event.event_id)
        .bind(&event.tenant_id)
        .bind(event.command_id)
        .bind(event.task_id)
        .bind(&event.event_type)
        .bind(event.payload.clone())
        .bind(event.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn acquire_execution_lease(&self, guild_id: &str, region: &str) -> Result<bool> {
        let expires_at = Utc::now() + TimeDelta::minutes(5);

        let rows = sqlx::query(
            "insert into regional_execution_leases (guild_id, region, expires_at)
             values ($1, $2, $3)
             on conflict (guild_id)
             do update set region = excluded.region, expires_at = excluded.expires_at
             where regional_execution_leases.expires_at < now() or regional_execution_leases.region = excluded.region",
        )
        .bind(guild_id)
        .bind(region)
        .bind(expires_at)
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(rows > 0)
    }
}

fn map_approval(row: (Uuid, String, Option<String>, String, Option<String>, chrono::DateTime<Utc>, chrono::DateTime<Utc>)) -> ApprovalRecord {
    ApprovalRecord {
        command_id: row.0,
        tenant_id: row.1,
        approved_by: row.2,
        status: row.3,
        reason: row.4,
        created_at: row.5,
        updated_at: row.6,
    }
}