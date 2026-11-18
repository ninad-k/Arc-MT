# Plan: Trading and Assistant modes

Spec: [spec-assistant-mode.md](spec-assistant-mode.md), approved 2026-11-11 with the defaults for its
open questions:
- $20 monthly cap;
- refusal fallback on;
- Jarvis only in Assistant mode.

## Decisions made while planning

- **Assistant mode gets its own small agent loop** (`crates/api/src/assistant.rs`), not a flag on the
  trading agent. Reasons:
  - The trading loop parses trade intents and rejects any figure no tool reported (the grounding
    check). Both are right for trading and wrong for a general assistant ("the Moon is 384,000 km
    away" would be blocked).
  - A separate loop holds a separate tool registry with no bridge in it, so MT5 tools cannot be offered
    by mistake.
- **Effort per request**: `ChatRequest.effort` (new, optional). The Anthropic provider sends it as
  `output_config.effort`; other providers ignore it. Chat turns use `low`, 3D builds `high`, from one
  provider.
- **Prompt caching** for every Anthropic request: `cache_control` on the system prompt and the last
  tool. Trading mode with Claude benefits too.
- **Usage breakdown**: `Usage` gains `cache_read_tokens` and `cache_write_tokens`, so cost is exact.
- **Refusal fallback**: a `refusal_fallback` flag on the provider config sends `fallbacks: "default"`
  with the `server-side-fallback-2026-07-01` beta header. `stop_reason: "refusal"` becomes a
  `StopReason::Refusal` with a plain reply.
- **3D sandbox runtime**: three.js and OrbitControls are bundled into one classic script,
  `static/sandbox/runtime.js`, by a small Vite plugin that runs the existing Vite build API. No new
  package; the file is generated and gitignored.
  - The page builds the iframe with `srcdoc` and `sandbox="allow-scripts"`, so it has an opaque origin.
  - A CSP meta tag allows only that one script URL plus `unsafe-eval`, with `connect-src 'none'`.
  - The model code arrives by `postMessage`.
- Models are saved under `data/models3d/` (not `data/models`, so they can't be confused with the Piper
  voice models in `models/`).

## Slices (each ends green)

1. **LLM**: request effort, caching markers, refusal stop reason, usage breakdown, fallback flag. Tests on
   the fake Anthropic server.
2. **Spend meter** (`crates/api/src/spend.rs`): cost from usage and configured prices, a monthly total
   persisted to `data/assistant-spend.json`, an 80% notice, a block at 100%.
3. **Captures** (`jarvis-notes`): `capture(text, now)` writes `captures/YYYY-MM-DD-slug.md`, never
   overwrites, then rescans.
4. **3D models** (`crates/api/src/models3d.rs`): store (save, list, get, cap 50), the builder prompt,
   parsing `TITLE:` plus a code block, the size cap, edits through `previous_id`.
5. **Assistant agent and routes**:
   - the butler prompt and the 4 tools;
   - sessions and audit, with the spend checked before each call;
   - `/api/chat` gets `mode`, plus the routes `/api/models*`, `/api/assistant/status` and
     `/api/notes/capture`;
   - config `[assistant]`.
6. **Web**:
   - the mode store and the top-bar switch, with a session per mode;
   - the brain badge, and the picker locked in Assistant mode;
   - the sandboxed `ModelStage` with its watchdog, and the "3D:" chips;
   - the status line (no key, budget) and the spend in Settings;
   - a live new star in the galaxy, and mock-mode support.
7. **Docs and checks**: ADR-0022, `docs/assistant-mode.md`, the config example, the API and
   configuration docs, the changelog and README; preflight checks; a code review; the full gate; the
   live check (about $1 to $2) once the key is set.
