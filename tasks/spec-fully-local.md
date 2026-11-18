# Spec: fully local mode (latency, preflight, egress proof, voice)

Status: approved and built 2026-11-11. Decision record: [ADR-0020](../docs/adr/0020-fully-local-mode.md).
User guide: [docs/local-mode.md](../docs/local-mode.md).

Changes made while building (the sections below are updated to match):

- The egress route filters with `since=<seq>` instead of a per-turn id: entries carry a
  monotonic `seq`, which needs no turn plumbing and serves the preflight just as well.
- `purpose` is `llm:<provider>` or `news:<provider>`. VoiceStudio is not logged: its URL is
  refused unless it is loopback, so it can never be a remote host.
- An Ollama model whose tag ends in `cloud` counts as not local (Ollama forwards it to its own
  servers through `127.0.0.1`, where the egress log cannot see it). Found while writing the
  "what needs the internet" list.
- The shared rules also lost their per-line indentation and stray space runs (tokens on every turn).

Source: ideas adapted from the "Run Jarvis Fully Local" prompt pack (Zubair Trabzada, Oct 2026),
which builds a separate notes assistant. Only the ideas that fit a trading tool are taken here,
rewritten for Arc MT. The pack itself is not copied into the repository.

## Objective

Arc MT already runs locally by default (Ollama, Whisper, Piper; the web app uses no browser speech
service and no CDN). What is missing is **speed you can measure** and **proof that nothing leaves
the PC**. A trader on a normal Windows PC should be able to:

1. Run one command and see a verdict for every live chain (API, model, voice out, voice in,
   served files, secrets) plus a latency budget: `hear X s, think Y s, speak Z s, total T s`.
2. See every outbound connection the backend opened, with its host and reason, and run a drill
   that fails if any host other than loopback was contacted while in local mode.
3. Get faster local replies: a system prompt the model can cache between turns, a model that is
   loaded before the first question, and no empty replies from a model that "thought" until its
   token budget ran out.
4. Hear long replies in full, without gaps: sentence-packed chunks where the next one is fetched
   while the current one plays.
5. Optionally use a cloned voice from VoiceStudio (local, port 3900) instead of Piper, with Piper
   as the automatic fallback.

Out of scope: the pack's notes galaxy and "remember that" capture, cloud speech services, a second
Whisper model for the wake word, spoken barge-in ("stop"/"wait" while speaking) and spoken brain
switching. The last three are listed under "Later candidates".

## Pack ideas mapped to Arc MT

| Pack idea | Arc MT today | This spec |
|---|---|---|
| Local brain via Ollama, `reasoning_effort: none` | Ollama `/api/chat` with `think: false`, `<think>` stripping | FL-2: retry once on an empty reply cut by the token limit; preflight shows if it happens |
| Stable prompt prefix so the model reuses its cache | System prompt **starts** with the current time to the second, so no turn can reuse the cache | FL-1: fixed rules first, clock last |
| Keep the model warm | Cold first reply about 100 s | FL-2: `keep_alive` and a warm-up request at boot |
| Local ears (Whisper), no browser speech | Done (whisper-rs, server side) | Preflight checks it (FL-4) |
| Cloned voice, pinned engine, fallback | Piper per persona | FL-6: optional VoiceStudio engine, Piper fallback |
| Chunked, prefetched playback; never cut mid-sentence | One clip, truncated at 600 characters | FL-5 |
| WAV cache for repeated lines | None | FL-5: in-memory cache (not disk: spoken lines contain account figures) |
| `?mute=1` test tabs never speak | None | FL-5: one play function checks it |
| Preflight with latency budget | `scripts/status.ps1` (health only, no live chains, no timings) | FL-4: `jarvis-api preflight` |
| Outbound-connection watcher and Wi-Fi-off drill | None | FL-3 (egress log) and FL-4 (drill check) |
| No CDN in the page | Already true (Vite bundle) | FL-4 checks the built files stay that way |
| "Honest list" of what needs the internet | Scattered in README | FL-7: one list in `docs/local-mode.md`, checked against the code |

## Assumptions (correct me now)

1. Target is Windows 11 (this PC). VoiceStudio's fast CSM engine is Apple-only, so on Windows
   FL-6 is optional and off by default; Piper stays the default voice.
2. The preflight is a subcommand of the existing binary (`jarvis-api preflight`), not a Python
   script, so it reuses the config loader, providers and voice code and needs no new runtime.
3. The egress log is in memory only (a bounded ring buffer), never written to disk, and reachable
   only through an authenticated API route, like the audit API.
4. The VoiceStudio client uses `reqwest`, which is already a workspace dependency (used by
   `llm` and `news`). Adding it to the `voice` crate is the only dependency change.
5. No new paid services or cloud resources. Everything here is free and local.

## Contracts

### FL-1 Stable prompt prefix (`crates/api/src/agent.rs`)

- `build_shared_rules` puts the fixed text first and the clock line last:
  `...rules...\n\nCurrent time: {utc} UTC. MT5 broker server time: {server}.`
- The persona frame from `compose_system_prompt` stays in front of the shared rules. Nothing that
  changes between turns (time, aliases edited at runtime) may appear before the fixed rules.
- Grounding keeps getting the time strings separately (unchanged).

### FL-2 Ollama: keep warm, no empty replies (`crates/llm/src/ollama.rs`, `crates/api`)

- New optional provider key `keep_alive` (string, Ollama duration such as `"30m"`, default
  `"30m"` for `kind = "ollama"`), sent as `keep_alive` in every `/api/chat` body.
- New `[llm] warm_up = true` (default true): at boot, in the background, send one tiny request to
  the default provider if it is Ollama. Boot never waits for it; a failure is a warning in the log.
- When a reply has no text, no tool calls and `done_reason = "length"`, retry once with
  `num_predict` doubled (capped at 4096). A second empty reply is returned as it is today.

### FL-3 Egress log (`crates/api/src/egress.rs`, new)

- `EgressLog`: ring buffer of the last 200 entries
  `{seq, ts_ms, scheme, host, port, purpose, ok, ms, local}`. `purpose` is `llm:<provider>` or
  `news:<provider>` (the warm-up goes through the provider, so it is `llm:<provider>` too).
- Every outbound HTTP call made by the backend goes through one recording point per caller
  (LLM provider calls in the agent loop, news fetches, VoiceStudio). Hosts are taken from the
  configured base URL. Redirects are disabled on these clients, so the logged host is the host
  that was contacted.
- `GET /api/egress?since=<seq>` (session required when login is on): `{entries, non_loopback:
  [hosts], last_seq}`. `since` returns only newer entries; without it, the last 200.
- `is_loopback(host)`: `localhost`, `127.0.0.0/8`, `::1`. Nothing else counts as local, including
  LAN addresses.

### FL-4 Preflight (`jarvis-api preflight`, `crates/api/src/preflight.rs`, new)

`jarvis-api preflight [--config <path>] [--api <url>] [--skip-chat] [--json]`

Runs real calls, no mocks, and prints one line per check: `PASS`, `WARN` or `FAIL` with a reason.

| # | Check | Fail or warn |
|---|---|---|
| 1 | Config loads and validates | FAIL |
| 2 | API answers `/api/health` at the configured address | FAIL |
| 3 | Ollama answers and the configured model is pulled; if not, print `ollama pull <model>` | FAIL when Ollama is the default provider, else WARN |
| 4 | One chat request with the real system prompt and tool list returns non-empty text; prints seconds and prompt/answer token counts | FAIL |
| 5 | TTS returns a valid WAV for a known sentence; prints seconds | WARN (voice is optional) |
| 6 | The loop: that WAV through STT gives back the same words (word overlap of at least 80%) | WARN |
| 7 | The API does not serve `config/jarvis.toml`, `.env`, `jarvis.db` or `bridge.key` (expects 404) | FAIL |
| 8 | The built web files contain no `http(s)://` URL outside an allowlist (XML namespaces, loopback) | FAIL |
| 9 | No secret value from `.env` appears in the built web files | FAIL |
| 10 | Egress during this preflight: in local mode, no non-loopback host | FAIL (local mode), listed (cloud default) |

Ends with two lines, then exits 1 on any FAIL:

```
LATENCY  hear 0.8 s  think 2.4 s  speak 0.6 s  total 3.8 s
SUMMARY  9 pass, 0 fail, 1 warn
```

Checks 4 to 6 run in process (no login needed). Checks 2 and 7 go over HTTP to the running API.
Checks 8 and 9 read `web/build`. Check 10 reads the in-process egress log. "Local mode" means the
default provider is Ollama and the news provider is off.

### FL-5 Chunked playback, mute flag, TTS cache (`web/src/lib/voice/*`, `crates/api/src/voice.rs`)

- `chunkForSpeech(text, target = 170, maxChunks = 8): string[]`: sentence-packed chunks, never
  split inside a sentence unless one sentence is longer than 600 characters (then split at a
  word). Pure, unit tested.
- `speech.speak()` fetches chunk 1, starts playing, fetches chunk n+1 while chunk n plays, and
  resolves only after the last chunk ends. The confirmation read-back still starts its window
  when reading finishes (ADR-0007). `stop()` aborts the fetch and the playback of every chunk.
- Mute: if the page URL has `mute=1`, the single play function (`playBlob`) returns at once
  without sound. Nothing else plays audio.
- Server: an in-memory LRU of 64 WAVs keyed by `sha256(persona | voice id | text)`. Never on disk.
  Response header `X-Jarvis-Voice: piper:<voice>` or `voicestudio:<profile>`, and
  `X-Jarvis-Cache: hit|miss`.

### FL-6 VoiceStudio engine, optional (`crates/voice/src/voicestudio.rs`, new)

```toml
[voice]
tts_engine = "piper"            # or "voicestudio"; Piper stays the fallback

[voice.voicestudio]
url = "http://127.0.0.1:3900"   # must be loopback; refused otherwise
engine = "mlx-audio"
model_id = "mlx-community/csm-1b-8bit"
language = "en"
keep_warm_minutes = 10          # 0 = off

[voice.voicestudio.profiles]    # persona id -> VoiceStudio profile name
jarvis = "Jarvis"
friday = "Friday"
```

- At boot: `GET /health` (1 s timeout), `POST /engines/select {"family":"tts","backend_id":engine,
  "model_id":model_id}` so the engine is pinned, `GET /profiles` to find each profile id by name.
- Speak: `POST /v1/audio/speech {"model","input","voice":profile_id,"response_format":"wav",
  "language"}`, 600 characters at most.
- Any failure (down, timeout, unknown profile) falls back to the persona's Piper voice for that
  request, logs a warning, and `/api/status` reports which voice is live.
- Keep warm: after `keep_warm_minutes` without speech, render "Standing by." and drop the audio.
- The VoiceStudio API contract above comes from the pack and is **not verified** yet (VoiceStudio
  is not installed on this PC). It is checked against a running VoiceStudio before FL-6 merges.

### FL-7 Docs

- `docs/local-mode.md`: install and proof commands (Ollama, Whisper models, Piper, VoiceStudio),
  choosing a model for your memory, the preflight and how to read the latency line, the egress
  drill with Wi-Fi off, tuning notes, and the honest list of what needs the internet: cloud LLM
  providers when chosen, news providers when a key is set, the remote tunnel or VPS access,
  `scripts/fetch-models.ps1` and `ollama pull` (one-time downloads).
- ADR-0020, `docs/configuration.md` (new keys), `docs/api.md` (`/api/egress`), README features,
  CHANGELOG.

## Commands

```
Test:      cargo test --workspace
Lint:      cargo clippy --workspace --all-targets -- -D warnings
Format:    cargo fmt --all
Web:       pnpm -C web check; pnpm -C web test:unit; pnpm -C web build
Full gate: pwsh scripts/check.ps1
Preflight: cargo run -p jarvis-api -- preflight          (needs the API running)
```

## Testing strategy

| Slice | Tests |
|---|---|
| FL-1 | Unit: two prompts built one second apart share everything except the final clock line |
| FL-2 | wiremock: `keep_alive` is in the body; empty + `length` triggers one retry with a bigger `num_predict`; no retry otherwise; warm-up failure does not block boot |
| FL-3 | Unit: ring buffer bounds, `is_loopback` table; integration: a chat turn against a wiremock provider records its host with the turn id; `/api/egress` needs a session when login is on |
| FL-4 | Each check as a function with a fake context (pass, warn, fail cases); the URL scanner and secret scanner on fixture files; exit code; one `#[ignore]` live run |
| FL-5 | Vitest: `chunkForSpeech` (boundaries, long sentence, max chunks), prefetch order with fake fetch/play, mute flag; Rust: cache hit/miss headers, cache bound |
| FL-6 | wiremock VoiceStudio: boot pins the engine, resolves profile by name, speaks; fallback to Piper on 500, timeout and unknown profile; non-loopback URL refused |

## Boundaries

- Always: keep every SPEC.md core rule; keep the confirmation timing of ADR-0007; run
  `scripts/check.ps1` before calling a slice done; never log request bodies or keys.
- Ask first: any new dependency beyond `reqwest` in `voice`; anything that writes spoken text or
  audio to disk; changing the bridge protocol.
- Never: send audio or text to a non-loopback voice service; put a key or the egress log in the
  browser bundle; make boot wait on Ollama or VoiceStudio.

## Success criteria

1. `jarvis-api preflight` against the running stack prints a verdict per check, the latency line
   and the summary, and exits non-zero on a failure. Missing voice gives WARN, not FAIL.
2. With Ollama as the default provider and news off, check 10 passes; switching the default to a
   cloud provider makes it list that provider's host.
3. Two consecutive turns produce system prompts that differ only after the fixed rules, and the
   second turn's prompt evaluation time in the preflight is lower than the first (measured, noted
   in the ADR).
4. A 500-character reply is spoken in full, in order, with no request starting after its
   predecessor finished playing; a `?mute=1` tab makes no sound.
5. With `tts_engine = "voicestudio"` and VoiceStudio stopped, replies are still spoken by Piper and
   `/api/status` says so.
6. `scripts/check.ps1` stays green.

## Later candidates (not in this spec)

- A small Whisper model for the wake word and the large one for commands.
- Spoken barge-in: "stop" or "wait" ends playback at once.
- Spoken brain switch ("go local", "which brain are you on") with a short-lived badge.

## Open questions

1. VoiceStudio on Windows: which engine and model would you use there, and is its licence fine
   for your use? Until then FL-6 is built against the pack's contract and stays off by default.
2. Should check 4 of the preflight send a real chat turn through the running API (needs a login
   session when auth is on) or stay in process as specified?
3. Order: I propose FL-1, FL-2, FL-3, FL-4, FL-5, FL-6, FL-7. FL-6 can wait if VoiceStudio is not
   something you plan to install.
