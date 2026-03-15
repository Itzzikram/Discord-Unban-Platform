<div align="center">
  <h1>🛡️ Unbanall</h1>
  <p><b>Enterprise-Grade Discord Moderation & Orchestration Platform</b></p>
  <p><i>Crafted with ❤️ by <b>ItzzIkram</b> | A <b>DemonZ</b> Project</i></p>
</div>

---

## 📖 Overview

**Unbanall** is a high-performance, distributed task platform engineered for massive-scale Discord server moderation. Written entirely in **Rust**, it is designed to safely execute high-volume workflows—such as bulk unbans, mass kicks, role management, and message cleanup—without hitting rate limits or dropping tasks.

By leveraging a robust stack of **Kafka**, **Redis**, **PostgreSQL**, and **Twilight**, Unbanall provides guarantees around durability, idempotency, and distributed rate-limit coordination.

---

## ✨ Highlights & Features

- 🏗️ **Distributed Microservices**: Clean separation of control, data, and operations planes.
- 💾 **Durable & Resilient**: Command and task lifecycles are persisted in PostgreSQL. No lost tasks.
- 🚦 **Global Rate Limiting**: Redis-backed coordination ensures Discord API limits are respected system-wide.
- 📨 **Event-Driven Backbone**: Kafka handles commands, tasks, task results, and audit logging.
- 🔐 **Zero-Trust Security**: JWT authentication + RBAC authorization for sensitive operations.
- 🌍 **Multi-Region Execution**: Lease models guarantee per-guild single-writer behavior across regions.
- ✅ **Approval Workflows**: Built-in human-in-the-loop approvals for highly destructive actions (like unban_all).

---

## 🏛️ Architecture

Unbanall is divided into three distinct operational planes:

### 1. Control Plane
Handles ingress, validation, and metadata.
- **pi-gateway**: Ingress API for commands and approvals.
- **	ask-controller**: Task planning and scheduling.
- **udit-service**: Immutable audit log intake.

### 2. Data Plane
Handles execution, queueing, and Discord interactions.
- **queue-service**: Manages Kafka publishing and replay logic.
- **worker**: Fetches tasks, manages rate limits, and executes Discord actions using the Twilight HTTP/Gateway clients.
- **shard-manager**: Dynamic shard provisioning based on Discord recommendations.
- **ate-limit-service** & **cache-service**: Centralized state management.

### 3. Ops Plane
Handles observability and autoscaling.
- **metrics-service**: Exposes Prometheus metrics.
- **	elemetry** library: Unified 	racing logs.
- **Kubernetes / KEDA**: Deployment manifests and event-driven autoscaling logic.

*For a deeper dive, check out [docs/architecture.md](docs/architecture.md).*

---

## 📂 Repository Structure

`	ext
unbanall/
├── services/       # Microservices (api-gateway, worker, queue-service, etc.)
├── libs/           # Shared crates (auth, discord-client, platform-store, etc.)
├── infra/          # Postgres migrations and Kubernetes/KEDA manifests
├── docs/           # Architecture and data contract documentation
├── config.json     # Sample request payloads and local config
└── Cargo.toml      # Root Rust workspace configuration
`

---

## 🚀 Getting Started

### Prerequisites

- Rust (Edition 2021)
- PostgreSQL
- Redis
- Kafka (e.g., Redpanda or local cluster)

### 1. Environment Configuration

Set the following environment variables (or rely on local defaults). See infra/kubernetes/configmap.yaml for reference.

`env
DISCORD_TOKEN="YOUR_DISCORD_BOT_TOKEN"
JWT_SECRET="development-secret"
KAFKA_BROKERS="localhost:9092"
KAFKA_CONSUMER_GROUP="worker-group"
REDIS_URL="redis://localhost:6379"
DATABASE_URL="postgres://platform:platform@localhost:5432/platform"
REGION="us-east-1"
`

### 2. Build the Workspace

`ash
cargo check
`

### 3. Run Core Services

Launch the minimum required services in separate terminal instances:

`ash
cargo run -p api-gateway
cargo run -p queue-service
cargo run -p worker
`

---

## 💻 Usage Example: Bulk Unban

Submit a command to the pi-gateway (POST /v1/commands). Ensure you pass your Operator JWT in the Authorization: Bearer <token> header.

`json
{
  "tenant_id": "tenant-1",
  "guild_id": "123456789012345678",
  "action": "unban",
  "requested_by": "admin-user",
  "idempotency_key": "unban-req-001",
  "dry_run": false,
  "payload": {
    "targets": ["987654321098765432"],
    "task_context": {}
  }
}
`
*Note: 	argets expects an array of stringified Discord Snowflakes. 	ask_context is used for conditional parameters like channel_id or ole_id.*

---

## 🧪 Testing

Unbanall includes comprehensive testing for correct task planning, authorization extraction, and Discord execution mapping.

`ash
# Run the full workspace test suite
cargo test

# Run specific module tests (e.g., Discord client unban mapping)
cargo test -p discord-client unban
`

---

## 🛡️ Security & Operations

- **Secret Management**: Never commit DISCORD_TOKEN or JWT_SECRET. 
- **RBAC**: Apply least-privilege principles to JWT payloads to tightly scope operator access.
- **Human In The Loop**: Use approval workflows for destructive high-cardinality operations.
- **Analytics**: Track route-level retry and rate-limit observations for tuning and capacity planning.

---

## 🤝 Contributing

Contributions are always welcome! 

1. Create a feature branch (git checkout -b feature/amazing-feature).
2. Ensure code formatting and tests pass (cargo fmt && cargo test).
3. Open a Pull Request.

---

## 📄 License

This project is licensed under the **MIT License**. Copyright (c) 2026 **ItzzIkram (DemonZ)**. See the [LICENSE](LICENSE) file for details.
