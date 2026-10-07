-- Prompt/response content for API requests, only written when the admin enables content logging.
alter table usage_log add column content jsonb;
create index usage_log_api_key_id on usage_log (api_key_id, created_at) where api_key_id is not null;
