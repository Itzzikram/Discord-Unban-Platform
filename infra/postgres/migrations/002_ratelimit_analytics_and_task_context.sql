alter table commands
    add column if not exists dry_run boolean not null default false,
    add column if not exists payload jsonb not null default '{}'::jsonb;

alter table tasks
    add column if not exists task_context jsonb not null default '{}'::jsonb;

alter table task_results
    add column if not exists bucket_key text,
    add column if not exists retry_after_ms bigint;

update task_results
set bucket_key = coalesce(bucket_key, route)
where bucket_key is null;

alter table task_results
    alter column bucket_key set not null;

create table if not exists route_ratelimit_observations (
    id bigserial primary key,
    command_id uuid not null references commands(id) on delete cascade,
    task_id uuid not null references tasks(id) on delete cascade,
    guild_id text not null,
    route text not null,
    bucket_key text not null,
    status_code int,
    retry_after_ms bigint,
    is_global boolean not null default false,
    observed_at timestamptz not null
);

create index if not exists idx_ratelimit_route_observed_at on route_ratelimit_observations(route, observed_at desc);