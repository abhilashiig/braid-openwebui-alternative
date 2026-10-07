#!/usr/bin/env python3
"""End-to-end acceptance check (spec section 14) against a fresh database and a mock provider.

Usage: python3 scripts/e2e.py            (needs a debug build: cd backend && cargo build)
Env:   E2E_DATABASE (default braid_e2e), PG_BIN (dir with createdb/dropdb), E2E_PORT (default 3099)

Covers acceptance criteria 1, 2, 3, 5, 7 and 8 plus both API formats. Stdlib only.
"""

import http.cookiejar
import json
import os
import secrets
import shutil
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DB = os.environ.get("E2E_DATABASE", "braid_e2e")
PORT = int(os.environ.get("E2E_PORT", "3099"))
BASE = f"http://127.0.0.1:{PORT}"
PROVIDER_KEY = "sk-mock-" + secrets.token_hex(16)  # must never leak anywhere
PG_BIN = os.environ.get("PG_BIN", "")


class Mock(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def authed(self):
        if self.path.startswith("/an/"):
            return self.headers.get("x-api-key") == PROVIDER_KEY
        return self.headers.get("authorization") == f"Bearer {PROVIDER_KEY}"

    def send_json(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if not self.authed():
            return self.send_json(401, {"error": {"message": "bad key"}})
        ids = ["claude-test"] if self.path.startswith("/an/") else ["gpt-test", "gpt-private"]
        self.send_json(200, {"data": [{"id": i} for i in ids]})

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"{}")
        if not self.authed():
            return self.send_json(401, {"error": {"message": "bad key"}})
        text = f"pong from {req['model']}"
        if self.path.startswith("/an/"):
            return self.send_json(200, {"id": "m", "type": "message", "role": "assistant", "model": req["model"],
                                        "content": [{"type": "text", "text": text}], "stop_reason": "end_turn",
                                        "usage": {"input_tokens": 5, "output_tokens": 3}})
        if not req.get("stream"):
            return self.send_json(200, {"id": "c", "object": "chat.completion", "model": req["model"],
                                        "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
                                        "usage": {"prompt_tokens": 5, "completion_tokens": 3}})
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.end_headers()
        for c in [{"choices": [{"index": 0, "delta": {"content": text}}]},
                  {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]},
                  {"choices": [], "usage": {"prompt_tokens": 5, "completion_tokens": 3}}]:
            self.wfile.write(f"data: {json.dumps(c)}\n\n".encode())
        self.wfile.write(b"data: [DONE]\n\n")
        self.close_connection = True


class Client:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.seen = []  # every response body, for the key-leak scan

    def req(self, method, path, body=None, headers=None, raw=False):
        h = {"x-braid-csrf": "1", **(headers or {})}
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            h["content-type"] = "application/json"
        r = urllib.request.Request(BASE + path, data=data, method=method, headers=h)
        try:
            with self.opener.open(r, timeout=30) as res:
                text, status = res.read().decode(), res.status
        except urllib.error.HTTPError as e:
            text, status = e.read().decode(), e.code
        self.seen.append(text)
        if raw:
            return status, text
        try:
            return status, json.loads(text)
        except json.JSONDecodeError:
            return status, text


failures = []


def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + ("" if cond else f"  -- {detail}"))
    if not cond:
        failures.append(name)


def pg(cmd):
    exe = os.path.join(PG_BIN, cmd) if PG_BIN else shutil.which(cmd) or cmd
    return exe


def main():
    binary = os.path.join(ROOT, "backend", "target", "debug", "braid")
    if not os.path.exists(binary):
        sys.exit("Build first: cd backend && cargo build")
    subprocess.run([pg("dropdb"), "--if-exists", DB], check=True)
    subprocess.run([pg("createdb"), DB], check=True)

    mock = ThreadingHTTPServer(("127.0.0.1", 0), Mock)
    threading.Thread(target=mock.serve_forever, daemon=True).start()
    mock_url = f"http://127.0.0.1:{mock.server_address[1]}"

    user = os.environ.get("USER", "postgres")
    env = {**os.environ, "DATABASE_URL": os.environ.get("E2E_DATABASE_URL", f"postgres://{user}@localhost:5432/{DB}"),
           "BRAID_MASTER_KEY": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", "BRAID_BIND": f"127.0.0.1:{PORT}",
           "BRAID_SETUP_TOKEN": "e2e-token", "BRAID_PUBLIC_URL": BASE, "BRAID_LOG_FORMAT": "json", "RUST_LOG": "braid=debug"}
    log_path = os.path.join(ROOT, "backend", "target", "e2e.log")
    log = open(log_path, "w")
    server = subprocess.Popen([binary], env=env, stdout=log, stderr=subprocess.STDOUT, cwd="/")
    try:
        for _ in range(100):
            try:
                urllib.request.urlopen(BASE + "/api/health", timeout=1)
                break
            except Exception:
                time.sleep(0.1)
        run(mock_url)
    finally:
        server.terminate()
        server.wait()
        mock.shutdown()
        log.close()

    # Criterion 7: no provider key in any response or log line.
    with open(log_path) as f:
        logs = f.read()
    check("7: provider key absent from server logs", PROVIDER_KEY not in logs)
    check("7: provider key absent from every API response", all(PROVIDER_KEY not in s for c in CLIENTS for s in c.seen))
    subprocess.run([pg("dropdb"), "--if-exists", DB])
    print(f"\n{len(failures)} failure(s)" if failures else "\nAll checks passed")
    sys.exit(1 if failures else 0)


CLIENTS = []


def new_client():
    c = Client()
    CLIENTS.append(c)
    return c


def run(mock_url):
    pw = "e2e-" + secrets.token_urlsafe(16)
    admin = new_client()
    s, _ = admin.req("POST", "/api/setup", {"token": "e2e-token", "name": "Admin", "email": "admin@e2e.test", "password": pw})
    check("1: admin created through setup wizard", s == 200, s)
    s, _ = admin.req("POST", "/api/setup", {"token": "e2e-token", "name": "X", "email": "x@e2e.test", "password": pw})
    check("setup disabled after first admin", s == 403, s)

    s, oa = admin.req("POST", "/api/admin/providers", {"name": "Mock OA", "api_format": "openai", "base_url": f"{mock_url}/oa/v1",
                                                       "allow_private": True, "keys": [{"label": "k", "key": PROVIDER_KEY}]})
    s2, an = admin.req("POST", "/api/admin/providers", {"name": "Mock AN", "api_format": "anthropic", "base_url": f"{mock_url}/an/v1",
                                                        "keys": [{"label": "k", "key": PROVIDER_KEY}]})
    check("1: OpenAI- and Anthropic-format providers added", s == 200 and s2 == 200, (s, s2))
    _, t = admin.req("POST", f"/api/admin/providers/{oa['id']}/test")
    check("provider connection test", t.get("ok") is True, t)
    _, up = admin.req("GET", f"/api/admin/providers/{oa['id']}/upstream-models")
    check("1: auto-fetch lists upstream models", {m["upstream_id"] for m in up["available"]} == {"gpt-test", "gpt-private"}, up)
    admin.req("POST", "/api/admin/models/import", {"provider_id": oa["id"], "upstream_ids": ["gpt-test"], "visibility": "public"})
    admin.req("POST", "/api/admin/models/import", {"provider_id": oa["id"], "upstream_ids": ["gpt-private"], "visibility": "private"})
    admin.req("POST", "/api/admin/models/import", {"provider_id": an["id"], "upstream_ids": ["claude-test"], "visibility": "public"})
    _, models = admin.req("GET", "/api/admin/models")
    by = {m["name"]: m for m in models}
    private_id = by["mock-oa/gpt-private"]["id"]

    for name in ["mock-oa/gpt-test", "mock-an/claude-test"]:
        _, chat = admin.req("POST", "/api/chats")
        s, body = admin.req("POST", f"/api/chats/{chat['id']}/messages", {"model_id": by[name]["id"], "content": "ping"}, raw=True)
        check(f"1: admin chats with {name}", s == 200 and "event: done" in body and "pong" in body, body[-300:])

    # Group A gets the private model; alice is in A, bob is not.
    _, group = admin.req("POST", "/api/admin/groups", {"name": "A"})
    admin.req("POST", f"/api/admin/groups/{group['id']}/grants", {"model_ids": [private_id]})
    _, inv = admin.req("POST", "/api/admin/invitations", {"emails": ["alice@e2e.test"], "role": "user", "group_ids": [group["id"]]})
    _, inv2 = admin.req("POST", "/api/admin/invitations", {"emails": ["bob@e2e.test"], "role": "user"})
    alice, bob = new_client(), new_client()
    for c, i in [(alice, inv), (bob, inv2)]:
        token = i["invited"][0]["link"].rsplit("/", 1)[1]
        s, _ = c.req("POST", f"/api/auth/invite/{token}", {"name": "U", "password": pw})
        check("invitation accepted", s == 200, s)
    s, _ = bob.req("POST", f"/api/auth/invite/{inv2['invited'][0]['link'].rsplit('/', 1)[1]}", {"name": "U", "password": pw})
    check("invitation link is single-use", s == 404, s)

    s, settings = admin.req("GET", "/api/admin/settings")
    settings["allow_api_keys"] = True
    admin.req("PUT", "/api/admin/settings", settings)
    keys = {}
    for who, c in [("alice", alice), ("bob", bob)]:
        s, k = c.req("POST", "/api/me/api-keys", {"name": "e2e"})
        keys[who] = k["key"]

    def api(who, path, body=None, fmt="openai"):
        h = {"authorization": f"Bearer {keys[who]}"} if fmt == "openai" else {"x-api-key": keys[who]}
        return CLIENTS[0].req("POST" if body else "GET", path, body, headers=h)

    _, chat_a = alice.req("POST", "/api/chats")
    s, _ = alice.req("POST", f"/api/chats/{chat_a['id']}/messages", {"model_id": private_id, "content": "hi"}, raw=True)
    check("2: granted group member can chat with private model", s == 200, s)
    _, chat_b = bob.req("POST", "/api/chats")
    s, _ = bob.req("POST", f"/api/chats/{chat_b['id']}/messages", {"model_id": private_id, "content": "hi"})
    check("2: non-member gets 403 in chat", s == 403, s)
    s, r = api("alice", "/api/v1/chat/completions", {"model": "mock-oa/gpt-private", "messages": [{"role": "user", "content": "hi"}]})
    check("2: granted member works through the API", s == 200 and "pong" in json.dumps(r), (s, r))
    s, r = api("bob", "/api/v1/chat/completions", {"model": "mock-oa/gpt-private", "messages": [{"role": "user", "content": "hi"}]})
    check("2: non-member gets 403 through the API", s == 403 and r["error"]["type"] == "permission_error", (s, r))
    s, r = api("bob", "/api/anthropic/v1/messages", {"model": "mock-oa/gpt-private", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}, fmt="anthropic")
    check("2: Anthropic endpoint returns Anthropic-shaped 403", s == 403 and r.get("type") == "error", (s, r))
    s, r = api("alice", "/api/anthropic/v1/messages", {"model": "mock-oa/gpt-test", "max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]}, fmt="anthropic")
    check("OpenAI-format model through Anthropic endpoint", s == 200 and r["content"][0]["text"].startswith("pong"), (s, r))
    s, body = CLIENTS[0].req("POST", "/api/v1/chat/completions", {"model": "mock-an/claude-test", "stream": True, "messages": [{"role": "user", "content": "hi"}]},
                             headers={"authorization": f"Bearer {keys['alice']}"}, raw=True)
    check("Anthropic-format model streamed through OpenAI endpoint", s == 200 and "pong" in body and "[DONE]" in body, body[-200:])

    # Criterion 3: public -> private removes it from bob's picker and API list on the next request.
    m = by["mock-oa/gpt-test"]
    _, lst = api("bob", "/api/v1/models")
    check("public model listed for bob", "mock-oa/gpt-test" in [d["id"] for d in lst["data"]], lst)
    admin.req("PUT", f"/api/admin/models/{m['id']}", {**m, "visibility": "private"})
    _, lst = api("bob", "/api/v1/models")
    check("3: private switch removes model from API list", "mock-oa/gpt-test" not in [d["id"] for d in lst["data"]], lst)
    _, mine = bob.req("GET", "/api/models")
    check("3: private switch removes model from picker", all(x["name"] != "mock-oa/gpt-test" for x in mine["models"]), mine)

    # Criterion 5: revoking a key / deactivating a user blocks the next call.
    _, my = alice.req("GET", "/api/me/api-keys")
    alice.req("DELETE", f"/api/me/api-keys/{my['keys'][0]['id']}")
    s, _ = api("alice", "/api/v1/models")
    check("5: revoked key is rejected", s == 401, s)
    _, users = admin.req("GET", "/api/admin/users?q=bob")
    admin.req("PATCH", f"/api/admin/users/{users[0]['id']}", {"status": "deactivated"})
    s, _ = api("bob", "/api/v1/models")
    check("5: deactivated user's key is rejected", s == 401, s)
    s, _ = bob.req("GET", "/api/auth/me")
    check("5: deactivated user's session ends", s == 401, s)
    _, me = admin.req("GET", "/api/auth/me")
    s, _ = admin.req("PATCH", f"/api/admin/users/{me['user']['id']}", {"role": "user"})
    check("last admin cannot be demoted", s == 409, s)

    # Criterion 8: admin changes are audited.
    _, audit = admin.req("GET", "/api/admin/audit")
    actions = {a["action"] for a in audit}
    want = {"provider.created", "model.imported", "model.updated", "group.created", "grant.created", "invitation.created", "settings.updated", "user.updated"}
    check("8: admin changes appear in the audit log", want <= actions, want - actions)

    # Provider keys stay write-only.
    _, providers = admin.req("GET", "/api/admin/providers")
    check("provider keys shown only as last 4", all(k["last4"] == PROVIDER_KEY[-4:] for p in providers for k in p["keys"]), providers)
    s, _ = bob.req("GET", "/api/admin/providers")
    check("admin endpoints refuse non-admins", s in (401, 403), s)


if __name__ == "__main__":
    main()
