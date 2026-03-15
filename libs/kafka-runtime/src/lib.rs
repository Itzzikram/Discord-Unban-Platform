use anyhow::{anyhow, Context, Result};
use rdkafka::{
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
    message::BorrowedMessage,
    producer::{FutureProducer, FutureRecord},
    Message,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{env, time::Duration};

#[derive(Debug, Clone)]
pub struct KafkaConfig {
    pub brokers: String,
    pub consumer_group: String,
    pub lag_scale_up_threshold: i64,
    pub lag_scale_out_threshold: i64,
}

impl KafkaConfig {
    pub fn from_env(service_name: &str) -> Self {
        Self {
            brokers: env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:9092".to_string()),
            consumer_group: env::var("KAFKA_CONSUMER_GROUP")
                .unwrap_or_else(|_| format!("{service_name}-group")),
            lag_scale_up_threshold: env::var("KAFKA_LAG_SCALE_UP_THRESHOLD")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1_000),
            lag_scale_out_threshold: env::var("KAFKA_LAG_SCALE_OUT_THRESHOLD")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(10_000),
        }
    }
}

pub fn create_producer(config: &KafkaConfig) -> Result<FutureProducer> {
    ClientConfig::new()
        .set("bootstrap.servers", &config.brokers)
        .set("message.timeout.ms", "5000")
        .create()
        .context("failed to create Kafka producer")
}

pub fn create_consumer(config: &KafkaConfig, topics: &[&str]) -> Result<StreamConsumer> {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &config.brokers)
        .set("group.id", &config.consumer_group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .create()
        .context("failed to create Kafka consumer")?;

    consumer
        .subscribe(topics)
        .context("failed to subscribe Kafka consumer")?;

    Ok(consumer)
}

pub async fn publish_json<T: Serialize>(
    producer: &FutureProducer,
    topic: &str,
    key: &str,
    payload: &T,
) -> Result<()> {
    let payload = serde_json::to_string(payload)?;
    let delivery = producer
        .send(
            FutureRecord::to(topic).payload(&payload).key(key),
            Duration::from_secs(5),
        )
        .await;

    match delivery {
        Ok(_) => Ok(()),
        Err((error, _)) => Err(anyhow!(error)).context("failed to publish Kafka message"),
    }
}

pub fn decode_json<T: DeserializeOwned>(message: &BorrowedMessage<'_>) -> Result<T> {
    let payload = message
        .payload_view::<str>()
        .transpose()?
        .ok_or_else(|| anyhow!("Kafka message payload was empty"))?;

    serde_json::from_str(payload).context("failed to deserialize Kafka payload")
}

pub fn commit_message(consumer: &StreamConsumer, message: &BorrowedMessage<'_>) -> Result<()> {
    consumer
        .commit_message(message, CommitMode::Async)
        .context("failed to commit Kafka offset")
}