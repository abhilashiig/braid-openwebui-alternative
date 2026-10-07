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

### 5. Email, Docker, end-to-end check

- **Admin → Settings → Outbound email (SMTP)**: host, port, security, username, password, From address, and **Send test**. With SMTP set, invitations and admin-forced resets are emailed (links still shown), and the sign-in page offers **Forgot your password?** (`/forgot`).
- **Docker**: `docker compose up -d --build` runs Braid plus PostgreSQL on http://localhost:3000 (see README.md; needs a `.env` with `BRAID_MASTER_KEY`).
- **Acceptance check**: `python3 scripts/e2e.py` (after `cargo build` in `backend/`) spins up a fresh database and a mock provider and checks the spec's acceptance criteria, including a scan that no provider key leaks into responses or logs.

### 4. API gateway, API keys, skills, usage

- **Admin → Settings**: tick **Allow all users to create platform API keys** (or allow it per group or per user).
- **Settings** (sidebar, `/settings`): profile, default model, metrics toggle, password change, **API keys** (create with optional expiry, model limit, rate limit and monthly token quota; the full key is shown once), and your usage for the last 30 days.
- **Settings → How to use the API** (`/settings/api`): base URLs and copy-ready curl, OpenAI SDK and Anthropic SDK examples for this instance. Any model works through either format; tested with the official `openai` and `anthropic` Python SDKs, streaming and tools included.
  ```sh
  curl http://localhost:5173/api/v1/models -H "Authorization: Bearer sk-braid-…"
  ```
- **Admin → Skills**: enable web search, paste a Brave Search API key, **Test**, and choose everyone or granted groups/users.
- **Admin → Usage**: requests, errors, tokens, cache rate, average speed and TTFT, and cost, grouped by day, user, model, provider or API key, filtered to chat or API traffic; **Export CSV**.
- **Admin → API keys**: every user's keys, with revoke. Revoking a key or deactivating a user blocks their next call.

### 3. Chat (any signed-in user)

- **http://localhost:5173**: pick a model (top left; only models you can access are listed) and send a message. Replies stream in, with Markdown, highlighted code blocks (with Copy), tables and LaTeX math (`$E=mc^2$`, `$$…$$`).
- **Under each reply**: output speed, prefill speed, time to first token, and input/output tokens. A `*` means measured by Braid, including network latency; hover for details. **details** shows cached tokens, cache writes and cost (when the model has pricing set).
- **Chat header**: total tokens for the session, cached tokens, cache hit rate and cost. **⋯** opens chat settings (system prompt, temperature, max tokens, hide metrics) and Markdown/JSON export.
- **Stop** (or Esc) mid-stream, **Regenerate** the last reply, **Edit** an earlier message to re-run from there, switch model mid-chat (each reply shows its model).
- Thinking models show a collapsible **Thought process**.
- **Attach** images (vision models) or text/code files via 📎, paste or drag-and-drop. PDFs are not supported yet.
- **Sidebar**: chats grouped by date; search by title or message content; rename ✎ or delete 🗑 on hover.
- **Web search**: an admin enables it under **Admin → Skills** with a Brave Search API key. Then the 🌐 toggle appears for tool-capable models, and replies show "Searched: …" steps and clickable source chips.
- Light/dark: **Theme** in the sidebar cycles system → light → dark. Works on phone-sized screens (☰ opens the sidebar).

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
