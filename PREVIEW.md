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

### 2. Admin console (sign in as the admin)

- **Setup wizard steps 2–3** (`/welcome`, right after creating the admin): add your first provider (presets for OpenAI, Anthropic, OpenRouter, Groq, Together, Mistral, DeepSeek, Ollama, vLLM), test the connection, tick models to import, then name the instance, upload a logo, pick the default model and sign-up mode.
- **Admin → Providers** (`/admin/providers`): add, edit, delete providers; add, disable and delete keys (only the last 4 characters are ever shown); **Test connection**; **Fetch models** opens a searchable checklist to import, and lists models no longer offered upstream. A key rejected with 401/403 turns red, and requests fail over to the next key.
- **Admin → Models** (`/admin/models`): click the visibility badge to switch public/private (warns how many users lose access); click Enabled to disable; **Edit** for capabilities, limits, pricing, default system prompt and **grants to groups and users**.
- **Admin → Users** (`/admin/users`): search and filter; **Invite users** (paste emails or upload a CSV, choose role and groups) gives copyable single-use links. Open the invite link in a private window to accept it. Click a user to promote or demote, deactivate, force a password reset (gives a link), delete (keep or erase chats), set groups, and see **effective model access** with the reason for each model.
- **Admin → Groups**: create a group, add members, bulk-grant models. "Everyone" always includes all active users.
- **Admin → Settings** and **Admin → Audit log**: every admin change above is listed with who, what, when and IP.
- To try a local model, add an **Ollama** provider; the "Local server" box allows `localhost` past the private-network block.

### 1. First-run setup, sign-in, invitations

- **http://localhost:5173**: on an empty database every page redirects to `/setup`. Enter the setup token, your name, email and a password (12+ characters, checked against the Have I Been Pwned breach list).
- After setup you land signed in. Sign out, then sign back in at `/login`. Five wrong passwords lock the email for 15 minutes.
- `/setup` is disabled once the first admin exists.
