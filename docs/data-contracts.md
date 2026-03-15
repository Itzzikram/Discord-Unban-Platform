# Data Contracts

## Kafka topics

| Topic | Purpose | Partition key |
| --- | --- | --- |
| `platform.commands` | high-level operator commands | `guild_id` |
| `platform.tasks` | executable tasks | `guild_id` |
| `platform.results` | task outcomes | `guild_id` |
| `platform.audit` | audit trail events | `command_id` |
| `platform.events` | gateway and domain events | `guild_id` |
| `platform.metrics` | operational metric snapshots | `service` |
| `platform.dlq` | poison tasks and exhausted retries | `guild_id` |

## Redis keys

| Key | Example | Purpose |
| --- | --- | --- |
| `rl:global` | `rl:global` | shared global limiter |
| `rl:route:{bucket}` | `rl:route:guild-ban-delete` | per-route bucket tracking |
| `guild:lease:{guild_id}` | `guild:lease:123` | single-writer guard |
| `shard:owner:{shard_id}` | `shard:owner:17` | shard coordination |
| `guild:{guild_id}:members` | `guild:123:members` | cached guild member data |
| `guild:{guild_id}:channels` | `guild:123:channels` | cached channel data |
| `user:{user_id}:profile` | `user:456:profile` | cached user profile |

## Prometheus metrics

- `requests_total`
- `requests_failed_total`
- `rate_limit_hits_total`
- `worker_count`
- `queue_size`
- `queue_lag`
- `discord_gateway_events_total`

## Postgres tables

### `commands`

- `id uuid primary key`
- `tenant_id text not null`
- `guild_id text not null`
- `action text not null`
- `requested_by text not null`
- `idempotency_key text not null unique`
- `status text not null`
- `approval_required boolean not null`
- `created_at timestamptz not null`

### `approvals`

- `id uuid primary key`
- `command_id uuid not null`
- `tenant_id text not null`
- `approved_by text null`
- `status text not null`
- `reason text null`
- `created_at timestamptz not null`
- `updated_at timestamptz not null`

### `tasks`

- `id uuid primary key`
- `command_id uuid not null`
- `guild_id text not null`
- `task_type text not null`
- `target_id text not null`
- `status text not null`
- `attempt_count int not null default 0`
- `next_retry_at timestamptz null`
- `created_at timestamptz not null`
- `requested_by text not null`
- `idempotency_key text not null unique`
- `lease_region text null`

### `task_results`

- `id bigserial primary key`
- `task_id uuid not null`
- `command_id uuid not null`
- `guild_id text not null`
- `route text not null`
- `success boolean not null`
- `status_code int null`
- `details text not null`
- `retryable boolean not null`
- `completed_at timestamptz not null`

### `audit_events`

- `id uuid primary key`
- `tenant_id text not null`
- `command_id uuid not null`
- `task_id uuid null`
- `event_type text not null`
- `payload jsonb not null`
- `created_at timestamptz not null`

### `regional_execution_leases`

- `guild_id text primary key`
- `region text not null`
- `expires_at timestamptz not null`
