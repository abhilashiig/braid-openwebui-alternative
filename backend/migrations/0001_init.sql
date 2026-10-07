create table users (
    id uuid primary key default gen_random_uuid(),
    email text not null,
    name text not null,
    password_hash text,
    role text not null check (role in ('admin', 'user')),
    status text not null default 'active' check (status in ('active', 'deactivated')),
    must_reset_password boolean not null default false,
    allow_api_keys boolean,
    default_model_id uuid,
    created_at timestamptz not null default now(),
    last_active_at timestamptz
);
create unique index users_email_key on users (lower(email));

create table sessions (
    token_hash bytea primary key,
    user_id uuid not null references users on delete cascade,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    ip text,
    user_agent text
);
create index sessions_user_id on sessions (user_id);

create table groups (
    id uuid primary key default gen_random_uuid(),
    name text not null unique,
    description text not null default '',
    is_everyone boolean not null default false,
    allow_api_keys boolean,
    created_at timestamptz not null default now()
);
insert into groups (name, description, is_everyone) values ('Everyone', 'All active users', true);

create table group_members (
    group_id uuid not null references groups on delete cascade,
    user_id uuid not null references users on delete cascade,
    primary key (group_id, user_id)
);
create index group_members_user_id on group_members (user_id);

create table invitations (
    id uuid primary key default gen_random_uuid(),
    email text not null,
    role text not null check (role in ('admin', 'user')),
    group_ids uuid[] not null default '{}',
    token_hash bytea not null unique,
    invited_by uuid references users on delete set null,
    expires_at timestamptz not null,
    accepted_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz not null default now()
);

create table password_resets (
    token_hash bytea primary key,
    user_id uuid not null references users on delete cascade,
    expires_at timestamptz not null,
    used_at timestamptz
);

create table settings (
    key text primary key,
    value jsonb not null
);

create table providers (
    id uuid primary key default gen_random_uuid(),
    name text not null,
    slug text not null unique,
    api_format text not null check (api_format in ('openai', 'anthropic')),
    base_url text not null,
    headers jsonb not null default '{}',
    organization text,
    project text,
    key_strategy text not null default 'failover' check (key_strategy in ('failover', 'round_robin')),
    timeout_secs integer not null default 120,
    max_retries integer not null default 2,
    enabled boolean not null default true,
    created_at timestamptz not null default now()
);

create table provider_keys (
    id uuid primary key default gen_random_uuid(),
    provider_id uuid not null references providers on delete cascade,
    label text not null,
    ciphertext bytea not null,
    last4 text not null,
    position integer not null default 0,
    enabled boolean not null default true,
    healthy boolean not null default true,
    last_error text,
    created_at timestamptz not null default now()
);
create index provider_keys_provider_id on provider_keys (provider_id);

create table models (
    id uuid primary key default gen_random_uuid(),
    provider_id uuid not null references providers on delete cascade,
    upstream_id text not null,
    name text not null unique,
    display_name text not null,
    description text not null default '',
    visibility text not null default 'private' check (visibility in ('public', 'private')),
    enabled boolean not null default true,
    context_window integer,
    max_output_tokens integer,
    default_temperature real,
    system_prompt text,
    supports_vision boolean not null default false,
    supports_tools boolean not null default false,
    supports_reasoning boolean not null default false,
    supports_streaming boolean not null default true,
    price_input_per_m double precision,
    price_output_per_m double precision,
    forced_skills text[] not null default '{}',
    created_at timestamptz not null default now(),
    unique (provider_id, upstream_id)
);

-- One row grants a user or a group access to a model or a skill.
create table grants (
    id uuid primary key default gen_random_uuid(),
    model_id uuid references models on delete cascade,
    skill text,
    user_id uuid references users on delete cascade,
    group_id uuid references groups on delete cascade,
    created_at timestamptz not null default now(),
    check (num_nonnulls(model_id, skill) = 1),
    check (num_nonnulls(user_id, group_id) = 1),
    unique nulls not distinct (model_id, skill, user_id, group_id)
);
create index grants_user_id on grants (user_id);
create index grants_group_id on grants (group_id);

create table skills (
    name text primary key,
    enabled boolean not null default false,
    visibility text not null default 'public' check (visibility in ('public', 'private')),
    secret_ciphertext bytea,
    secret_last4 text,
    config jsonb not null default '{}'
);
insert into skills (name) values ('web_search');

create table chats (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references users on delete cascade,
    title text not null default 'New chat',
    system_prompt text,
    temperature real,
    max_tokens integer,
    skills text[] not null default '{}',
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
create index chats_user_id_updated on chats (user_id, updated_at desc);

create table messages (
    id uuid primary key default gen_random_uuid(),
    chat_id uuid not null references chats on delete cascade,
    role text not null check (role in ('user', 'assistant')),
    content text not null default '',
    attachments jsonb not null default '[]',
    reasoning text,
    tool_steps jsonb not null default '[]',
    sources jsonb not null default '[]',
    model_id uuid references models on delete set null,
    model_name text,
    metrics jsonb,
    error text,
    created_at timestamptz not null default now()
);
create index messages_chat_id on messages (chat_id, created_at);
create index messages_search on messages using gin (to_tsvector('simple', content));

create table api_keys (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references users on delete cascade,
    name text not null,
    prefix text not null,
    key_hash bytea not null unique,
    model_ids uuid[],
    allow_skills boolean not null default true,
    rate_limit_rpm integer,
    token_quota_month bigint,
    expires_at timestamptz,
    revoked_at timestamptz,
    last_used_at timestamptz,
    last_used_ip text,
    created_at timestamptz not null default now()
);
create index api_keys_user_id on api_keys (user_id);

create table usage_log (
    id bigserial primary key,
    created_at timestamptz not null default now(),
    source text not null check (source in ('chat', 'api', 'skill')),
    user_id uuid references users on delete set null,
    api_key_id uuid references api_keys on delete set null,
    chat_id uuid references chats on delete set null,
    model_id uuid references models on delete set null,
    provider_id uuid references providers on delete set null,
    model_name text,
    provider_name text,
    status integer not null,
    error text,
    input_tokens integer,
    output_tokens integer,
    cached_tokens integer,
    cache_write_tokens integer,
    reasoning_tokens integer,
    cost double precision,
    latency_ms integer,
    ttft_ms integer,
    output_tps real,
    prefill_tps real,
    metrics_source text
);
create index usage_log_created_at on usage_log (created_at);
create index usage_log_user_id on usage_log (user_id, created_at);
create index usage_log_chat_id on usage_log (chat_id);

create table audit_log (
    id bigserial primary key,
    created_at timestamptz not null default now(),
    actor_id uuid references users on delete set null,
    actor_email text,
    action text not null,
    target_type text not null,
    target_id text,
    details jsonb not null default '{}',
    ip text
);
create index audit_log_created_at on audit_log (created_at desc);
