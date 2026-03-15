use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use task_model::TaskEnvelope;
use twilight_http::Client;
use twilight_http::api_error::ApiError;
use twilight_http::error::ErrorType;
use twilight_model::{
    gateway::Intents,
    id::{
        marker::{ChannelMarker, GuildMarker, MessageMarker, RoleMarker, UserMarker},
        Id,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordRuntimeConfig {
    pub shard_count: u32,
    pub intents_bits: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordRequestPreview {
    pub method: String,
    pub route: String,
    pub bucket_key: String,
    pub guild_id: u64,
    pub target_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionOutcome {
    pub route: String,
    pub bucket_key: String,
    pub success: bool,
    pub status_code: Option<u16>,
    pub retry_after_ms: Option<u64>,
    pub global_ratelimit: bool,
    pub details: String,
}

pub fn recommended_intents() -> Intents {
    Intents::GUILDS | Intents::GUILD_MEMBERS | Intents::GUILD_MODERATION
}

pub fn build_runtime_config(shard_count: u32) -> DiscordRuntimeConfig {
    DiscordRuntimeConfig {
        shard_count,
        intents_bits: recommended_intents().bits(),
    }
}

pub fn create_http_client(token: impl Into<String>) -> Client {
    Client::new(token.into())
}

pub async fn execute_task(client: &Client, task: &TaskEnvelope) -> Result<ExecutionOutcome> {
    let preview = preview_task(task)?;

    let result = match task.task_type.as_str() {
        "unban" | "unban_all" => client
            .delete_ban(parse_guild_id(&task.guild_id)?, parse_user_id(&task.target_id)?)
            .await,
        "ban" => {
            let delete_message_seconds = task
                .task_context
                .get("delete_message_seconds")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as u32;

            client
                .create_ban(parse_guild_id(&task.guild_id)?, parse_user_id(&task.target_id)?)
                .delete_message_seconds(delete_message_seconds)
                .await
        }
        "kick" => client
            .remove_guild_member(parse_guild_id(&task.guild_id)?, parse_user_id(&task.target_id)?)
            .await,
        "delete_message" => {
            let channel_id = task
                .task_context
                .get("channel_id")
                .ok_or_else(|| anyhow!("delete_message requires channel_id in task_context"))?;
            client
                .delete_message(
                    parse_channel_id_value(channel_id, "channel_id")?,
                    parse_message_id(&task.target_id)?,
                )
                .await
        }
        "remove_member_role" => {
            let role_id = task
                .task_context
                .get("role_id")
                .ok_or_else(|| anyhow!("remove_member_role requires role_id in task_context"))?;
            client
                .remove_guild_member_role(
                    parse_guild_id(&task.guild_id)?,
                    parse_user_id(&task.target_id)?,
                    parse_role_id_value(role_id, "role_id")?,
                )
                .await
        }
        "add_member_role" => {
            let role_id = task
                .task_context
                .get("role_id")
                .ok_or_else(|| anyhow!("add_member_role requires role_id in task_context"))?;
            client
                .add_guild_member_role(
                    parse_guild_id(&task.guild_id)?,
                    parse_user_id(&task.target_id)?,
                    parse_role_id_value(role_id, "role_id")?,
                )
                .await
        }
        other => return Err(anyhow!("unsupported Discord action `{other}`")),
    };

    match result {
        Ok(_) => Ok(ExecutionOutcome {
            route: preview.route,
            bucket_key: preview.bucket_key,
            success: true,
            status_code: Some(204),
            retry_after_ms: None,
            global_ratelimit: false,
            details: format!("{} executed at {}", task.task_type, Utc::now()),
        }),
        Err(error) => Ok(execution_outcome_from_error(preview, error)),
    }
}

pub fn preview_task(task: &TaskEnvelope) -> Result<DiscordRequestPreview> {
    let guild_id = parse_guild_id(&task.guild_id)?;
    let target_id = parse_user_id(&task.target_id)?;

    let (method, route, bucket_key) = match task.task_type.as_str() {
        "unban" | "unban_all" => (
            "DELETE".to_string(),
            format!("/guilds/{}/bans/{}", guild_id.get(), target_id.get()),
            "guild-ban-delete".to_string(),
        ),
        "ban" => (
            "PUT".to_string(),
            format!("/guilds/{}/bans/{}", guild_id.get(), target_id.get()),
            "guild-ban-create".to_string(),
        ),
        "kick" => (
            "DELETE".to_string(),
            format!("/guilds/{}/members/{}", guild_id.get(), target_id.get()),
            "guild-member-delete".to_string(),
        ),
        "delete_message" => (
            "DELETE".to_string(),
            format!(
                "/channels/{}/messages/{}",
                task.task_context
                    .get("channel_id")
                    .and_then(id_value_as_string)
                    .unwrap_or_else(|| "unknown".to_string()),
                task.target_id
            ),
            "channel-message-delete".to_string(),
        ),
        "remove_member_role" | "add_member_role" => (
            "DELETE".to_string(),
            format!(
                "/guilds/{}/members/{}/roles/{}",
                guild_id.get(),
                target_id.get(),
                task.task_context
                    .get("role_id")
                    .and_then(id_value_as_string)
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            if task.task_type == "add_member_role" {
                "guild-member-role-add".to_string()
            } else {
                "guild-member-role-delete".to_string()
            },
        ),
        other => (
            "POST".to_string(),
            format!("/guilds/{}/actions/{other}/{}", guild_id.get(), target_id.get()),
            format!("generic-{other}"),
        ),
    };

    Ok(DiscordRequestPreview {
        method,
        route,
        bucket_key,
        guild_id: guild_id.get(),
        target_id: target_id.get(),
    })
}

pub fn parse_guild_id(raw: &str) -> Result<Id<GuildMarker>> {
    parse_id(raw).map(Id::new)
}

pub fn parse_user_id(raw: &str) -> Result<Id<UserMarker>> {
    parse_id(raw).map(Id::new)
}

pub fn parse_channel_id(raw: &str) -> Result<Id<ChannelMarker>> {
    parse_id(raw).map(Id::new)
}

pub fn parse_message_id(raw: &str) -> Result<Id<MessageMarker>> {
    parse_id(raw).map(Id::new)
}

pub fn parse_role_id(raw: &str) -> Result<Id<RoleMarker>> {
    parse_id(raw).map(Id::new)
}

fn parse_channel_id_value(value: &serde_json::Value, field: &str) -> Result<Id<ChannelMarker>> {
    parse_id_value(value, field).map(Id::new)
}

fn parse_role_id_value(value: &serde_json::Value, field: &str) -> Result<Id<RoleMarker>> {
    parse_id_value(value, field).map(Id::new)
}

fn parse_id_value(value: &serde_json::Value, field: &str) -> Result<u64> {
    match value {
        serde_json::Value::String(raw) => parse_id(raw),
        serde_json::Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| anyhow!("expected `{field}` to be a positive integer snowflake")),
        _ => Err(anyhow!(
            "expected `{field}` to be a string or number snowflake"
        )),
    }
}

fn id_value_as_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => Some(raw.clone()),
        serde_json::Value::Number(number) => number.as_u64().map(|raw| raw.to_string()),
        _ => None,
    }
}

fn execution_outcome_from_error(preview: DiscordRequestPreview, error: twilight_http::Error) -> ExecutionOutcome {
    let mut retry_after_ms = None;
    let mut status_code = None;
    let mut details = error.to_string();
    let mut global_ratelimit = false;

    if let ErrorType::Response { body, status, .. } = error.kind() {
        status_code = Some(status.get());

        if let Ok(api_error) = serde_json::from_slice::<ApiError>(body.as_ref()) {
            if let ApiError::Ratelimited(ratelimited) = api_error {
                retry_after_ms = Some((ratelimited.retry_after * 1000.0).ceil() as u64);
                global_ratelimit = ratelimited.global;
                details = format!("ratelimited for route {}", preview.route);
            }
        }
    }

    ExecutionOutcome {
        route: preview.route,
        bucket_key: preview.bucket_key,
        success: false,
        status_code,
        retry_after_ms,
        global_ratelimit,
        details,
    }
}

fn parse_id(raw: &str) -> Result<u64> {
    raw.parse::<u64>()
        .map_err(|_| anyhow!("expected Discord snowflake id, got `{raw}`"))
}

#[cfg(test)]
mod tests {
    use super::preview_task;
    use chrono::Utc;
    use serde_json::json;
    use task_model::TaskEnvelope;
    use uuid::Uuid;

    fn build_task(task_type: &str) -> TaskEnvelope {
        TaskEnvelope {
            task_id: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            guild_id: "123456789012345678".to_string(),
            task_type: task_type.to_string(),
            target_id: "987654321098765432".to_string(),
            requested_by: "tester".to_string(),
            idempotency_key: "test-key".to_string(),
            lease_region: Some("us-east".to_string()),
            task_context: json!({}),
            attempt: 0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn preview_unban_uses_delete_ban_route_and_bucket() {
        let task = build_task("unban");

        let preview = preview_task(&task).expect("unban preview should build");

        assert_eq!(preview.method, "DELETE");
        assert_eq!(
            preview.route,
            "/guilds/123456789012345678/bans/987654321098765432"
        );
        assert_eq!(preview.bucket_key, "guild-ban-delete");
        assert_eq!(preview.guild_id, 123456789012345678);
        assert_eq!(preview.target_id, 987654321098765432);
    }

    #[test]
    fn preview_unban_all_reuses_unban_route_shape() {
        let task = build_task("unban_all");

        let preview = preview_task(&task).expect("unban_all preview should build");

        assert_eq!(preview.method, "DELETE");
        assert_eq!(
            preview.route,
            "/guilds/123456789012345678/bans/987654321098765432"
        );
        assert_eq!(preview.bucket_key, "guild-ban-delete");
    }

    #[test]
    fn preview_delete_message_accepts_numeric_channel_id_from_json() {
        let mut task = build_task("delete_message");
        task.task_context = json!({
            "channel_id": 555555555555555555u64
        });

        let preview = preview_task(&task).expect("delete_message preview should build");

        assert_eq!(preview.method, "DELETE");
        assert_eq!(
            preview.route,
            "/channels/555555555555555555/messages/987654321098765432"
        );
        assert_eq!(preview.bucket_key, "channel-message-delete");
    }

    #[test]
    fn preview_member_role_accepts_string_role_id_from_json() {
        let mut task = build_task("add_member_role");
        task.task_context = json!({
            "role_id": "222222222222222222"
        });

        let preview = preview_task(&task).expect("member role preview should build");

        assert_eq!(preview.method, "DELETE");
        assert_eq!(
            preview.route,
            "/guilds/123456789012345678/members/987654321098765432/roles/222222222222222222"
        );
        assert_eq!(preview.bucket_key, "guild-member-role-add");
    }
}
