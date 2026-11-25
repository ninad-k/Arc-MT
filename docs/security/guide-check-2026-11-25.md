# Security check against an external testing guide, 2026-11-25

Arc MT checked against every test in
[GizzZmo/Master-Prompt-Editor `SECURITY_TESTING_GUIDE.md`](https://github.com/GizzZmo/Master-Prompt-Editor/blob/main/SECURITY_TESTING_GUIDE.md).
That guide was written for a Node/Express app (`/api/prompts`, port 3001). Each test below was
translated to the matching Arc MT route and run live.

**Method.** A separate test instance ran from the debug build of `feat/architecture-page` (`d32e357`) on
`127.0.0.1:18787`. It used the mock bridge on port 15555, an empty scratch database, no `.env` and no
provider keys. The only LLM was a dead local endpoint, so nothing was billed and no real broker was touched.
The probes ran twice: once with login on (no users) and once with login off (loopback only). The
dependency audits ran on the real lockfiles. Static review covered `security.rs`, `routes.rs`,
`ratelimit.rs`, `auth/`, `serve.rs`, the SPA and the Tauri config.

## Result per guide section

| # | Guide test | Arc MT result | Verdict |
|---|---|---|---|
| 1 | Rate limiting: general 100 per 15 min | No global limit. 105 of 105 `GET /api/health` returned 200. | Gap G1, **fixed** (NET-3) |
| 1 | Rate limiting: AI generation 20 per 5 min | `POST /api/chat`: 20 got through, then 5 got 429 (per-session budget, 20/min). The same budget covers ask, learning review and from-image. | Pass |
| 1 | Rate limiting: sensitive ops 10 per 10 min | Login: 5 per minute per IP, then 429 with `Retry-After: 59`, plus account lockout. Confirm tokens: 10/min, then 429. TOTP-gated releases: 5/min. `DELETE /api/settings/limits` with login off: 15 of 15 returned 200. | Pass (with login off, fixed by G1) |
| 1 | 429 and rate-limit headers | 429 in the standard JSON error shape, with `Retry-After`. No `RateLimit-*` headers. | Pass (info) |
| 2 | XSS `<script>` payload | Watchlist entry stored as `SCRIPTALERT1SCRIPT` (normalised). Symbol and instrument fields were rejected (400/422). The SPA has no `{@html}`, `innerHTML` or `eval` (the only `new Function` is in the sandboxed 3D worker, by design). | Pass |
| 2 | SQL injection `"; DROP TABLE …; --` | Rejected as an unknown persona, symbol or user. All SQL goes through sqlx with `?` binds. The one dynamic query (`decisions.rs:960`) only appends fixed fragments. | Pass |
| 2 | Oversized content (100 KB) | 413 `payload_too_large` on login and settings (64 KiB JSON cap). Uploads and voice have their own caps, checked before the body is read. | Pass |
| 2 | Invalid IDs | `abc`, `-1`, a 23-digit overflow and `1' OR 1=1` as a ticket: all 400. A bogus confirm token: 404 `token_unknown`. `../` in model or note IDs: 404/400. Static-site traversal (plain and `%2e%2e`): 404. | Pass |
| 3 | `X-Content-Type-Options: nosniff` | Present on every response. | Pass |
| 3 | `X-Frame-Options: DENY` | `DENY`, and `SAMEORIGIN` only on `/sandbox/model.html`. | Pass |
| 3 | `X-XSS-Protection: 1; mode=block` | Not sent. OWASP now advises against it: it is ignored by current browsers and could create leaks in old ones. CSP replaces it. | Pass (intentional deviation) |
| 3 | `Strict-Transport-Security … preload` | `max-age=31536000; includeSubDomains`, only when serving TLS. No `preload`, which is correct for a self-hosted app on local or private host names. | Pass (intentional deviation) |
| 3 | Content-Security-Policy | Strict: `script-src 'self'` plus hashes, no `unsafe-inline` or `unsafe-eval`, `connect-src 'self'`, `frame-ancestors 'none'`, `object-src 'none'`. The sandbox page has its own `default-src 'none'; sandbox allow-scripts`. Also sends Referrer-Policy, Permissions-Policy, COOP and `Cache-Control: no-store` on `/api`. | Pass (exceeds the guide) |
| 4 | CORS from `https://malicious-site.com` | Preflight returns no `Access-Control-Allow-Origin`, so the browser blocks it. A cross-origin POST gets 403 `origin not allowed`. The same applies to a sibling port (`localhost:5173`) and to `Origin: null`. A WebSocket upgrade from a foreign Origin gets 403 (same origin gets 101). | Pass |
| 5 | Error boundaries | No `+error.svelte`, no `<svelte:boundary>`, no `hooks.client.ts` `handleError`. All `localStorage` reads are wrapped in try/catch, so corrupted storage is handled. | Gap G2, **fixed** |
| 6 | Audit log of sensitive ops | Kept in SQLite and read through `GET /api/audit`, which needs a session (401 without one). Each entry has a timestamp, kind, result and actor. Failed logins carry the IP and reason, and the username is deliberately not recorded. Covered: confirms, limit changes, rate-limited logins (one entry per window). | Gaps G3, **fixed** (AUD-1) |
| — | Auth on every endpoint (guide does not cover it) | With login on, `/api/audit`, positions, account, killswitch, limits, `ws`, unknown `/api/*` paths and a forged `alg:none` JWT cookie all got 401. | Pass |
| — | DNS rebinding (login off) | `Host: evil.example` and `Host: 127.0.0.1.evil.io` got 403. `localhost` is allowed. | Pass |

## Automated tools (guide: npm audit, ZAP, Burp)

| Tool | Result |
|---|---|
| `pnpm audit` (web, apps/desktop) | No known vulnerabilities |
| `cargo deny check advisories bans sources` | advisories ok, bans ok, sources ok |
| `cargo audit` (workspace) | 2 medium, both documented. RUSTSEC-2026-0459 (bundled SQLite; ignored in `deny.toml` with a reason, review by 2027-01-31). RUSTSEC-2023-0071 (`rsa`, Marvin attack): in `Cargo.lock` only through sqlx's MySQL driver and not compiled (`cargo tree -i rsa` is empty). CI ignores both explicitly. |
| `cargo audit` (desktop lockfile) | 0 vulnerabilities; 6 unmaintained or unsound warnings in transitive Tauri/GTK crates |
| CI (`security.yml`) | cargo-deny, cargo-audit, pnpm audit (high), gitleaks (full history), Semgrep and CodeQL (SARIF), plus OpenSSF Scorecard. This already covers the guide's "monthly scans". |
| ZAP / Burp | Not run (not installed). The curl probes above cover the guide's ZAP and Burp targets (XSS, SQLi, CSRF, auth bypass). |

## Gaps found, and how they were fixed

All three were fixed on 2026-11-25 (uncommitted on `feat/architecture-page`). See "Re-test after
the fixes" below.

**G1 (low): no global request limit. Fixed as NET-3.** Only login, chat, market, transcribe, confirm
and TOTP release were limited. Cheap routes that still call the bridge (`/api/positions`,
`/api/account`, `/api/orders`) and non-TOTP writes (`PUT /api/settings`) were not. With login on, only
a signed-in session could reach them; with login off, any local process could.
*Fix:* every route behind the session check now shares one budget per session (per peer address with
login off): `[server] api_rate_limit_per_minute`, default 600, range 60 to 100000. Going over it gets
`429` with `Retry-After`. It sits on top of the existing per-route budgets (`Budget::Api` in
`state.rs`, layered in `routes.rs`). `/api/health` and `/api/auth/login` are outside it; login keeps its
own 5/min limit.

**G2 (low): no app-specific error screen. Fixed.** SvelteKit's built-in fallback already caught crashes,
but it showed only a bare status line ("500 / Internal Error"). Nothing was logged with route context,
and nothing told the trader that the server and trading were unaffected.
*Fix:* `web/src/routes/+error.svelte`, backed by `CrashScreen.svelte` and `lib/crash.ts` (unit-tested),
shows a plain message with Try again and Home. It says the server, positions, limits and kill switch
are unaffected, and never shows raw exception text. `web/src/hooks.client.ts` logs each error with its
kind, route and URL. A `<svelte:boundary>` in the layout was tried and removed: SvelteKit 3 already
routes page render errors, even ones after mount, through `handleError` to `+error.svelte`, so the
boundary never fired.

**G3 (low): audit coverage. Fixed as AUD-1.**
- The client IP was recorded only on `login_failed`. *Fix:* every row written while a request is handled now carries `ip`. A task-local is set by an outermost middleware (`audit::with_client_ip`); background work records none.
- A successful `PUT /api/settings` wrote no row. *Fix:* new `settings_changed` row with `{by, patch}`.
- Refused requests appeared only in the process log. *Fix:* new `request_refused` row for a foreign `Origin`, a non-loopback `Host`, or a used-up budget. It holds method, path (tokens redacted), origin and host, or budget and `retry_after_secs`. To stop a flood of refusals flooding the table, there is at most one row per reason and caller per minute.

## Re-test after the fixes

- **Tests:** `cargo test -p jarvis-api` passes in full, including the TLS, auth, uploads and server tests that could not link before. The build went into a target folder on C:. New tests are in `crates/api/tests/limits.rs`: the shared budget, per-address budgets without login, the settings audit, throttled refusal rows, token redaction, and the client IP. One existing test, `http.rs`, now expects exactly one `request_refused` row where it used to expect none.
- **Checks:** `cargo clippy -D warnings` and `cargo fmt --check` are clean. `pnpm check` and `pnpm test` pass (309 tests, 6 new), and `pnpm build` is clean.
- **Live probes** (new build, fresh database, mock bridge, login off):
  - 650 × `GET /api/settings`: 600 returned 200 and 51 returned 429.
  - The 15 × `DELETE /api/settings/limits` that earlier all succeeded now got 429 with `Retry-After: 55`.
  - `/api/health`: 105 of 105 returned 200 (public, not counted).
  - 3 foreign-origin and 3 bad-Host refusals wrote one `request_refused` row each, as did the budget refusal. Every new row had `"ip": "127.0.0.1"`.
- **Browser** (mock dev server):
  - An unknown route shows the 404 screen.
  - A page that threw after mounting showed the crash screen and logged `[arc-mt] navigation failed {kind: unknown, route, name: TypeError, …}`. This used a temporary probe page, since deleted.
  - Home and Settings render as before.
- **Not run:** the Playwright e2e suite. Its Chromium build is not installed, so all 121 tests failed at browser launch before any assertion; installing it is a separate download.

**Info.** Login off means no TOTP on sensitive writes. That is by design for loopback-only use
(ADR-0020), and the Host guard blocks DNS rebinding. Keep login on for any setup that leaves the machine.

## Not covered

- The guide's external pen-test items (firewall, network segmentation, open ports, TLS cipher scan) were not run live. The code review shows rustls, and the server refuses a non-loopback bind unless both TLS and login are on.
- On the first pass the TLS, auth, upload and server integration tests did not link: drive D: had 0.1 GB free (`target/` is about 59 GB). They were run after the fixes, from a target folder on C: (see above).
- The MQL5 EA and the real broker were out of scope (see [audit-2026-10-30.md](audit-2026-10-30.md)).
