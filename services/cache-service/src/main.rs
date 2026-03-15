use axum::{extract::Path, routing::get, Json, Router};
use std::{env, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("cache-service");

    let port = env::var("PORT").unwrap_or_else(|_| "8086".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/cache/guilds/{guild_id}/members", get(guild_members_key))
        .route("/v1/cache/guilds/{guild_id}/channels", get(guild_channels_key))
        .route("/v1/cache/users/{user_id}/profile", get(user_profile_key))
        .layer(TraceLayer::new_for_http());

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "cache-service", "status": "ok", "backend": "redis" }))
}

async fn guild_members_key(Path(guild_id): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "redis_key": format!("guild:{guild_id}:members") }))
}

async fn guild_channels_key(Path(guild_id): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "redis_key": format!("guild:{guild_id}:channels") }))
}

async fn user_profile_key(Path(user_id): Path<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "redis_key": format!("user:{user_id}:profile") }))
}