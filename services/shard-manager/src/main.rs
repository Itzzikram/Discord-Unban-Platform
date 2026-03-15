use axum::{routing::get, Json, Router};
use discord_client::{build_runtime_config, create_http_client, recommended_intents};
use std::{env, net::SocketAddr};
use tower_http::trace::TraceLayer;
use tracing::info;
use twilight_gateway::{create_recommended, ConfigBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init("shard-manager");

    let port = env::var("PORT").unwrap_or_else(|_| "8082".to_string());
    let address: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/shards", get(list_shards))
        .layer(TraceLayer::new_for_http());

    info!(%address, "starting service");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "service": "shard-manager", "status": "ok" }))
}

async fn list_shards() -> Json<serde_json::Value> {
    let shard_count = env::var("DISCORD_SHARD_COUNT")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(16);
    let config = build_runtime_config(shard_count);

    let (recommended_shards, bootstrapped) = match env::var("DISCORD_TOKEN") {
        Ok(token) => match fetch_recommended_shards(token).await {
            Ok(value) => (value, true),
            Err(_) => (config.shard_count, false),
        },
        Err(_) => (config.shard_count, false),
    };

    let shards = plan_shards(recommended_shards, &["us", "eu", "asia"]);

    Json(serde_json::json!({
        "gateway": "twilight-gateway",
        "strategy": "guild affinity",
        "bootstrapped": bootstrapped,
        "intents_bits": config.intents_bits,
        "shard_count": recommended_shards,
        "shards": shards
    }))
}

async fn fetch_recommended_shards(token: String) -> anyhow::Result<u32> {
    let http = create_http_client(token.clone());
    let config = ConfigBuilder::new(token, recommended_intents()).build();
    let shards = create_recommended(&http, config, |_, builder| builder.build()).await?;
    Ok(shards.len() as u32)
}

fn plan_shards(shard_count: u32, regions: &[&str]) -> Vec<serde_json::Value> {
    (0..shard_count)
        .map(|shard_id| {
            let region = regions[(shard_id as usize) % regions.len()];

            serde_json::json!({
                "id": shard_id,
                "region": region,
                "transport": "twilight-gateway",
            })
        })
        .collect()
}
