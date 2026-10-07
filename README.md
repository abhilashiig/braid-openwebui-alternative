# Braid

A lightweight, self-hosted alternative to Open WebUI. Bring your own provider API keys (OpenAI, Anthropic, OpenRouter, Groq, Mistral, a local Ollama or vLLM, …); members chat with the models they are granted, and call every model through one OpenAI- and Anthropic-compatible API with personal keys.

- Rust backend (Axum, Tokio, sqlx/PostgreSQL) serving the API and the embedded SvelteKit frontend: one binary, one database.
- See [specification.md](specification.md) for requirements and [PREVIEW.md](PREVIEW.md) for what to try in the browser.

## Run with Docker

```sh
cat > .env <<EOF
BRAID_MASTER_KEY=$(openssl rand -base64 32)
POSTGRES_PASSWORD=$(openssl rand -hex 16)
BRAID_PUBLIC_URL=http://localhost:3000
EOF
docker compose up -d --build
docker compose logs braid | grep "setup token"
```

Open `BRAID_PUBLIC_URL`, enter the setup token from the logs, and create the admin account. Put Braid behind a TLS-terminating reverse proxy for anything beyond localhost, and set `BRAID_TRUST_PROXY=true` so client IPs come from `X-Forwarded-For`.

**Keep `BRAID_MASTER_KEY` safe.** It encrypts provider keys, skill keys and the SMTP password, and is never stored in the database. Losing it makes those secrets unreadable (re-enter them); leaking it together with a database dump exposes them.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `DATABASE_URL` | required | PostgreSQL connection string |
| `BRAID_MASTER_KEY` | required | 32 random bytes, base64 (`openssl rand -base64 32`) |
| `BRAID_PUBLIC_URL` | `http://localhost:3000` | Used in invitation and password-reset links; `https://` turns on Secure cookies |
| `BRAID_BIND` | `0.0.0.0:3000` | Listen address |
| `BRAID_SETUP_TOKEN` | random, logged | Fixed first-run setup token for headless deploys |
| `BRAID_TRUST_PROXY` | `false` | Trust `X-Forwarded-For` for client IPs |
| `BRAID_SECURE_COOKIES` | from public URL | Force Secure cookies on or off |
| `BRAID_LOG_FORMAT` | `json` | `json` or `pretty` |
| `RUST_LOG` | `braid=info` | Log filter |

These override what admins save in the UI (shown as such on Admin → Settings): `BRAID_INSTANCE_NAME`, `BRAID_SIGNUP_MODE` (`invite`/`open`), `BRAID_SIGNUP_DOMAINS`, `BRAID_ALLOW_API_KEYS`, `BRAID_ALLOWED_PRIVATE_HOSTS`, `BRAID_SMTP_HOST`, `BRAID_SMTP_PORT`, `BRAID_SMTP_USERNAME`, `BRAID_SMTP_PASSWORD`, `BRAID_SMTP_FROM`, `BRAID_SMTP_TLS` (`starttls`/`tls`/`none`).

## Upgrades, backup and restore

Database migrations run automatically at startup, so upgrading means pulling the new image and restarting.

Back up the database and the master key together:

```sh
docker compose exec -T db pg_dump -U braid -Fc braid > braid-$(date +%F).dump
```

Restore into a fresh install (same `BRAID_MASTER_KEY`), before Braid starts writing to it:

```sh
docker compose up -d db
docker compose exec -T db pg_restore -U braid -d braid --clean --if-exists < braid-2026-10-08.dump
docker compose up -d braid
```

## Development

Requirements: Rust (stable), Node 24, PostgreSQL.

```sh
createdb braid_development
cp backend/.env.example backend/.env   # set BRAID_MASTER_KEY
(cd backend && cargo run)              # API on :3000
(cd frontend && npm install && npm run dev)   # UI on :5173, proxies /api to :3000
```

Checks: `cargo test` in `backend/`, `npm run check` in `frontend/`.

## API

With API keys enabled for them, users create keys under Settings → API keys, then:

```python
from openai import OpenAI
client = OpenAI(base_url="https://braid.example.com/api/v1", api_key="sk-braid-…")
client.chat.completions.create(model="anthropic/claude-sonnet-4-5", messages=[{"role": "user", "content": "Hi"}])
```

The Anthropic SDK works the same way with `base_url="https://braid.example.com/api/anthropic"`. Any model works through either format.
