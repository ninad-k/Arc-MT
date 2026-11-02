# HTTP API

Served by `jarvis-api` on `server.bind` (default `http://127.0.0.1:8787`, or `https://` when
`tls_cert`/`tls_key` are set). All bodies are JSON unless stated. The first part of this file is
the read-only chat API (Sprint 1); "Sprint 2–5 additions" below covers trading, login, voice,
events, static files and TLS. Every `/api/*` route except `GET /api/health` and
`POST /api/auth/login` needs a login session when `[auth] enabled = true` (the default).

Errors use one shape everywhere:

```json
{ "error": { "code": "bridge_unavailable", "message": "bridge did not reply within 3000 ms" } }
```

| code | HTTP | when |
|---|---|---|
| `bad_request` | 400 | invalid body, empty message, unknown or unavailable provider, `session_id` not a UUID |
| `mode_mismatch` | 409 | config `mode = "demo"` but the account reports `trade_mode = real` (chat refused) |
| `bridge_unavailable` | 503 | bridge timeout or transport error (also `/api/watchlist` when every symbol failed) |
| `bridge_error` | 502 | EA answered `ok:false` (message includes retcode) |
| `llm_error` | 502 | LLM provider failed |
| `internal` | 500 | anything else (details only in server logs) |

Added later (see below): `unauthorized` 401, `forbidden` 403, `not_found` 404 (also any
unknown `/api/...` path), `method_not_allowed` 405, `request_timeout` 408, `payload_too_large` 413.

## GET /api/status

Health of every part. This endpoint never fails because the bridge is down; it reports it instead.

```json
{
  "version": "0.1.0",
  "mode": "demo",
  "mode_mismatch": false,
  "bridge": {
    "connected": true,
    "latency_ms": 4,
    "ea_version": "0.5.0",
    "protocol_version": 2,
    "trade_mode": "demo",
    "terminal_connected": true,
    "error": null
  },
  "llm": {
    "default": "local",
    "providers": [
      { "name": "local",  "kind": "ollama",    "model": "qwen3.5:9b",       "available": true },
      { "name": "claude", "kind": "anthropic", "model": "claude-sonnet-5-5", "available": false }
    ]
  }
}
```

When the bridge is down: `"connected": false`, with `"error": "..."` set and the other bridge fields `null`.
`available` is false when a provider's API key env var is missing. Sprint 1 doesn't probe the network.

## GET /api/account

Account summary for the HUD (same redacted data the LLM sees):

```json
{ "currency": "USD", "balance": 10000.0, "equity": 10012.5, "profit": 12.5, "margin": 230.0,
  "free_margin": 9782.5, "margin_level": 4353.26, "leverage": 100, "trade_mode": "demo" }
```

## GET /api/positions

```json
{ "positions": [ { "ticket": 123, "symbol": "XAUUSD", "side": "buy", "volume": 0.1, "price_open": 2390.1,
  "price_current": 2391.2, "sl": null, "tp": 2420.0, "profit": 11.0, "swap": 0.0, "magic": 0,
  "comment": "", "time": 1793340000 } ] }
```

## GET /api/orders

Pending orders, in the bridge's `Order` shape (auth-protected like `/api/positions`):

```json
{ "orders": [ { "ticket": 2001001, "symbol": "EURUSD", "type": "buy_limit", "volume_initial": 0.2,
  "volume_current": 0.2, "price_open": 1.0820, "sl": null, "tp": null, "time_setup": 1793340000,
  "comment": "" } ] }
```

## GET /api/watchlist

Prices for `symbols.watchlist`. Symbols that fail are listed with an error, so one bad symbol doesn't fail the whole call.

```json
{ "prices": [ { "symbol": "XAUUSD", "bid": 2391.12, "ask": 2391.40, "last": 0.0, "spread_points": 28,
                "digits": 2, "time": 1793341200 } ],
  "errors": [ { "symbol": "US30", "message": "unknown symbol: US30" } ] }
```

## POST /api/chat

Request:

```json
{ "session_id": "optional, omit to start a new session", "message": "what's gold at?", "provider": "optional provider name" }
```

Response:

```json
{
  "session_id": "4b0d…",
  "reply": "Gold (XAUUSD) is 2391.12 bid / 2391.40 ask.",
  "provider": "local",
  "tool_calls": [
    { "name": "get_price", "arguments": { "symbol": "XAUUSD" }, "ok": true, "duration_ms": 6 }
  ]
}
```

- `message` must be 1–2000 characters.
- At most `[server] chat_rate_limit_per_minute` (20) requests per minute for one login session
  (per client address when login is off): the next one gets `429 rate_limited` with
  `Retry-After`, before the body is read and without calling the model. A request that takes
  longer than `[server] long_request_timeout_secs` (200 s) is stopped with `408 request_timeout`.
- Session history stays in memory (lost on restart) and is capped at the last 40 messages.
- Every turn writes audit rows: `utterance`, `tool_call`, `bridge_request`, `bridge_reply`, `assistant_reply`.

---

# Sprint 2–5 additions

Everything above still holds. New error codes:

| code | HTTP | when |
|---|---|---|
| `unauthorized` | 401 | no or invalid session (when `auth.enabled`), a failed login, or a refused kill-switch release (see below) |
| `forbidden` | 403 | WebSocket upgrade from a foreign `Origin` |
| `not_found` | 404 | unknown `/api/...` path (after the session check, so an unauthenticated client gets 401) |
| `method_not_allowed` | 405 | wrong HTTP method for the path |
| `request_timeout` | 408 | the request took longer than `[server] request_timeout_secs` (30 s), or `long_request_timeout_secs` (200 s) for `/api/chat`, `/api/voice/*`, `/api/confirm/*`, `/api/market/view` and `/api/market/news`; the WebSocket has no limit |
| `payload_too_large` | 413 | body over the limit (64 KB for JSON routes, 5 MB for `/api/voice/transcribe`) |
| `gate_rejected` | 422 | the gate refused to build a preview (message says which limit or why), **or refused a confirmed order when the checks ran again at confirmation** (see `POST /api/confirm/{token}`) |
| `token_unknown` | 404 | confirmation token not found |
| `token_expired` | 410 | confirmation token expired |
| `token_used` | 409 | confirmation token already used or cancelled |
| `kill_switch` | 423 | kill switch engaged; all write endpoints refuse |
| `rate_limited` | 429 | too many requests (`Retry-After` header set): login, confirm, `POST /api/chat` and `POST /api/voice/transcribe` have limits |
| `voice_unavailable` | 503 | voice disabled or models missing |
| `trade_failed` | 502 | MT5 rejected a write (message includes the trade retcode) |
| `outcome_unknown` | 504 | bridge timed out after a write was sent; positions must be checked |

## Authentication (Sprint 4)

When `[auth] enabled = true` (the default), every `/api/*` route except `GET /api/health` and
`POST /api/auth/login` needs a session; so does an unknown `/api/...` path (401 before 404).
The web UI itself (static files) is public: it has to load to show the login page.

The session is a JWT (HS256; claims `sub`, `exp`, `iat`, `jti`; secret from env
`JARVIS_JWT_SECRET`, at least 32 bytes with at least 16 different byte values and no repeated
block — the server refuses to start with login enabled and no usable secret) in the cookie `jarvis_session`:
`Path=/; HttpOnly; SameSite=Strict; Max-Age=<session_hours>`, plus `Secure` when the server
serves TLS itself (or `[auth] cookie_secure = true`, for a TLS-terminating proxy in front of a
loopback port). The only case without `Secure` is plain HTTP on loopback during development.
Chrome and Firefox also accept `Secure` cookies from `http://localhost`, but not every browser does,
so in development prefer the Vite proxy (same origin) or run with TLS. WebSocket upgrades use the
same cookie. Nothing in the API is addressed with a bearer header.

Users are created on the PC that runs Jarvis, from the command line (there is no HTTP endpoint
for it):

```text
jarvis-api user add <name>        # asks for the password twice (no echo), prints the otpauth:// URI and a QR code
jarvis-api user add <name> --password-stdin   # for scripts: the password is the first line of stdin
jarvis-api user reset-totp <name> # new authenticator secret; ends the user's sessions, clears the kill-switch release lock
jarvis-api user list
jarvis-api user remove <name>     # sessions stop working at once
```

These need `JARVIS_JWT_SECRET` (in `.env` or the environment) and use `database.url`. Passwords are
12 to 256 characters and hashed with Argon2id (19 MiB, 2 passes, 1 lane). The TOTP secret
(SHA-1, 6 digits, 30 s) is stored encrypted (ChaCha20-Poly1305, key derived from
`JARVIS_JWT_SECRET` with HKDF-SHA256, bound to the username). **Changing `JARVIS_JWT_SECRET`
logs everyone out and makes the stored TOTP secrets unreadable: run `user reset-totp` for each
user afterwards.** The database file holds password hashes and encrypted secrets, so protect it
like `.env` (readable only by the account that runs Jarvis).

- `POST /api/auth/login` `{username, password, totp}` → `200 {username, expires_at}` (`expires_at`
  is RFC 3339 UTC) and sets the cookie. Every failure — unknown user, wrong password, wrong,
  reused or malformed code, locked account — is the same `401 unauthorized` with the message
  `invalid username, password or code`; the real reason is only in the audit log
  (`login_failed`). A code is accepted for the current 30 s step and one step either side, and
  **never twice** (the last accepted step is remembered per user). Limits: 5 attempts per minute per
  client IP (every attempt counts; the sixth gets `429 rate_limited` with `Retry-After`).
  Lockout is per **(account, client IP)**: 10 failed attempts from one address lock that address
  out of that account for 15 minutes (answered with the same `401`), and nobody else is
  affected, so a stranger who knows the name cannot lock the owner out. Failures from all
  addresses together are counted per account as well; once 50 have been seen within 15 minutes,
  every attempt on that account waits 2 s before it is looked at (a delay, never a lock). The
  state is in memory and a restart clears it. The IP is the TCP peer address; `X-Forwarded-For`
  is not trusted, so behind a proxy every client shares one bucket (and one lockout key).
- `POST /api/auth/logout` → `204`, clears the cookie, and revokes the session's `jti` (kept in
  the database until the token would have expired, so a restart does not revive it).
- `GET /api/auth/me` → `200 {username, expires_at}`; `401` when there is no valid session. When
  auth is disabled: `200 {"username": null, "expires_at": null}`.
- `GET /api/health` → exactly `{"ok":true}` (no auth, no details; for service monitors and the
  desktop launcher).

Session checks on every request: signature and algorithm (`alg: none` and other algorithms are
refused), expiry, not logged out, the user still exists, and no TOTP reset since the token was
issued.

**401 on the kill-switch release** is an answer, not a lost session: a missing, wrong, reused or
rate-limited TOTP code gives `401 unauthorized` with the message `code not accepted`. An expired or
missing session gives `401 unauthorized` with the message `login required`. The UI tells the two
apart with `GET /api/auth/me`.

## Status additions

`GET /api/status` also returns:

```json
{
  "kill_switch": { "engaged": false, "reason": null, "changed_at_ms": null },
  "auth": { "enabled": true, "username": "ninad" },
  "voice": { "stt": true, "tts": true },
  "limits": { "max_lot": 1.0, "max_positions": 5, "max_daily_loss": 500.0,
              "allowed_symbols": ["XAUUSD", "EURUSD"], "trading_hours": null }
}
```

`auth.username` is the caller's name (`null` when auth is disabled). `voice.stt` / `voice.tts`
are true only when that engine loaded at startup.

## Previews and confirmation (Sprint 2)

A preview is created either by the LLM (`propose_*` tools during `/api/chat`) or by a
UI button (close endpoints below). Both go through the same gate. The token is only
ever sent to the browser, never to the LLM.

`PreviewDto`:

```json
{
  "id": "6f1c…",
  "description": "Close 3 positions on XAUUSD (0.30 lots), floating P/L −42.10 USD",
  "symbols": ["XAUUSD"],
  "tickets": [101, 102, 103],
  "total_volume": 0.3,
  "est_pnl": { "current": -42.1, "at_sl": null, "at_tp": null },
  "is_batch": true,
  "count": 3,
  "warnings": [],
  "token": "9a0f…64 hex…",
  "stage": "first",
  "expires_at_ms": 1793341578000,
  "timing": "immediate"
}
```

- `POST /api/chat` response gains `"preview": PreviewDto | null`. The request gains
  optional `"voice": true`; previews created in a voice turn use timing `after_readback`.
- `POST /api/confirm/{token}` → one of:
  - `200 {"status":"executed","preview_id","results":[{"ticket","ok","retcode","retcode_description","order","deal","price","volume","error"}]}`
  - `200 {"status":"needs_second_confirmation","preview_id","token","stage":"second","expires_at_ms","count","message":"Confirm again to close 3 positions"}`
  - errors `token_unknown` / `token_expired` / `token_used` / `kill_switch` / `rate_limited` / `outcome_unknown` / `trade_failed`
    / `gate_rejected`.
  - **The checks run again at confirmation** (a preview is valid for 15 s, and the account and
    the market move). Confirmed actions are sent one at a time, and each is checked under that
    turn:
    - a **new order** goes through the gate again with a fresh snapshot (limits, trading hours,
      daily loss, `max_positions`, stops distance) and must produce the same order (symbol,
      side, type, volume, price, SL, TP). If it fails, the answer is `422 gate_rejected` with a
      message that starts "The checks were run again at confirmation", nothing is sent, the token
      stays spent (a second confirm is `409 token_used`), and the audit log gets a
      `gate_rejected` row with `reason: "re-check at confirm"` and the `rule` (for example
      `limit:max_positions`). Example: two sessions each hold a preview that fits `max_positions`;
      confirming the first succeeds and the second is refused. The order is never adjusted: if the
      market moved so that SL or TP no longer fits the broker's minimum distance, it is refused;
      if it still fits, the order that was confirmed is sent, with its own comment.
    - a **close, cancel or modify** is not held to the limits (it reduces or keeps risk), only to
      the kill switch and to its tickets still existing. If no ticket exists any more the answer
      is `422 gate_rejected` (reason `re-check at confirm`, rule `ticket_gone`). In a batch, the
      tickets that are gone are skipped and listed in `results` as `ok: false` with `error`
      "not sent: position 123 no longer exists"; the rest is sent.
    - If the checks cannot be run (the bridge fails while reading the account), the answer is the
      bridge error, nothing is sent, and the token is spent.
  Rate limit: 10/min **per login session** (keyed by the token's `jti`, so two sessions of the same
  user have separate allowances; with auth disabled it is per client IP). Batch commands run
  sequentially; one failure does not stop the rest; each result is reported.
  - **`trade_failed` (502) is only for a single-write action** (one order, one close) that MT5
    rejected; the message is the broker's error text with the retcode. A **batch** always
    answers `200 executed` with one entry per write, and failures appear as `ok: false` entries in
    `results`. The UI must read `results`, not the HTTP status, for batches.
  - **`aborted`**: when the kill switch is engaged part-way through a batch, the response is
    `200 {"status":"executed", ..., "aborted":"kill_switch"}`. The writes already sent are
    reported as usual; the writes not sent appear as `ok: false` with `error` "not sent: the kill
    switch was engaged". The field is absent otherwise. If the switch was already engaged
    before the first write, nothing is sent and the answer is `423 kill_switch`.
  - **`outcome_unknown` (504)**: a write was sent and no reply arrived (never retried, ADR-0013).
    The message states which write it was ("write 2 of 3"), that its outcome is unknown, **how
    many earlier writes completed**, that the remaining writes were not sent, and tells the
    user to check MetaTrader 5. The UI should refresh positions (the server also emits
    `positions_changed`).
  - **`TradeResultDto.ticket`**: the ticket the write was *about*: the position ticket for a
    close or modify, the order ticket for a cancel, and for a **new order** the order ticket MT5
    assigned (`null` when MT5 did not accept the order). `order` and `deal` are the MT5 order
    and deal numbers of the execution, which for a market order can differ from `ticket`.
- `POST /api/cancel/{token}` → `204` (also cancels the batch sibling).
- `POST /api/readback/{token}` → `200 {"expires_at_ms"}`: the UI calls this when TTS has
  finished reading the preview aloud; the 15 s window starts then (cap 30 s from issue).
- `POST /api/positions/{ticket}/close` `{"volume": null | 0.05}` → `200 PreviewDto`.
- `POST /api/positions/close` `{"symbol": null|"XAUUSD", "side": null|"buy"|"sell", "all": true}` → `200 PreviewDto` (batch).
- `POST /api/orders/{ticket}/cancel` → `200 PreviewDto`.
- UI-button previews (the `/close` and `/cancel` endpoints) are not part of any chat session: they
  live in one shared "ui" bucket, and a new UI preview replaces the previous UI preview. Each
  chat session keeps its own live preview; a chat message never replaces a UI preview and
  the other way round.
- `GET /api/killswitch` → `{engaged, reason, changed_at_ms}`. The state is saved in the database
  (table `kill_switch`): a restart or a service restart after a crash keeps it engaged (audit row
  `kill_switch` with `restored_after_restart`), and only a release with a session and a fresh
  TOTP code turns trading back on. If the saved state cannot be read, trading stays off.
- `POST /api/killswitch` `{"engaged": true, "reason": "manual"}` → same shape. Engaging
  revokes every live token immediately and needs only a session. Releasing needs a valid session
  **and** a fresh TOTP code in the body (`{"engaged": false, "totp": "123456"}`) when auth is
  enabled; a code works once, so the code that opened the session cannot be reused in the same
  30 s step (wait for the next one). Refusals are `401 unauthorized` (see "401 on the kill-switch
  release" above); 5 attempts per minute per user, and failed attempts count towards the account
  lockout. With auth disabled a release needs no code.
- `GET /api/audit?limit=100&before_id=` → `{"entries":[{"id","ts","session_id","kind","payload"}]}` newest first.

Audit kinds added: `intent`, `preview`, `gate_rejected`, `confirm`, `confirm_second`,
`cancel`, `trade_result`, `kill_switch`, `login`, `login_failed`, `voice_stt`, `voice_tts`,
`limits_changed`, `ungrounded_reply`, `ungrounded_reply_blocked`.

**Grounding check.** Every final chat reply is checked for figures (prices, amounts,
percentages, lots) that did not come from a tool result, the user's own messages or the
clock in this conversation. An ungrounded reply is never shown: the model is asked once to
fetch the figures with a tool (`ungrounded_reply` audit row); if it still states an
unsourced figure, the user gets "I couldn't verify that from MetaTrader just now, so I
won't guess. Please ask again." (`ungrounded_reply_blocked`). This applies identically to
every persona and provider.

- `intent` rows are written once per chat turn, **only for intents other than `Other`**
  (a confirm, cancel, order, close, modify or cancel-orders request); small talk and questions
  do not get one.
- `login` rows carry `{username, ip, jti}` (and `event: "logout"` for a logout); `login_failed`
  rows carry `{username, ip, reason, locked_now}` with `reason` one of `unknown_user`,
  `bad_password`, `bad_code`, `code_reused`, `locked` (this address is locked out of the account),
  `rate_limited`, `malformed`. Passwords and
  TOTP codes are never written. `username` is the stored spelling of a real account; a name that
  belongs to nobody (and every `rate_limited` and `malformed` row) shows `<not recorded>`, because
  people type passwords into the name field. A flood of refused logins writes one `rate_limited`
  row per client per minute. A refused kill-switch release is a `kill_switch` row with
  `release_refused`.
- `voice_stt` rows hold the transcript text, raw text, timings and the parsed intent, never the
  audio. `voice_tts` rows hold the spoken text and the size of the WAV.

## Voice (Sprint 3)

- `POST /api/voice/transcribe` is limited to `[server] transcribe_rate_limit_per_minute` (20)
  requests per minute per session (`429` with `Retry-After`, checked before the body is read).
- `POST /api/voice/transcribe` body `audio/wav` (`Content-Type: audio/wav`, 16-bit PCM, at most
  `voice.max_seconds` (30 s) and 5 MB) →
  `200 {"text","raw","audio_ms","elapsed_ms","intent":"confirm"|"cancel"|"other"}`.
  The UI uses `intent` to call `/api/confirm/{token}` or `/api/cancel/{token}` when a
  preview is pending; otherwise it sends `text` to `/api/chat` with `"voice": true`.
  `intent` comes from the same deterministic parser the gate uses. A wrong content type,
  an unreadable or too long recording is `400 bad_request`; over 5 MB is `413`. Only one
  transcription runs at a time; a request that waits more than 5 s for its turn gets
  `429 rate_limited`.
- `POST /api/voice/speak` `{"text"}` (1–600 characters) → `200 audio/wav` (`400` for empty or longer text).
- Both answer `503 voice_unavailable` when `[voice]` is off or that engine did not load. Engines
  are loaded at startup; a missing model or a binary built without the `whisper` cargo feature
  logs a warning and switches only that half off. `GET /api/status` shows `voice.stt` / `voice.tts`.
  Speech-to-text needs `cargo build -p jarvis-api --features whisper` (CMake and LLVM); the default
  build has text-to-speech only.

## Events (Sprint 3)

`GET /api/ws` (WebSocket, JSON text frames, server → client). It needs the session cookie
(401 otherwise) and an `Origin` that is the server's own origin or a dev origin (403
otherwise); a request with no `Origin` header is accepted. At most 16 sockets are open at
once (`429`). The server pings every 20 s and drops a socket that has been silent for 70 s; it also
closes (code 1008) when the session ends or expires, and (code 1001) when the server shuts down.
Client messages are ignored (keep them under 1 KB).

```json
{"type":"kill_switch","engaged":true,"reason":"manual"}
{"type":"preview_expired","token":"…"}
{"type":"trade_executed","preview_id":"…","results":[…]}
{"type":"positions_changed"}
```

If the server could not keep up and dropped events for a socket, it sends
`{"type":"positions_changed"}` so the UI re-reads. The UI keeps polling as a fallback; events just
make it faster.

## Static files and TLS (Sprints 4–5)

- `[server] static_dir = "web/build"`: the API serves the built SPA at `/`. A path with a file
  extension is a file or a plain 404; any other path that is not a file gets `index.html`
  (client-side routes). `/api/...` is never served from there (unknown paths are a JSON 404, or
  401 without a session). Cache headers: `public, max-age=31536000, immutable` under
  `/_app/immutable/`, `no-cache` for everything else (`index.html`, the service worker, the
  manifest). API responses carry `Cache-Control: no-store`. Pre-compressed `.br`/`.gz` files next
  to the originals are used when the browser accepts them.
- `[server] tls_cert` / `tls_key` (PEM, key unencrypted): serve HTTPS/WSS (rustls). Both or neither.
  Binding a non-loopback address is refused at startup unless TLS **and** `[auth] enabled` are on.
  With TLS the server sends `Strict-Transport-Security: max-age=31536000; includeSubDomains`, the
  session cookie is `Secure`, and the CSP gets `upgrade-insecure-requests`.
- Security headers on **every** response (API, static files, errors):
  - `Content-Security-Policy: default-src 'self'; script-src 'self' 'sha256-<inline bootstrap>';
    style-src 'self'; style-src-attr 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self'
    blob:; font-src 'self'; connect-src 'self'; worker-src 'self'; manifest-src 'self';
    object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'`.
    - SvelteKit's static build writes one inline `<script>` into `index.html`. At startup the
      server hashes every inline script in the HTML files under `static_dir` and lists the
      hashes in `script-src`; there is no `'unsafe-inline'` or `'unsafe-eval'` for scripts, so
      **rebuild the web UI, then restart the server** (the hashes are read once).
    - `style-src-attr 'unsafe-inline'` exists only because the SvelteKit wrapper element has a
      `style="display: contents"` attribute; `<style>` blocks are not allowed.
    - `connect-src` is `'self'` and nothing else (CSP-1). In CSP level 3 `'self'` covers a
      WebSocket to the same host and port (`ws:` from an `http:` page, `wss:` from an `https:`
      one), which is what the live feed uses, in Chrome, Edge (so the desktop shell's WebView2)
      and Firefox. The bare `ws:`/`wss:` that used to be listed let a script open a WebSocket to
      any host; they were only for old Safari, which is not a supported browser. The header is not
      built from the request's `Host`: it would only repeat what `'self'` already says.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
    `Permissions-Policy: microphone=(self), camera=(), geolocation=()`,
    `Cross-Origin-Opener-Policy: same-origin`, `X-Frame-Options: DENY`.
- Request bodies: 64 KB for JSON routes (including login), 5 MB for `/api/voice/transcribe`;
  larger ones get `413 payload_too_large`.
- Time and connection limits (NET-1, all in `[server]`, see [configuration.md](configuration.md#server)):
  - `header_read_timeout_secs` (10): a client has this long to send the headers of a request. The
    same clock closes a keep-alive connection that sits idle, and a TLS handshake has its own 10 s.
    HTTP/2 (TLS only) is pinged every 20 s and a peer that does not answer within 10 s is dropped,
    with at most 64 streams per connection.
  - `request_timeout_secs` (30) for every route, and `long_request_timeout_secs` (200) for
    `/api/chat`, `/api/voice/*` and `/api/confirm/*`. The limit covers the whole exchange up to the
    response, a slow body upload included. A request that runs out is answered `408` in the
    standard error shape (`request_timeout`). `GET /api/ws` has no limit: the upgrade is
    answered at once, and an open socket is kept by pings (see Events). A confirmed trade keeps
    running when its request is stopped; read `/api/positions` and the audit log.
  - `max_connections` (256): another connection is closed before anything is read from it.
- CORS is off. `[server] dev_origins` (default empty; for example `["http://localhost:5173"]`
  when a dev server calls the API directly) lists browser origins that may call the API with the
  session cookie. Anything listed can drive the whole API, so list only origins you control. The
  Vite proxy (`pnpm dev`) and the desktop shell are same-origin and need no entry.
- Request guard on every `/api/*` route (`403 forbidden`):
  - a state-changing request (not GET, HEAD or OPTIONS) with an `Origin` header must come from
    the server's own origin (the `Origin` equals the `Host`) or a listed dev origin. A request with
    no `Origin` (curl, scripts) passes: it still needs the session cookie. `SameSite=Strict`
    already stops other sites; this also stops other *origins of the same site* (another port on
    `localhost`, a sibling sub-domain);
  - while `[auth] enabled = false`, the `Host` must be a loopback name (`localhost`,
    `*.localhost`, `127.x.x.x`, `[::1]`). Without a session cookie, a web page whose DNS name is
    switched to 127.0.0.1 ("DNS rebinding") would otherwise be same-origin with the API. With
    login on, the cookie is bound to the real host name and no `Host` rule applies, so a reverse
    proxy keeps working.

## Running it

- Console: `jarvis-api` (or `jarvis-api serve`), `--config <file>` or env `JARVIS_CONFIG`. Ctrl+C
  stops it; open WebSockets are closed and other connections get 10 s to finish.
- Windows service (an elevated prompt for install/uninstall/start/stop):
  `jarvis-api service install` registers the current executable as the service `JarvisMT`
  (automatic, delayed start; restarts after a failure after 5 s, 15 s, then 60 s) with the arguments
  `--config <absolute config path> service run --workdir <current directory>`, so install it
  from the directory that holds `config/`, `data/`, `models/` and `.env`. `service uninstall`,
  `service start` and `service stop` do what they say. The service writes a daily rolling log
  to `logs/jarvis-api.log.<date>` under the working directory. It runs as the virtual account
  `NT SERVICE\JarvisMT` (SVC-1) and `install` prints the `icacls` commands that give it its
  files; `service install --account <name>` picks another account (`--password-stdin` reads its
  password) and `--local-system` restores LocalSystem, with a warning. See
  [windows-service.md](windows-service.md).
- Startup refuses: DEMO mode on a real account, login enabled with no usable `JARVIS_JWT_SECRET`,
  a non-loopback bind without TLS and login, and an unreadable TLS certificate or key. A bridge that
  is down, a missing voice model or a missing LLM key only log a warning.

---

# Personas and user settings

Arc MT has selectable personas (Jarvis, Friday, and any pack added to `personas/`).
A persona changes the name, personality, voice, wake phrase and HUD colours only.
Tools, limits, previews, confirmation tokens, the kill switch and all safety rules are
shared and identical for every persona (ADR-0017). Every audit row records the persona
that was active (`persona` column).

## GET /api/personas

```json
{
  "default": "jarvis",
  "active": "friday",
  "personas": [
    { "id": "jarvis", "name": "Jarvis", "description": "Calm, formal and concise.",
      "wake_word": "hey jarvis", "voice_available": true,
      "theme": { "accent": "#38bdf8", "accent_soft": "#0c4a6e", "orb_core": "#7dd3fc", "orb_glow": "#0ea5e9" } },
    { "id": "friday", "name": "Friday", "description": "Warm, upbeat and casual.",
      "wake_word": "hey friday", "voice_available": true,
      "theme": { "accent": "#f5b942", "accent_soft": "#5c3d0a", "orb_core": "#fcd77f", "orb_glow": "#f59e0b" } }
  ]
}
```

`active` is the current user's chosen persona (from settings). `voice_available` is false
when that persona's Piper model file is missing.

## GET /api/settings

The signed-in user's preferences (with login off: one shared local profile).

```json
{
  "persona": "jarvis",
  "llm_provider": null,
  "voice": { "read_aloud": true, "input_mode": "hold", "language": "en" },
  "watchlist": ["XAUUSD", "EURUSD", "GBPUSD"],
  "aliases": { "yellow metal": "XAUUSD" }
}
```

- `llm_provider`: provider name from `/api/status`, or `null` for the server default.
- `voice.input_mode`: `"hold"` (press and hold), `"toggle"` (tap to start/stop) or
  `"wake_word"` (hands-free: speech segments are transcribed and only acted on when
  they start with a persona's wake phrase).
- `voice.language`: `"en"` or `"auto"`.
- `watchlist`: up to 30 symbols (aliases allowed; resolved like chat input).
- `aliases`: up to 100 extra spoken-name → broker-symbol entries, merged over the config
  aliases (a user entry wins). Keys are lower-cased; values `[A-Za-z0-9._#]{1,32}`.

## PUT /api/settings

Partial update: send only the fields to change; returns the full settings. Unknown
fields → 400. An unknown persona or provider → 400. Changing the persona takes effect
on the next chat turn; open chat sessions keep their history. Emits the WebSocket event
`{"type":"settings_changed"}`.

## Trading limits from the UI (tighten-only)

The config file sets the limits. The UI can only make them **stricter**.

- `GET /api/settings/limits` →
  `{ "config": Limits, "overrides": PartialLimits | null, "effective": Limits }` where
  `Limits` is the shape in `/api/status` and `effective` is what the gate enforces.
- `PUT /api/settings/limits` `{ "totp": "123456", "max_lot": 0.5, "max_positions": 3,
  "max_daily_loss": 200.0, "allowed_symbols": ["XAUUSD"], "trading_hours": {...} }`
  (all limit fields optional). Each value must be at least as strict as the config:
  `max_lot`, `max_positions`, `max_daily_loss` ≤ config; `allowed_symbols` ⊆ config;
  `trading_hours` days ⊆ config days and the window inside the config window (same-day
  windows only). Anything looser → 422 `limit_not_stricter`. Needs a valid TOTP code
  when login is on (401 `unauthorized` "code not accepted" otherwise). The fields sent are
  merged into the stored overrides (fields left out keep their override); an empty body
  is 400. Malformed (400) and looser (422) requests are refused before the TOTP check, so
  they do not use up the code. A change emits `settings_changed` and writes a
  `limits_changed` audit row.
- `DELETE /api/settings/limits` `{ "totp": "123456" }` → removes the overrides (back to
  the config values). Needs a TOTP code when login is on.

Overrides are stored in SQLite and apply to every persona and session. Previews already
issued keep the limits they were checked against; the confirm-time re-check (TRD-2) uses
the effective limits at that moment.

## Changes to existing endpoints

- `GET /api/status` gains `"persona": {"id": "jarvis", "name": "Jarvis"}` (active) and
  `limits` now reports the effective limits.
- `POST /api/chat` uses the active persona; the response gains `"persona": "jarvis"`.
- `POST /api/voice/speak` speaks with the active persona's voice (or `{"persona": "id"}`
  in the body to preview another persona's voice in settings).
- `POST /api/voice/transcribe?wake=1`: in wake-word mode the response gains
  `"wake": {"detected": true, "persona": "friday", "command": "what's gold at"}`.
  A detected wake phrase also switches the active persona to the one that was called,
  except while a confirmation is open (the phrase is still detected and its command used,
  but the persona stays the one the confirmation started with).
  Without a wake phrase the object is `{"detected": false, "persona": null, "command": ""}`
  and the UI ignores the segment.
- `GET /api/audit` entries gain `"persona"` (null for rows written before personas).
- `GET /api/watchlist` uses the caller's watchlist and aliases from settings.
- If saved limit overrides cannot be read at startup, new orders are refused until they
  are reset with `DELETE /api/settings/limits` (fail closed, like the kill switch).

---

# Broker symbol mapping

Arc MT uses **canonical** instrument names everywhere (`XAUUSD`, `US100`, `US30`,
`EURUSD`, ... from `config/instruments.toml`); each broker's own names (`XAUUSD_`,
`US100_Spot`, `USTEC`, `UT100Roll`, `EURUSD.pr`, `GOLD_CASH`) are mapped per broker,
keyed by the MT5 company name (`ACCOUNT_COMPANY`). Resolution order: the user's
overrides > curated `config/broker_symbols.toml` > auto-detected > the name unchanged.

Detection runs automatically at startup and whenever the bridge reports a different
broker (an account switch in MT5): the API fetches the broker's full catalog
(`get_broker`, `list_symbols`), runs the auto-mapper, saves the result to
`data/broker_symbols.generated.toml` and a catalog snapshot to
`data/broker-catalogs/<broker>.json`, writes a `symbol_map_detected` audit row and
emits the WebSocket event `{"type":"symbol_map_changed","broker":"..."}`. Limits
(`allowed_symbols`), the watchlist, aliases and chat all keep using canonical names, so
they keep working across brokers.

## GET /api/symbols

```json
{
  "broker": { "company": "CFI Financial Markets L.L.C", "server": "CFI3-Demo",
              "detected_at": "2026-10-30T12:00:00Z", "catalog_size": 1834,
              "detected_suffix": "_", "detected_prefix": "" },
  "mappings": [
    { "canonical": "XAUUSD", "class": "metal", "aliases": ["GOLD", "XAU"],
      "broker_symbol": "XAUUSD_", "source": "curated", "available": true,
      "candidates": [ { "broker_symbol": "XAUUSD_", "description": "Gold", "score": 9,
                        "reasons": ["visible", "tradable", "matches detected suffix"] } ] },
    { "canonical": "US100", "class": "index", "aliases": ["NASDAQ", "NAS100", "USTEC"],
      "broker_symbol": "US100_Spot", "source": "generated", "available": true,
      "candidates": [] }
  ],
  "unmapped": ["HK50"]
}
```

`source` is `user` | `curated` | `generated` | `pattern` | `passthrough`. `available` is
false when the resolved name is not in this broker's catalog (the entry needs attention).
`broker` is `null` while the bridge has never been reachable.

## PUT /api/symbols/map

`{ "canonical": "US100", "broker_symbol": "NAS100.cash" }` sets a user override for the
current broker (`broker_symbol` must exist in its catalog → else 422
`unknown_broker_symbol`); `"broker_symbol": null` removes the override. Saved to
`data/broker_symbols.user.toml`. Emits `symbol_map_changed`; audited `symbol_map_override`.
Needs a TOTP code (`"totp"`) when login is on, because a wrong mapping could send an
order to the wrong instrument. Errors: 400 `bad_request` (unknown canonical name, or
`broker_symbol` missing: send a name or `null`), 401 (login/TOTP), 422
`unknown_broker_symbol`, 409 `broker_changed` (the account switched while the request was
being checked), 503 `bridge_unavailable` (no broker known yet). The response body is the
same as `GET /api/symbols` (the refreshed state).

## POST /api/symbols/detect

Re-runs detection for the current broker now (e.g. after the broker added symbols).
Needs a session but no TOTP. Returns the same shape as `GET /api/symbols` and emits
`symbol_map_changed`. 503 `bridge_unavailable` when the bridge is down.

## GET /api/symbols/catalog?q=nas&limit=50

Search the current broker's catalog snapshot (name or description, case-insensitive):
`{"symbols":[{"name","description","path","trade_mode","visible"}],"total"}`. Used by
the settings page to pick an override.

## GET /api/symbols/export

The effective map for the current broker as a downloadable JSON file
(`{"broker", "server", "generated_at", "map": {"XAUUSD": "XAUUSD_", ...}}`).

## Changes to existing endpoints

- `GET /api/status` gains `"broker": {"company", "server", "mapped", "total"}`.
- Watchlist, positions and history rows gain `"canonical"` next to the broker `symbol`.
- `[symbols] suffix` in the config is now only a last-resort fallback for brokers with
  no map at all; `[limits] allowed_symbols` and `[symbols] watchlist` hold canonical
  names.

Details:

- `class` is one of `fx | metal | index | energy | crypto | other`; `trade_mode` in the
  catalog results is `full | long_only | short_only | close_only | disabled`; `unmapped` is a
  list of canonical names (instruments whose resolved name the broker does not have).
- `GET /api/status` `broker.mapped` counts instruments that resolve to a symbol the broker
  has; `broker.total` is the number of canonical instruments. `broker` is `null` until the
  bridge has reported a broker.
- `GET /api/symbols/catalog`: `total` is the number of matches before `limit` (default 50,
  maximum 500). With no snapshot yet: `{"symbols": [], "total": 0}`.
- `symbol_map_changed` (`{"type":"symbol_map_changed","broker":"<company>"}`) is sent after a
  detection (start-up, account switch, `POST /api/symbols/detect`), after a user override is
  set or cleared, and when the account switches to another broker.
- Selectors that take a symbol (`POST /api/positions/close` `symbol`, chat tools, the
  watchlist, `allowed_symbols`) accept a canonical name, a generic alias or the broker's own
  symbol; a canonical name is resolved through the current broker's map, a name that is
  already a symbol in the broker's catalog is used as is.
- A preview is bound to the broker account (company and trade server) it was made on.
  `POST /api/confirm/{token}` answers 409 `broker_changed` if MT5 is on another account
  by then; nothing is sent and the token is cancelled.
- Positions, orders and watchlist prices rows carry `"canonical"`; the chat tools return both
  the broker `symbol` and `canonical`.

---

# Market view

Intraday views from the broker's bars, the MT5 economic calendar and optional news headlines. A view is information: it never creates a preview or sends an order by itself. How it is computed is in [market-view.md](market-view.md); the decision is [ADR-0019](adr/0019-market-views.md). The routes need a login session like the other `/api/*` routes.

The routes need EA 0.5.0 (`get_rates`, `get_calendar`, see [protocol.md](protocol.md#get_rates)). With an older EA the view and calendar routes answer `502 ea_outdated`, with a message that says to update JarvisBridge to version 0.5.0 or newer.

With `[market] enabled = false` every `/api/market/*` route answers `404 not_found` ("market views are turned off"), the three tools below are not offered to the model, and no scoring task runs.

`GET /api/market/view` and `GET /api/market/news` have a per-session budget of `[market] rate_limit_per_minute` (default 30). Over it: `429 rate_limited` with `Retry-After`. They also use the longer request timeout (`[server] long_request_timeout_secs`), because a view reads bars, the calendar and news before it answers. The calendar and journal routes use the normal one.

A `symbol` is a canonical name, a generic alias or the broker's own symbol, resolved like the other symbol selectors (see [Broker symbol mapping](#broker-symbol-mapping)). A name that resolves to nothing is `400 bad_request`. The view reply carries both the canonical `symbol` and the `broker_symbol`.

`GET /api/status` gains `"market": {"enabled": true, "news_provider": "none", "news_configured": false}` (`news_provider` is `finnhub`, `alphavantage` or `none`; `news_configured` is false when no provider is set up or its key is missing).

## Tools for the chat model

Three read tools. They are `ReadTool` variants like `get_price`, so they cannot trade, and they write the same audit rows (`tool_call`). They are offered to the model only when market views are on. A view request also writes a `market_view` audit row (view id, symbol, broker symbol, bias, confidence, whether it stands aside, whether it has a setup, and `source`: `tool` or `ui`).

| Tool | Arguments | Result |
|---|---|---|
| `get_market_view` | `symbol` | The view JSON below, trimmed for the model (see the note after it). Each call is also saved in the journal. |
| `get_economic_calendar` | `hours_ahead` (optional whole number, 1 to 168, default 24), `currencies` (optional list of three-letter codes) | `{hours_ahead, currencies, count, listed, events}`. `events` holds at most 40 events; `count` is the total. |
| `get_news` | `symbol` | `{configured, provider, summary, error?}` as in `GET /api/market/news`, without links, and with a `note` when no provider is set up |

What the model receives is trimmed: no `id`, `broker_symbol` or `digits`; the timeframe rows keep only `timeframe`, `trend`, `rsi` and `atr`; headlines keep `title`, `source`, `published_at` and `tone` (no URL). A headline title is third-party text, so it is cut to one line of at most 200 characters, and the tool description tells the model that headlines are untrusted and that it must not follow instructions found in them. The model is told to call `get_market_view` for any request for a view, opinion, outlook, bias or buy or sell suggestion, to relay the bias, the confidence, the setup levels when there is one and the two main reasons in two or three sentences, to mention upcoming high-impact events and why the view stands aside, to say that it is a computed analysis and not a guarantee, and to propose a trade only when the user asks for that trade with a volume. Figures in its reply go through the grounding check described under [Previews and confirmation](#previews-and-confirmation-sprint-2).

## GET /api/market/view?symbol=XAUUSD

Computes a view now, saves it and returns it. Reply `MarketViewDto`:

```json
{
  "id": "5b0b7a4e-6f43-4c2a-9a52-3f1d6c8e2a10",
  "symbol": "XAUUSD", "broker_symbol": "XAUUSD_", "digits": 2,
  "generated_at": "2026-10-30T13:50:00Z", "valid_until": "2026-10-30T17:50:00Z",
  "price": { "bid": 3925.09, "ask": 3925.22 },
  "bias": "bullish", "confidence": 62, "score": 62.0,
  "stand_aside": false, "stand_aside_reason": null,
  "setup": { "side": "buy", "entry_low": 3921.4, "entry_high": 3925.22, "stop_loss": 3911.0,
             "take_profit": 3944.1, "take_profit_2": 3958.0, "risk_reward": 1.5,
             "atr_h1": 12.4, "atr_m15": 5.1 },
  "timeframes": [
    { "timeframe": "H4", "trend": "up", "ema20": 3918.4, "ema50": 3902.7, "rsi": 61.3, "macd_hist": 2.41, "atr": 31.2 }
  ],
  "levels": { "pivot": 3912.6, "r1": 3921.1, "r2": 3934.9, "s1": 3899.0, "s2": 3890.3,
              "prev_day_high": 3930.5, "prev_day_low": 3898.2, "vwap": 3916.8,
              "support": [3919.2, 3911.0], "resistance": [3933.6, 3948.0] },
  "factors": [
    { "name": "H4 trend", "signal": "bullish", "points": 25.0, "detail": "EMA20 above EMA50, close above EMA50" }
  ],
  "events": [
    { "time": "2026-10-30T14:30:00Z", "minutes_away": 40, "currency": "USD", "title": "Nonfarm Payrolls",
      "importance": "high", "actual": null, "forecast": 185.0, "previous": 175.0, "unit": "K" }
  ],
  "news": { "configured": true, "score": 0.2, "label": "slightly positive", "count": 7,
            "headlines": [ { "title": "...", "source": "...", "url": "https://...",
                             "published_at": "2026-10-30T12:10:00Z", "tone": 0.4 } ] },
  "summary": "Bullish intraday bias, confidence 62 of 100: H4 and H1 trends are up. Buy zone 3921.40 to 3925.22, stop 3911.00, target 3944.10.",
  "disclaimer": "Computed from price data, indicators, the economic calendar and headlines. It is not financial advice and no outcome is guaranteed.",
  "notes": []
}
```

(Shortened: `timeframes` has a row for H4, H1 and M15, and `factors` one entry per factor that could be computed. The numbers are illustrative.)

- `bias` is `bullish`, `bearish` or `neutral`. `score` is -100 to 100 and `confidence` 0 to 100. `factors[].signal` uses the same three words, and `points` is the signal times the weight. The factor names are `H4 trend`, `H1 trend`, `M15 momentum`, `M15 RSI`, `VWAP`, `Daily pivot` and `News`.
- `setup` is `null` when the bias is neutral, when `stand_aside` is true, or when there is not enough data. `side` is `buy` or `sell`. For a sell the zone is `[bid, bid + 0.5 * ATR(M15)]`. `risk_reward` is the distance to `take_profit` over the distance to `stop_loss`.
- `stand_aside_reason` is plain text (for example `High-impact USD event "Nonfarm Payrolls" in 12 minutes`) and joins two reasons with `; `.
- `levels` fields are `null` (and the lists empty) where the data was missing. `timeframes[].trend` is `up`, `down` or `mixed`.
- `events` are the calendar events of the instrument's currencies at or above `[market] calendar_min_importance`, from 60 minutes ago to 24 hours ahead, sorted by time. `minutes_away` is negative once an event has happened. `time` is ISO 8601 UTC. `actual`, `forecast`, `previous` and `unit` are `null` when MT5 has no value. Events in a view have no `country`.
- `news.configured` is false when no provider is set up (or its key is missing or was refused), with `label` `"not configured"`, `count` 0 and no headlines. When a provider is set up but did not answer (an error, or no reply within about 4 seconds), `configured` is true, `label` is `"unavailable"`, and `notes` says so. `headlines` holds up to 5 items. Headline text is external and must be shown as plain text.
- `valid_until` is `generated_at` plus `[market] horizon_minutes` (4 hours by default).
- `notes` lists, in plain words, what was skipped or missing: a factor that lacked history, a calendar that could not be read, news that was unavailable.

Errors: `400 bad_request` (missing, empty or unusable `symbol`), `429 rate_limited`, `502 ea_outdated`, `503 bridge_unavailable`, and `502 bridge_error` (the EA's refusal, with its MT5 code in the message, for example unknown symbol, or history not loaded after the one retry). A calendar that fails for any other reason does not fail the view: it is computed without events and `notes` says so.

## GET /api/market/calendar?hours=24&currencies=USD,EUR

`hours` is 1 to 168 (default 24); anything else is `400 bad_request`. `currencies` is an optional comma list of three-letter codes (case-insensitive); anything that is not three letters is `400 bad_request`. Events run from 1 hour ago to `hours` ahead, and come from the MT5 calendar at `[market] calendar_min_importance` or above.

```json
{
  "events": [
    { "time": "2026-10-30T14:30:00Z", "minutes_away": 40, "currency": "USD", "country": "US",
      "title": "Nonfarm Payrolls", "importance": "high",
      "actual": null, "forecast": 185.0, "previous": 175.0, "unit": "K" }
  ]
}
```

`actual`, `forecast`, `previous` and `unit` are `null` when MT5 has no value. At most 500 events are returned (the EA's cap). Errors: `400 bad_request`, `502 ea_outdated`, `503 bridge_unavailable`, `502 bridge_error`.

## GET /api/market/news?symbol=US100

```json
{
  "configured": true,
  "provider": "finnhub",
  "summary": {
    "score": -0.18, "label": "slightly negative", "count": 7,
    "headlines": [ { "title": "...", "source": "...", "url": "https://...",
                     "published_at": "2026-10-30T12:10:00Z", "tone": -0.5 } ]
  }
}
```

With no provider: `{"configured": false, "provider": null, "summary": null}`. `score` is -1 to 1 in the instrument's direction, `count` is the number of relevant headlines in the last 24 hours, and `headlines` holds at most 5.

A provider failure (rate limit, timeout after about 4 seconds, an outage) is **not** an HTTP error: the reply is `200` with `"configured": true`, the provider name, `"summary": null` and an `"error"` string. If a cached summary younger than an hour exists, it is returned instead. A key the provider rejects makes the reply `configured: false`. Errors: `400 bad_request` (missing or unusable `symbol`), `429 rate_limited`.

## GET /api/market/journal?limit=20&symbol=XAUUSD

The saved views, newest first, with their outcomes, and the track-record statistics. `limit` is 1 to 200 (default 20; a value outside it is clamped). `symbol` is optional and is resolved to its canonical name; it filters both `views` and `stats`.

```json
{
  "stats": {
    "total": 42, "setups": 24, "open": 1,
    "tp_hit": 9, "sl_hit": 8, "expired": 6, "no_setup": 18,
    "bias_correct": 22, "bias_wrong": 16,
    "tp_rate": 0.391, "bias_accuracy": 0.579
  },
  "views": [
    { "id": "5b0b7a4e-6f43-4c2a-9a52-3f1d6c8e2a10", "created_at": "2026-10-30T13:50:00Z",
      "symbol": "XAUUSD", "broker_symbol": "XAUUSD_",
      "bias": "bullish", "confidence": 62, "score": 62.0, "stand_aside": false,
      "side": "buy", "entry": 3923.31, "stop": 3911.0, "tp": 3944.1,
      "price": 3925.155, "valid_until": "2026-10-30T17:50:00Z",
      "outcome": "tp_hit", "evaluated_at": "2026-10-30T15:05:00Z",
      "bias_correct": true, "move_atr": 0.9 }
  ]
}
```

(Illustrative numbers.)

- `stats.setups` counts views that had a setup. `tp_rate` is `tp_hit` divided by the setups that have closed (`tp_hit` + `sl_hit` + `expired`), and `bias_accuracy` is `bias_correct` divided by `bias_correct` + `bias_wrong`. Both are rounded to 3 decimals and are `null` when nothing counts yet.
- In a row, `side`, `entry`, `stop` and `tp` are `null` for a view without a setup. `entry` is the middle of the entry zone and `tp` is take profit 1. `price` is the mid of bid and ask when the view was made.
- `outcome` is `open`, `tp_hit`, `sl_hit`, `expired` or `no_setup`. `evaluated_at` is when the row last changed. `bias_correct` and `move_atr` are `null` until the bias is judged at `valid_until`. `move_atr` is the move in H1 ATRs, positive when the price went the way the bias said (for a neutral view, positive when it went up).

See [Track record](market-view.md#track-record) for how views are scored. The views are stored in the `market_views` table (migration `0007_market_views.sql`). The scoring task runs at start-up and then every 5 minutes.

## POST /api/market/prepare

Turns a view with a setup into an order proposal. It does not send anything and does not confirm.

```json
{ "view_id": "5b0b7a4e-6f43-4c2a-9a52-3f1d6c8e2a10", "volume": 0.1, "order_type": "market" }
```

- `view_id` must be a saved view that has not expired and has a setup.
- `volume` is required, in lots, and must be above 0. There is no default (core rule 9).
- `order_type` is required: `market` or `limit`. A `market` order uses the setup's stop loss and take profit 1. A `limit` order is placed at `entry_low` for a buy or `entry_high` for a sell, with the same stop loss and take profit 1. Take profit 2 is not sent.

The reply is a `PreviewDto`, the same shape as every other preview (see [Previews and confirmation](#previews-and-confirmation-sprint-2)), with a confirmation `token` and its 15-second window. The proposal goes through `Trading::propose` with the source set to the UI, so **every limit and the kill switch apply**: `max_lot`, `max_positions`, `max_daily_loss`, `allowed_symbols`, trading hours and the stops-distance checks. The broker symbol is looked up again at that moment, so a view made on another account maps to the current one. Confirming is the normal `POST /api/confirm/{token}`. A `market_prepare` audit row records the view, the volume and the order type before the gate decides.

| Status | Code | When |
|---|---|---|
| 400 | `bad_request` | invalid JSON, a missing `view_id`, a missing or non-positive `volume`, a missing or unknown `order_type`, or a symbol that no longer resolves |
| 404 | `not_found` | no such view |
| 410 | `view_expired` | the view is past its `valid_until` |
| 409 | `broker_changed` | the MT5 account changed while the proposal was prepared |
| 422 | `no_setup` | the view stood aside, or its bias was neutral, so it has no setup |
| 422 | `gate_rejected` | a limit or a broker rule refused the proposal (the message says which) |
| 423 | `kill_switch` | the kill switch is engaged |

## Configuration

`[market]` and `[market.news]` are described in [configuration.md](configuration.md#market). The audit kinds added are `market_view` and `market_prepare`.
