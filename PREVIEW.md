# What you can see in the browser

Updated as features land. Newest first.

## Running it

Two terminals, from the repo root:

```sh
# 1. Backend (Rust) on :3000, uses the braid_development database
cd backend && cargo run

# 2. Frontend (Vite dev server) on :5173, proxies /api to :3000
cd frontend && npm run dev
```

Open **http://localhost:5173**.

- The setup token is `BRAID_SETUP_TOKEN` in `backend/.env` (also printed in the backend log until setup is done).
- Config lives in `backend/.env`; see `backend/.env.example` for every option.
- Node 24 is required (Vite 8). `mise.local.toml` points mise at Homebrew's Node 24 for this folder.
- Start over: `dropdb braid_development && createdb braid_development`, then restart the backend.

## Available now

### 1. First-run setup, sign-in, invitations

- **http://localhost:5173**: on an empty database every page redirects to `/setup`. Enter the setup token, your name, email and a password (12+ characters, checked against the Have I Been Pwned breach list).
- After setup you land signed in. Sign out, then sign back in at `/login`. Five wrong passwords lock the email for 15 minutes.
- `/setup` is disabled once the first admin exists.
