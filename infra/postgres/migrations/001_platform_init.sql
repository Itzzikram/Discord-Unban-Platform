create extension if not exists pgcrypto;

create table if not exists commands (
    id uuid primary key,
    tenant_id text not null,
    guild_id text not null,
    action text not null,
    requested_by text not null,
    idempotency_key text not null unique,
    dry_run boolean not null default false,
    payload jsonb not null default '{}'::jsonb,
    status text not null,
    approval_required boolean not null default false,
    created_at timestamptz not null
);

create table if not exists approvals (
    command_id uuid not null references commands(id) on delete cascade,
    id uuid not null default gen_random_uuid(),
    tenant_id text not null,
    approved_by text,
    status text not null,
    reason text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    primary key (command_id),
    unique (id)
);

create table if not exists tasks (
    id uuid primary key,
    command_id uuid not null references commands(id) on delete cascade,
    guild_id text not null,
    task_type text not null,
    target_id text not null,
    status text not null,
    attempt_count int not null default 0,
    next_retry_at timestamptz,
    created_at timestamptz not null,
    requested_by text not null,
    idempotency_key text not null unique,
    lease_region text
);

create table if not exists task_results (
    id bigserial primary key,
    task_id uuid not null references tasks(id) on delete cascade,
    command_id uuid not null references commands(id) on delete cascade,
    guild_id text not null,
    route text not null,
    success boolean not null,
    status_code int,
    details text not null,
    retryable boolean not null,
    completed_at timestamptz not null
);

create table if not exists audit_events (
    id uuid primary key,
    tenant_id text not null,
    command_id uuid not null references commands(id) on delete cascade,
    task_id uuid references tasks(id) on delete cascade,
    event_type text not null,
    payload jsonb not null,
    created_at timestamptz not null
);

create table if not exists regional_execution_leases (
    guild_id text primary key,
    region text not null,
    expires_at timestamptz not null
);

create index if not exists idx_tasks_command_id on tasks(command_id);
create index if not exists idx_tasks_guild_status on tasks(guild_id, status);
create index if not exists idx_task_results_command_id on task_results(command_id);
create index if not exists idx_audit_events_command_id on audit_events(command_id);