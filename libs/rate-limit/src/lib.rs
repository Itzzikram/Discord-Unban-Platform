use anyhow::Result;
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::env;

pub const GLOBAL_KEY: &str = "rl:global";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketState {
    pub key: String,
    pub remaining: u32,
    pub reset_at: DateTime<Utc>,
}

impl BucketState {
    pub fn is_available(&self, now: DateTime<Utc>) -> bool {
        self.remaining > 0 || now >= self.reset_at
    }
}

#[derive(Debug, Clone)]
pub struct RedisBackplane {
    client: redis::Client,
}

impl RedisBackplane {
    pub fn from_url(url: &str) -> Result<Self> {
        Ok(Self {
            client: redis::Client::open(url)?,
        })
    }

    pub fn from_env() -> Result<Option<Self>> {
        match env::var("REDIS_URL") {
            Ok(url) => Ok(Some(Self::from_url(&url)?)),
            Err(_) => Ok(None),
        }
    }

    pub async fn acquire(&self, key: &str, default_limit: u32, window_ms: i64) -> Result<BucketState> {
        let now = Utc::now();
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let remaining: Option<u32> = conn.hget(key, "remaining").await?;
        let reset_at_ms: Option<i64> = conn.hget(key, "reset_at_ms").await?;

        let reset_at = reset_at_ms
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .unwrap_or_else(|| now + chrono::TimeDelta::milliseconds(window_ms));

        let current_remaining = if now >= reset_at {
            default_limit
        } else {
            remaining.unwrap_or(default_limit)
        };

        let next_remaining = current_remaining.saturating_sub(1);
        let next_reset = if now >= reset_at {
            now + chrono::TimeDelta::milliseconds(window_ms)
        } else {
            reset_at
        };

        let _: () = conn
            .hset_multiple(
                key,
                &[
                    ("remaining", next_remaining as i64),
                    ("reset_at_ms", next_reset.timestamp_millis()),
                ],
            )
            .await?;
        let _: bool = conn.expire(key, ((window_ms / 1000).max(1)) as i64).await?;

        Ok(BucketState {
            key: key.to_string(),
            remaining: current_remaining,
            reset_at: next_reset,
        })
    }

    pub async fn update_retry_after(&self, key: &str, retry_after_ms: u64) -> Result<BucketState> {
        let reset_at = Utc::now() + chrono::TimeDelta::milliseconds(retry_after_ms as i64);
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let _: () = conn
            .hset_multiple(
                key,
                &[("remaining", 0_i64), ("reset_at_ms", reset_at.timestamp_millis())],
            )
            .await?;
        let _: bool = conn.expire(key, ((retry_after_ms / 1000).max(1)) as i64).await?;

        Ok(BucketState {
            key: key.to_string(),
            remaining: 0,
            reset_at,
        })
    }

    pub async fn get_cache(&self, key: &str) -> Result<Option<String>> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let value: Option<String> = conn.get(key).await?;
        Ok(value)
    }
}

pub fn route_key(route: &str) -> String {
    format!("rl:route:{route}")
}

pub fn guild_lease_key(guild_id: &str) -> String {
    format!("guild:lease:{guild_id}")
}

pub fn user_profile_key(user_id: &str) -> String {
    format!("user:{user_id}:profile")
}
