use chrono::Utc;
use discord_client::{build_runtime_config, create_http_client, execute_task, preview_task};
use kafka_runtime::{commit_message, create_consumer, create_producer, decode_json, publish_json, KafkaConfig};
use rate_limit::{route_key, user_profile_key, RedisBackplane};
use std::{env, sync::Arc, time::Duration};
use task_model::{AuditEvent, RateLimitObservation, TaskEnvelope, TaskResult, AUDIT_TOPIC, RESULTS_TOPIC, TASKS_TOPIC};
use tracing::{error, info, warn};
use twilight_http::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("worker");
    let runtime = build_runtime_config(env::var("DISCORD_SHARD_COUNT")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(16));
    let token = match env::var("DISCORD_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            warn!(shard_count = runtime.shard_count, intents_bits = runtime.intents_bits, "DISCORD_TOKEN is not set; worker loop will not start");
            return Ok(());
        }
    };
    let http_client = Arc::new(create_http_client(token));
    let kafka = KafkaConfig::from_env("worker");
    let consumer = create_consumer(&kafka, &[TASKS_TOPIC])?;
    let producer = create_producer(&kafka)?;
    let backplane = RedisBackplane::from_env()?;
    let store = platform_store::PlatformStore::connect_from_env().await?;
    let region = env::var("REGION").unwrap_or_else(|_| "local".to_string());

    info!(shard_count = runtime.shard_count, intents_bits = runtime.intents_bits, consumer_group = kafka.consumer_group, "worker started with Twilight, Kafka, and Redis wiring");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("worker shutdown requested");
                break;
            }
            message = consumer.recv() => {
                match message {
                    Err(error) => warn!(%error, "Kafka receive failed"),
                    Ok(message) => {
                        if let Err(error) = handle_message(&consumer, &producer, http_client.clone(), backplane.as_ref(), store.as_ref(), &region, &message).await {
                            error!(%error, "worker task handling failed");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_message(
    consumer: &rdkafka::consumer::StreamConsumer,
    producer: &rdkafka::producer::FutureProducer,
    http_client: Arc<Client>,
    backplane: Option<&RedisBackplane>,
    store: Option<&platform_store::PlatformStore>,
    region: &str,
    message: &rdkafka::message::BorrowedMessage<'_>,
) -> anyhow::Result<()> {
    let task: TaskEnvelope = decode_json(message)?;

    if let Some(store) = store {
        if !store.acquire_execution_lease(&task.guild_id, task.lease_region.as_deref().unwrap_or(region)).await? {
            warn!(task_id = %task.task_id, guild_id = %task.guild_id, "skipping task because another region owns the lease");
            commit_message(consumer, message)?;
            return Ok(());
        }
    }

    let preview = preview_task(&task)?;
    let route_bucket_key = route_key(&preview.route);

    if let Some(backplane) = backplane {
        let bucket = backplane.acquire(&route_bucket_key, 1, 1_000).await?;
        if !bucket.is_available(Utc::now()) {
            let wait_ms = (bucket.reset_at.timestamp_millis() - Utc::now().timestamp_millis()).max(0) as u64;
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }

        if let Ok(Some(cached_profile)) = backplane.get_cache(&user_profile_key(&task.target_id)).await {
            info!(task_id = %task.task_id, cache_hit = true, cached_profile = %cached_profile, "loaded cached user profile before Discord action");
        }
    }

    let result = match execute_task(&http_client, &task).await {
        Ok(outcome) => TaskResult {
            task_id: task.task_id,
            command_id: task.command_id,
            guild_id: task.guild_id.clone(),
            success: outcome.success,
            route: outcome.route.clone(),
            bucket_key: outcome.bucket_key.clone(),
            status_code: outcome.status_code,
            retry_after_ms: outcome.retry_after_ms,
            details: outcome.details.clone(),
            retryable: outcome.retry_after_ms.is_some() || !outcome.success,
            completed_at: Utc::now(),
        },
        Err(error) => TaskResult {
            task_id: task.task_id,
            command_id: task.command_id,
            guild_id: task.guild_id.clone(),
            success: false,
            route: preview.route.clone(),
            bucket_key: preview.bucket_key.clone(),
            status_code: None,
            retry_after_ms: None,
            details: error.to_string(),
            retryable: true,
            completed_at: Utc::now(),
        },
    };

    if let Some(backplane) = backplane {
        if let Some(retry_after_ms) = result.retry_after_ms {
            let _ = backplane.update_retry_after(&route_bucket_key, retry_after_ms).await?;
        } else if !result.success {
            let _ = backplane.update_retry_after(&route_bucket_key, 1_000).await?;
        }
    }

    publish_json(producer, RESULTS_TOPIC, &task.guild_id, &result).await?;

    let audit = AuditEvent {
        event_id: uuid::Uuid::new_v4(),
        tenant_id: "default".to_string(),
        command_id: task.command_id,
        task_id: Some(task.task_id),
        event_type: if result.success { "task_completed".to_string() } else { "task_failed".to_string() },
        payload: serde_json::json!({
            "guild_id": task.guild_id,
            "target_id": task.target_id,
            "details": result.details,
            "retryable": result.retryable,
        }),
        created_at: Utc::now(),
    };

    publish_json(producer, AUDIT_TOPIC, &audit.command_id.to_string(), &audit).await?;

    if let Some(store) = store {
        store.record_task_result(&result).await?;
        store.record_audit(&audit).await?;
        store.record_ratelimit_observation(&RateLimitObservation {
            command_id: task.command_id,
            task_id: task.task_id,
            guild_id: task.guild_id.clone(),
            route: result.route.clone(),
            bucket_key: result.bucket_key.clone(),
            status_code: result.status_code,
            retry_after_ms: result.retry_after_ms,
            global: result.retry_after_ms.is_some() && result.details.contains("ratelimited"),
            observed_at: Utc::now(),
        }).await?;
    }

    commit_message(consumer, message)?;
    Ok(())
}
