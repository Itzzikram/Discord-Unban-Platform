# Unbanall

Unbanall is an enterprise-focused Rust platform for Discord moderation orchestration at scale.

It combines a service-oriented control plane, Kafka-backed task delivery, Redis coordination, Postgres durability, and Twilight-based Discord execution for high-volume moderation workloads such as bulk unban, ban, kick, role changes, and message cleanup.

## Ownership

- Author: ItzzIkram
- Brand: DemonZ

## Highlights

- Distributed architecture with clear control, data, and operations planes.
- Durable command and task lifecycle persisted in Postgres.
- Redis-backed route and global rate-limit coordination.
- Kafka event backbone for commands, tasks, results, and audits.
- JWT + RBAC authorization and approval workflows for sensitive actions.
- Multi-region execution lease model to keep per-guild single-writer behavior.

## Architecture

- Control plane:
  - api-gateway
  - task-controller
  - audit-service
- Data plane:
  - queue-service
  - worker
  - shard-manager
  - rate-limit-service
  - cache-service
- Ops plane:
  - metrics-service
  - telemetry shared library
  - Kubernetes manifests and autoscaling definitions

Additional architecture detail is documented in docs/architecture.md.

## Repository Structure

```text
services/
  api-gateway/
  audit-service/
  cache-service/
  metrics-service/
  queue-service/
  rate-limit-service/
  shard-manager/
  task-controller/
  worker/
libs/
  auth/
  discord-client/
  kafka-runtime/
  platform-store/
  rate-limit/
  task-model/
  telemetry/
infra/
  postgres/
  kubernetes/
docs/
config.json
```

## Technology Stack

- Language: Rust (workspace monorepo)
- HTTP/API: Axum
- Messaging: Kafka (rdkafka)
- Cache/coordination: Redis
- Durable state: Postgres (sqlx)
- Discord integration: Twilight HTTP + Gateway
- Observability: Prometheus + tracing
- Deployment assets: Kubernetes + KEDA manifests

## Runtime Configuration

Core environment variables:

- DISCORD_TOKEN: bot token used by worker and shard-manager for live Discord calls.
- JWT_SECRET: HMAC secret used by api-gateway to validate bearer tokens.
- KAFKA_BROKERS: Kafka bootstrap servers.
- KAFKA_CONSUMER_GROUP: consumer group for worker instances.
- REDIS_URL: Redis connection string for distributed coordination.
- DATABASE_URL: Postgres connection string for commands, tasks, approvals, audits, and analytics.
- REGION: region identifier used for execution lease ownership.

Default development values are available in infra/kubernetes/configmap.yaml and sample request payloads are in config.json.

## Quick Start

1. Start dependencies (Kafka, Redis, Postgres) using your preferred local setup.
2. Export required environment variables.
3. Build the workspace:

```powershell
cargo check
```

4. Run services (example):

```powershell
cargo run -p api-gateway
cargo run -p queue-service
cargo run -p worker
```

5. Submit commands to api-gateway at /v1/commands.

## Unban Command Example

```json
{
  "tenant_id": "tenant-1",
  "guild_id": "123456789012345678",
  "action": "unban",
  "requested_by": "admin-user",
  "idempotency_key": "unban-req-001",
  "dry_run": false,
  "payload": {
    "targets": [
      "987654321098765432"
    ],
    "task_context": {}
  }
}
```

Notes:

- payload.targets is currently planned as string IDs.
- task_context supports JSON values and now accepts both string and numeric snowflake IDs for fields like channel_id and role_id where relevant.

## Testing

Run full workspace tests:

```powershell
cargo test
```

Run unban-focused tests:

```powershell
cargo test -p discord-client unban
```

## Security and Operations Guidance

- Never commit production secrets to source control.
- Rotate JWT and bot credentials regularly.
- Apply least-privilege RBAC for operators and service identities.
- Use approval workflows for destructive high-cardinality operations.
- Track route-level retry and rate-limit observations for tuning and capacity planning.

## Contributing

Contributions are welcome through issues and pull requests.

Recommended workflow:

1. Create a feature branch.
2. Keep changes scoped and well-tested.
3. Run cargo fmt, cargo check, and cargo test before opening a PR.
4. Include migration notes when schema changes are introduced.

## License

This project is licensed under the MIT License. See LICENSE for details.
