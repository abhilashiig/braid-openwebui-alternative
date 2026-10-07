alter table users add column totp_secret bytea;
alter table users add column totp_enabled boolean not null default false;
-- Last accepted 30-second step, so a code cannot be replayed.
alter table users add column totp_last_step bigint;
