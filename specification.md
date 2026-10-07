# Braid — Requirements Specification

Oct 7, 2026 · @Abhilash

## 1. Introduction

Braid ("the platform" below) is a lightweight, self-hosted alternative to Open WebUI for organizations: a multi-user web app where the organization brings its own AI provider API keys (BYOK), and members chat with those models through one interface or call them through one API.

It keeps the Open WebUI features most organizations actually use, drops the long tail, and is built for speed: a compiled backend and a small, fast frontend, so one modest server runs the whole organization. The name reflects the core idea: many providers braided into one chat app and one API.

**Purpose.** This spec defines what version 1 must do, so design, build and test work from one shared list of requirements.

**Scope relative to Open WebUI**

| Open WebUI capability | This platform |
| --- | --- |
| Multiple OpenAI-compatible connections | v1, plus native Anthropic API format |
| Users, groups, roles | v1 |
| Per-model access control | v1 (public/private plus user and group grants) |
| Web search | v1 with Brave; more search providers later |
| OpenAI-compatible API with user keys | v1: one API across all providers, OpenAI- and Anthropic-compatible |
| SSO (OIDC, LDAP) | Phase 3 |
| Document chat and knowledge bases (RAG) | Phase 3 candidate |
| Tools via MCP servers | Phase 3 candidate |
| Python Functions and Pipelines plugins | Not planned; replaced by a typed skill framework |
| Image generation, voice, Ollama model pulling | Not planned for v1 |

**In scope for v1:**

- First-run setup that creates the admin account
- User invitations, groups and role-based access
- Multiple AI providers, each with one or more API keys, in OpenAI-compatible or Anthropic format
- Model catalog per provider, added by hand or auto-fetched, marked public or private
- Model access granted to groups or individual users
- A chat UI with streaming, history and model switching
- An API gateway: users generate personal API keys and reach models from every provider through one unified API
- Skills (tools the model can call), starting with web search via Brave Search

**Out of scope for v1:** billing and payments, image or audio generation, fine-tuning, a mobile app, SSO (planned for a later phase), and multi-tenant hosting of several organizations in one install.

**Definitions**

| Term | Meaning |
| --- | --- |
| BYOK | Bring your own key: the organization supplies provider API keys; the platform never resells usage |
| Provider | A configured upstream AI service (e.g. OpenAI, Anthropic, OpenRouter, a local Ollama or vLLM server) with a base URL and API format |
| Provider key | A secret API key for a provider, stored encrypted by the platform |
| Model | A model ID exposed by a provider (e.g. gpt-4o, claude-sonnet-4-5) and registered in the platform |
| Public model | A model every active user can use without an explicit grant |
| Private model | A model only granted users or groups can use |
| Group | A named set of users that receives model access as a unit |
| Platform API key | A key a user generates to call the platform's own API; distinct from provider keys |
| Skill | A tool the model may call during a chat, such as web search |

## 2. User roles

Three roles cover v1: Admin, User, and API client (a program using a user's platform API key, acting with that user's permissions).

| Role | Who | Can do |
| --- | --- | --- |
| Admin | Created at first-run setup; more can be promoted later | Everything: providers, keys, models, users, groups, access grants, skills, settings, audit log, usage reports |
| User | Invited by an admin | Chat with models they can access; manage own chats, profile and platform API keys (if allowed) |
| API client | Software calling the platform API | Call permitted models and skills using a user's platform API key, within that key's scopes and limits |

- The system must always keep at least one active admin; demoting or deleting the last admin is blocked.
- Only org admins can add, edit or delete providers, API keys and models; regular users only use what they are granted, in chat or through their own platform API keys when the admin allows them. Roles are fixed in v1. Custom roles and fine-grained permissions are a later-phase item.

## 3. First-run setup

On a fresh install the app shows a setup wizard instead of the login page, and that wizard is the only way to create the first admin.

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-SET-01 | Detect an empty install (no users) and redirect every route to the setup wizard | Must |
| FR-SET-02 | Wizard step 1: create admin with name, email and password (min 12 characters, checked against a breached-password list) | Must |
| FR-SET-03 | Protect the wizard with a one-time setup token printed to server logs or set via environment variable, so a public instance can't be claimed by a stranger | Must |
| FR-SET-04 | Wizard step 2 (skippable): add the first provider and API key, test the connection, auto-fetch models | Must |
| FR-SET-05 | Wizard step 3 (skippable): instance name, logo, default model, whether self-signup by invite only (default) or open with domain allow-list | Should |
| FR-SET-06 | Disable the wizard permanently once the first admin exists | Must |
| FR-SET-07 | Allow outbound email (SMTP) configuration for invitations and password resets; if absent, show invite links for the admin to copy | Must |
| FR-SET-08 | Support configuration by environment variables for headless deploys (e.g. Docker), overriding wizard defaults | Should |

## 4. Users, groups and invitations

Admins invite users by email and organize them into groups; groups are the main unit for granting model access.

**Invitations and accounts**

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-USR-01 | Admin invites one or many users by email (bulk paste or CSV), choosing role and initial groups | Must |
| FR-USR-02 | Invitation link is single-use and expires after a configurable period (default 7 days); admin can resend or revoke | Must |
| FR-USR-03 | Invitee sets name and password on acceptance; the account is then active | Must |
| FR-USR-04 | Users can reset forgotten passwords by email; admins can force a reset | Must |
| FR-USR-05 | Admin can deactivate a user: sessions end, platform API keys stop working, chat history is kept | Must |
| FR-USR-06 | Admin can delete a user, with a choice to keep or erase their chats | Should |
| FR-USR-07 | Admin can promote a user to admin or demote an admin (subject to the last-admin rule) | Must |
| FR-USR-08 | Optional TOTP two-factor authentication; admin can make it mandatory for admins or everyone | Should |
| FR-USR-09 | User list with search, filters (role, group, status) and last-active date | Must |

**Groups**

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-GRP-01 | Admin creates, renames and deletes groups, with an optional description | Must |
| FR-GRP-02 | A user can belong to any number of groups | Must |
| FR-GRP-03 | Admin adds and removes members from the group page or the user page | Must |
| FR-GRP-04 | Deleting a group removes its model grants; members keep access granted by other routes | Must |
| FR-GRP-05 | A built-in "Everyone" group contains all active users and cannot be deleted | Should |
| FR-GRP-06 | Group-level limits: token or request quotas per day/month, applied to each member | Could |

## 5. Providers and API keys

Admins can register any number of providers, each speaking either the OpenAI-compatible API format or the Anthropic Messages API format, and each holding one or more encrypted API keys.

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-PRV-01 | Admin adds a provider with: display name, API format (OpenAI-compatible or Anthropic), base URL, optional custom headers, optional organization/project ID | Must |
| FR-PRV-02 | Offer presets that pre-fill base URL and format: OpenAI, Anthropic, OpenRouter, Groq, Together, Mistral, DeepSeek, Ollama (local), vLLM/LM Studio (custom URL) | Should |
| FR-PRV-03 | Admin adds one or more API keys to a provider, each with a label; a key's value is write-only and shown afterwards only as its last 4 characters | Must |
| FR-PRV-04 | "Test connection" calls a cheap endpoint (model list, or a 1-token completion) and reports success, auth failure, or network error | Must |
| FR-PRV-05 | When a provider has several keys, choose one per request by a setting: primary-with-failover (default) or round-robin | Should |
| FR-PRV-06 | On a 401/403 from a key, mark it unhealthy, fail over to the next key, and notify admins | Should |
| FR-PRV-07 | Admin can enable, disable, edit and delete providers and keys; deleting a provider asks for confirmation and lists affected models | Must |
| FR-PRV-08 | Provider keys are encrypted at rest (AES-256-GCM with a master key from env or a KMS) and never sent to the browser or written to logs | Must |
| FR-PRV-09 | Per-provider request timeout and retry policy (default 120 s timeout, 2 retries on 429/5xx with backoff) | Should |
| FR-PRV-10 | Only admins can create, edit or delete providers and keys; these endpoints return 403 to regular users and the UI hides them | Must |

**API format support**

The platform keeps one internal message format and translates to and from each provider format.

| Capability | OpenAI-compatible | Anthropic |
| --- | --- | --- |
| Chat endpoint called | POST /v1/chat/completions | POST /v1/messages |
| Model list for auto-fetch | GET /v1/models | GET /v1/models |
| Auth header | Authorization: Bearer | x-api-key + anthropic-version |
| Streaming | SSE, delta chunks | SSE, typed events |
| System prompt | system role message | top-level system field |
| Tool calling (for skills) | tools / tool\_calls | tools / tool\_use blocks |
| Image input | image\_url parts | image content blocks |

FR-PRV-11 (Must): translation must preserve streaming, system prompts, multi-turn history, tool calls and image inputs in both directions. FR-PRV-12 (Could): support the OpenAI Responses API format as a third option.

## 6. Model management

Each model belongs to one provider, is added manually or by auto-fetch, and is set as public or private when it is added.

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-MDL-01 | Add a model manually: provider, model ID (as the provider expects it), display name, description | Must |
| FR-MDL-02 | Auto-fetch: call the provider's model list, show results as a checklist with search, and import the selected ones | Must |
| FR-MDL-03 | Choose visibility at add time: Public (all active users) or Private (only granted users/groups); default Private | Must |
| FR-MDL-04 | Visibility can be changed later; switching Public to Private keeps existing grants and warns how many users lose access | Must |
| FR-MDL-05 | Per-model settings: context window, max output tokens, default temperature, default system prompt | Should |
| FR-MDL-06 | Capability flags (vision, tool calling, reasoning, streaming), pre-filled from a known-model table and editable | Should |
| FR-MDL-07 | Optional price per 1M input and output tokens, used for cost estimates in usage reports | Should |
| FR-MDL-08 | Model alias: a stable platform name (e.g. "default-fast") that maps to a provider model and can be repointed without breaking clients | Could |
| FR-MDL-09 | Re-sync: re-run auto-fetch to flag models removed upstream and offer newly available ones; nothing is imported or deleted without confirmation | Should |
| FR-MDL-10 | Enable/disable a model without deleting it; disabled models disappear from pickers and return a clear API error | Must |
| FR-MDL-11 | The same upstream model may be registered under two providers (e.g. Claude direct and via OpenRouter) as separate models | Must |
| FR-MDL-12 | Admin sets one organization default model; users may set their own default among models they can access | Should |

Only admins add, edit and delete models; regular users can only use the models they are granted.

## 7. Model access control

A user can use a model if it is public, or if it is private and granted to the user directly or to any group they belong to; the same rule applies in chat and through the API.

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-ACL-01 | Admin grants a private model to any mix of groups and individual users, from the model page | Must |
| FR-ACL-02 | Admin sees and edits a user's or group's accessible models from the user or group page | Must |
| FR-ACL-03 | Bulk grant: assign several models to a group in one action | Should |
| FR-ACL-04 | Access is the union of all routes (public, direct grant, any group grant); there are no deny rules in v1 | Must |
| FR-ACL-05 | Revoking access takes effect on the next request; an in-flight stream may finish | Must |
| FR-ACL-06 | Model pickers and the API's model list show only models the caller can use | Must |
| FR-ACL-07 | A request for a model without access returns 403 (API) or a clear message (chat), never the provider's error | Must |
| FR-ACL-08 | Optional per-grant limits (requests/day, tokens/month) on a user or group | Could |
| FR-ACL-09 | "Effective access" view: for a user, list each model and why they have access (public, direct, or via group X) | Should |

**Access check order** (evaluated per request):

1. Model exists and is enabled, and its provider has a healthy key — else 404/503.
2. User is active — else 401.
3. Model is public — allow.
4. User has a direct grant — allow.
5. Any of the user's groups has a grant — allow.
6. Otherwise deny with 403.

## 8. Chat experience

Users get a familiar chat interface with streaming replies, saved history and a model picker limited to their permitted models.

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-CHT-01 | Start a new chat, pick a model from those accessible, send messages, receive streamed replies | Must |
| FR-CHT-02 | Render Markdown, code blocks with syntax highlighting and copy button, tables, and LaTeX math | Must |
| FR-CHT-03 | Stop generation mid-stream; regenerate the last reply; edit a previous user message and re-run from there | Must |
| FR-CHT-04 | Switch model mid-chat; each reply shows which model produced it | Must |
| FR-CHT-05 | Chat history in a sidebar: auto-generated titles, rename, delete, search by title and content | Must |
| FR-CHT-06 | Per-chat settings: system prompt, temperature, max tokens, enabled skills | Should |
| FR-CHT-07 | Attach images to models with vision; attach text/PDF files whose extracted text is added to the prompt | Should |
| FR-CHT-08 | Show token usage per reply (and estimated cost if pricing is set) | Should |
| FR-CHT-09 | Show reasoning/thinking output in a collapsible block for models that return it | Should |
| FR-CHT-10 | Saved prompts (personal prompt library) | Could |
| FR-CHT-11 | Export a chat as Markdown or JSON | Could |
| FR-CHT-12 | Share a read-only chat link with other users in the instance | Could |
| FR-CHT-13 | Responsive layout usable on mobile browsers; light and dark themes | Must |
| FR-CHT-14 | Provider errors (rate limit, context too long, outage) are shown in plain words with a retry option | Must |

**Response speed and cache metrics**

Every successful reply shows its generation speed and prefill speed, and each chat shows its total tokens consumed and prompt-cache hit rate, whenever the provider returns the data needed; metrics with no data are hidden, never shown as zero.

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-CHT-15 | Under each successful reply, show output speed (tok/s), prefill speed (tok/s) and time to first token, plus input and output token counts | Must |
| FR-CHT-16 | Use provider-reported timings when present (e.g. Ollama, llama.cpp, vLLM); otherwise compute from stream timestamps and reported token counts, and mark the value as measured | Must |
| FR-CHT-17 | Measured values include network latency; the tooltip says whether a value is provider-reported or measured | Should |
| FR-CHT-18 | In the chat header, show total tokens consumed in the session (input, output, cached; reasoning tokens when reported), estimated cost when pricing is set, and the cache hit rate when the provider reports cached tokens; totals update live after each reply | Must |
| FR-CHT-19 | Per reply, a details popover lists cached input tokens, cache-write tokens and estimated savings when pricing is set | Should |
| FR-CHT-20 | Store all metrics on the message and in the usage log, so the usage dashboard can show average speed and cache rate per model and provider | Must |
| FR-CHT-21 | Users can hide the metrics line in their settings; admins set the default | Could |

| Metric | How it is calculated | Data source |
| --- | --- | --- |
| Output speed | output tokens ÷ (last token time − first token time) | Provider timings, else stream timestamps + usage |
| Prefill speed | input tokens ÷ time to first token | Provider timings (e.g. prompt\_eval\_duration), else measured |
| Time to first token | first content chunk time − request sent time | Measured by the platform |
| Cache hit rate (session) | sum of cached input tokens ÷ sum of all input tokens, over the chat's replies | OpenAI: prompt\_tokens\_details.cached\_tokens; Anthropic: cache\_read\_input\_tokens (cache\_creation\_input\_tokens shown as writes) |
| Tokens consumed (session) | sum of input, output and cached tokens over every reply in the chat, including regenerated and tool-call turns | Provider usage fields (OpenAI usage, Anthropic usage) |

Streaming requests ask the provider for usage in the final chunk (e.g. OpenAI stream\_options.include\_usage) so token counts are available for these figures.

## 9. API gateway and user API keys

When an admin allows it, users generate their own platform API keys and the platform behaves like an inference provider: one base URL and one key reach every model the user may use, whichever upstream provider serves it.

This merges several providers (e.g. OpenAI, Anthropic, OpenRouter, a local vLLM server) behind one OpenAI-compatible and one Anthropic-compatible API. Provider keys stay with the admin; users and their apps only ever hold platform keys.

**Unified multi-provider API**

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-GW-01 | One base URL and one platform key reach all models the user may use, across every provider | Must |
| FR-GW-02 | API model names are unique across providers: admin-set platform names, defaulting to provider-slug/model-id (e.g. openai/gpt-4o, anthropic/claude-sonnet-4-5) | Must |
| FR-GW-03 | GET /api/v1/models merges models from all providers into one list, with owned\_by set to the provider name | Must |
| FR-GW-04 | Upstream provider keys are never exposed to users or API clients | Must |
| FR-GW-05 | Fallback routes: an alias (FR-MDL-08) lists several provider models in order; on 429, 5xx or timeout before the first token, the request moves to the next one | Should |
| FR-GW-06 | Load balancing: an alias spreads traffic by weight across equivalent models, e.g. the same model on two providers | Could |
| FR-GW-07 | Response headers name the provider and model that served the request, for debugging and cost tracking | Should |
| FR-GW-08 | Usage and cost reports break API traffic down by user, key, model and provider | Must |

**Enablement and key management**

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-API-01 | Global setting "Allow user API keys" (default off); can also be enabled per group or per user | Must |
| FR-API-02 | User creates a key with a name and optional expiry; the full key is shown once, then only its prefix (e.g. sk-braid-ab12…) | Must |
| FR-API-03 | Keys are stored as salted hashes, never in plain text | Must |
| FR-API-04 | Optional key scopes: restrict to a subset of the user's models, and to chat/skills | Should |
| FR-API-05 | Per-key rate limits (requests/minute) and quotas (tokens/month), capped by admin-set maximums | Should |
| FR-API-06 | Users revoke their own keys; admins see and revoke any key | Must |
| FR-API-07 | Max keys per user, set by admin (default 5) | Should |
| FR-API-08 | Key last-used time and IP shown to the user | Should |

**Endpoints**

| Endpoint | Format | Notes |
| --- | --- | --- |
| GET /api/v1/models | OpenAI | Lists only models the key can use |
| POST /api/v1/chat/completions | OpenAI | Streaming and non-streaming, tools supported |
| POST /api/anthropic/v1/messages | Anthropic | Same models, Anthropic request/response shape |
| GET /api/v1/usage | Platform | Caller's own usage by day and model |

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-API-09 | Any model can be called through either endpoint format, regardless of its provider's format (translation per section 5) | Must |
| FR-API-10 | The model field takes the platform model name or alias, not the upstream ID, so admins can swap providers freely | Must |
| FR-API-11 | Errors use the format of the endpoint called (OpenAI error object or Anthropic error object) with correct HTTP codes | Must |
| FR-API-12 | Each request is logged with user, key, model, tokens in/out, latency and status — not prompt content unless content logging is enabled | Must |
| FR-API-13 | API usage counts toward the same user and group quotas as chat | Must |
| FR-API-14 | Built-in API docs page with copy-ready examples (curl, Python OpenAI SDK, Anthropic SDK) using the instance's base URL | Should |
| FR-API-15 | Embeddings endpoint (/v1/embeddings) for providers that support it | Could |

## 10. Skills (tools)

Skills are tools a model can call during a chat; v1 ships one skill, web search backed by the Brave Search API, on a framework that lets more skills be added later.

**Skill framework**

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-SKL-01 | Skills are defined by name, description, JSON input schema and a server-side handler, and are offered to the model as tools | Must |
| FR-SKL-02 | Admin enables or disables each skill globally and configures its secrets (e.g. Brave key, encrypted like provider keys) | Must |
| FR-SKL-03 | Admin grants skills to groups or users, using the same public/private model as models | Should |
| FR-SKL-04 | Skills are offered only to models flagged as supporting tool calling | Must |
| FR-SKL-05 | User toggles skills per chat; admin can force-enable a skill for a model | Should |
| FR-SKL-06 | The tool loop runs server-side: model calls tool, platform runs it, result goes back, repeated up to a limit (default 5 calls per reply) | Must |
| FR-SKL-07 | Chat UI shows each tool call as a collapsible step ("Searched: …") with its results | Must |
| FR-SKL-08 | Skills also work through the API gateway when the client sends a flag (e.g. platform\_skills: \["web\_search"\]) | Could |

**Web search (Brave)**

| ID | Requirement | Priority |
| --- | --- | --- |
| FR-WEB-01 | Admin adds a Brave Search API key and tests it | Must |
| FR-WEB-02 | Tool input: query, optional count (default 5, max 20), optional freshness filter (day/week/month/year) | Must |
| FR-WEB-03 | Tool returns title, URL, snippet and date per result; the model is instructed to cite sources | Must |
| FR-WEB-04 | Answers show clickable source citations under the reply | Must |
| FR-WEB-05 | Optional page fetch: retrieve and extract the text of a result URL for deeper answers, with size cap and SSRF protection | Should |
| FR-WEB-06 | Searches are rate-limited and counted in usage reports per user | Should |
| FR-WEB-07 | Search provider is behind an interface so others (Tavily, SearXNG, Google PSE) can be added later | Should |

## 11. Non-functional requirements

Because the platform holds the organization's provider keys and conversations, security and auditability come first, followed by low added latency on the request path.

| ID | Area | Requirement | Priority |
| --- | --- | --- | --- |
| NFR-SEC-01 | Security | All secrets (provider keys, skill keys, SMTP password) encrypted at rest; master key outside the database | Must |
| NFR-SEC-02 | Security | Passwords hashed with Argon2id; platform API keys hashed | Must |
| NFR-SEC-03 | Security | HTTPS only; secure, HttpOnly, SameSite cookies; CSRF protection; strict CSP | Must |
| NFR-SEC-04 | Security | Login rate limiting and lockout after repeated failures | Must |
| NFR-SEC-05 | Security | Admin-only endpoints enforced server-side; every request checks role and model access | Must |
| NFR-SEC-06 | Security | Outbound requests (custom base URLs, page fetch) block private IP ranges unless the admin allow-lists them (needed for local Ollama) | Must |
| NFR-AUD-01 | Audit | Audit log of admin actions: provider/key/model changes, grants, user and role changes, settings; who, what, when, from where | Must |
| NFR-AUD-02 | Audit | Usage log per request (user, key, model, tokens, cost estimate, latency, status), with a dashboard and CSV export | Must |
| NFR-AUD-03 | Privacy | Chat content stored only for the owning user; admins cannot read chats unless a compliance setting is enabled and disclosed to users | Should |
| NFR-AUD-04 | Privacy | Configurable retention for chats and logs; user can delete their data | Should |
| NFR-PRF-01 | Performance | Added latency before the first streamed token under 20 ms at p95, excluding provider time | Must |
| NFR-PRF-02 | Performance | Support 1,000 concurrent streaming chats on a 2 vCPU / 4 GB server | Should |
| NFR-REL-01 | Reliability | Provider failure affects only that provider's models; app stays up | Must |
| NFR-REL-02 | Reliability | Health endpoint and structured JSON logs; optional Prometheus metrics | Should |
| NFR-OPS-01 | Deployment | Single Docker image plus docker-compose with PostgreSQL; SQLite allowed for small installs | Must |
| NFR-OPS-02 | Deployment | Database migrations run automatically on upgrade; backup and restore documented | Must |
| NFR-UX-01 | Usability | WCAG 2.1 AA for core flows; keyboard navigation in chat | Should |
| NFR-UX-02 | i18n | UI strings externalized for later translation; English in v1 | Should |

## 12. Data model

The access grant is the one table that decides who can use what: it links a user or a group to a model or a skill.

&#91;embedded content: core entities and how they relate\]

A model belongs to one provider, which holds one or more encrypted keys; chats, messages and platform API keys belong to one user. Every chat or API request writes a usage-log row, and every admin change writes an audit-log row.

## 13. Architecture and technology choices

The backend is a single Rust binary (Axum on Tokio) that serves the API, streams model output, and embeds the compiled SvelteKit frontend; one container plus a database is the whole deployment.

&#91;embedded content: request path through the backend\]

A request passes the HTTP layer and the access check, then a provider adapter translates it to the provider's format and streams tokens back over SSE. Usage and audit rows are batched off the request path.

**Stack decisions**

| Layer | Choice | Why |
| --- | --- | --- |
| Backend language | Rust | No GC pauses, low memory, predictable tail latency across thousands of long-lived streams |
| Runtime and web framework | Tokio + Axum, tower middleware | Fast, widely used; auth, rate limits and tracing compose as middleware |
| Upstream HTTP client | reqwest (hyper), pooled HTTP/2 connections | Reuses TLS connections to providers; streams bodies without buffering |
| Streaming to clients | Server-Sent Events | Matches OpenAI and Anthropic streaming; simpler than WebSockets and proxy-friendly |
| Database | PostgreSQL (default) or SQLite (small installs), via sqlx | Compile-time-checked queries; one codebase for both engines |
| Chat search | Postgres full-text / SQLite FTS5 | No separate search service to run |
| Cache and rate limits | In-process (moka, governor); optional Valkey/Redis for multi-node | Zero extra services on a single node |
| Crypto | Argon2id for passwords, HMAC-SHA256 for platform API keys, AES-256-GCM for stored secrets | Standard primitives; high-entropy API keys allow a fast keyed hash and O(1) lookup |
| Observability | tracing + OpenTelemetry, Prometheus endpoint | Structured logs and metrics with no add-ons |
| API contract | OpenAPI generated from Rust (utoipa), typed TypeScript client | Frontend and backend cannot drift apart |
| Frontend framework | SvelteKit (Svelte 5), built as a static SPA | Compiled, fine-grained reactivity keeps token streaming smooth; small bundles; familiar to Open WebUI users and contributors |
| UI components and styling | Tailwind CSS v4, bits-ui / shadcn-svelte | Accessible headless components, minimal CSS |
| Data fetching | TanStack Query (Svelte) | Caching, retries, optimistic updates |
| Markdown, code, math | markdown-it, Shiki (lazy-loaded), KaTeX | Rich rendering without delaying first paint |
| Long chats | Virtualized message list | Smooth scrolling with thousands of messages |
| Packaging | One multi-arch image (amd64, arm64), frontend embedded; docker-compose with Postgres | Self-hosting in one command, also on ARM servers |
| Testing | cargo tests with mock providers, Playwright for UI, k6 for load | Section 14 gates run in CI |

**Alternatives considered**

- Go (chi or Echo): close in speed and easier to hire for; the fallback if the team lacks Rust experience. Rust wins on memory and tail latency under many concurrent streams.
- React 19 + Vite: biggest ecosystem, but larger bundles and more re-render tuning while tokens stream.
- WebSockets: only needed for real-time collaboration; SSE covers v1.

**Reuse before building**

- Use an existing, well-maintained library or framework when one covers the need; do not reinvent the wheel.
- Write custom code only for what is unique to the platform or where no suitable library exists.
- Before writing new code, check the libraries already in the stack, then the standard library, then established crates and npm packages.

**Performance budget** (enforced in CI)

| Metric | Target |
| --- | --- |
| Added latency to first token, p95, excluding provider | < 20 ms |
| Concurrent streaming chats on 2 vCPU / 4 GB | ≥ 1,000 |
| Backend idle memory | < 100 MB |
| Server cold start | < 1 s |
| Docker image size | < 60 MB |
| Initial JavaScript, gzipped | < 200 KB |
| Time to interactive, mid-range laptop | < 1.5 s |

**Design rules for speed**

- Nothing blocking on the request path: logging and usage go through a bounded channel to a batch writer.
- Access decisions are cached in memory per user and invalidated when grants change.
- Streams are translated chunk by chunk and never buffered whole.
- Model lists and key health are cached; key failover happens without the user retrying.

## 14. Phasing, open questions and acceptance

v1 ships in two phases, core platform then gateway and skills, with each phase closed by a gate tied to the acceptance criteria below.

&#91;embedded content: delivery phases and release gates\]

Phase 3 holds the Could items; its order should follow user demand after v1 is live.

**Open questions**

- [ ] Should admins ever be able to read users' chats for compliance (NFR-AUD-03)?
- [ ] One organization per install, or is multi-tenant hosting needed later?
- [ ] Which quota unit matters most to admins: tokens, requests, or estimated cost?
- [ ] Open-source or proprietary license?

**Acceptance criteria (v1)**

1. On a fresh install, an admin completes setup, adds one OpenAI-format and one Anthropic-format provider, auto-fetches models, and chats with both.
2. A private model granted only to group A works for A's members and returns 403 for everyone else, in chat and through the API.
3. Switching a model from public to private removes it from non-granted users' pickers and API model list on the next request.
4. A platform API key works with the official OpenAI and Anthropic Python SDKs by changing only the base URL and key, streaming included.
5. Revoking a platform API key or deactivating a user blocks their next API call.
6. With Brave configured, a question about a current event returns an answer with at least one clickable cited source.
7. No provider key appears in any API response, browser payload or log line, verified by an automated scan.
8. Every admin change to providers, keys, models, grants, users and settings appears in the audit log.
