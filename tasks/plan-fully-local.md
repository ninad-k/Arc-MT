# Implementation plan: fully local mode

Spec: [spec-fully-local.md](spec-fully-local.md) (approved 2026-11-11, "build all").

Status: all tasks built 2026-11-11. Measured on the maintainer's PC: `qwen3.5:9b` 41.9 s for
the first request (model load), 0.7 s warm; Piper 0.7 s per sentence; prompt evaluation
0.40 s -> 0.24 s per turn with the clock moved last. Not verified: VoiceStudio against a real
install, and the speech round trip (this PC's build has no `whisper` feature).

## Overview

Seven slices, each leaving `scripts/check.ps1` green. FL-1 and FL-2 make local replies faster
and steadier, FL-3 records outbound connections, FL-4 turns everything into one preflight
command, FL-5 and FL-6 improve the voice, FL-7 documents it.

## Architecture decisions

- **Egress is recorded by wrappers, not inside each client.** `EgressProvider` wraps any
  `LlmProvider`; the news service and the VoiceStudio client report through the same
  `EgressLog`. Entries carry a monotonic `seq`, so a caller (the preflight) can ask for "everything
  since seq N" instead of needing turn ids. (`GET /api/egress?since=N`; the spec's `turn`
  filter becomes `since`.)
- **Redirects off** on the LLM and news HTTP clients, so the logged host is the host contacted.
- **Preflight runs in process** for the model, voice and loop checks (no login needed) and over
  HTTP for the served-files checks. It reuses `Config`, `build_provider`, `Voice::from_config`,
  `compose_system_prompt` and the tool specs, so it measures the real prompt the server sends.
- **`reqwest` becomes a normal dependency of `jarvis-api`** (it is already a dev-dependency and in
  the lockfile through `jarvis-llm`) and of `jarvis-voice`. No new crate enters the lockfile.
- **TTS cache in memory only** (spoken lines contain account figures), 64 entries, LRU.
- **Chunked playback is a client change**; the server keeps its 600-character limit per request.

## Task list

### Phase 1: faster local brain
- [x] T1 (FL-1) Clock line moves to the end of the shared rules. Test: prompts built at two
  different times are identical up to the clock line. Files: `crates/api/src/agent.rs`.
- [x] T2 (FL-2) Ollama `keep_alive` (provider key, default `30m`) and the empty-reply retry.
  wiremock tests. Files: `crates/llm/src/{types,ollama}.rs`, `crates/llm/tests/ollama.rs`.
- [x] T3 (FL-2) `[llm] warm_up` at boot, in the background. Files: `crates/api/src/{config,state,serve}.rs`.

### Checkpoint A: cargo fmt, clippy, test green.

### Phase 2: proof
- [x] T4 (FL-3) `EgressLog` + `is_loopback` + `EgressProvider`; wire providers and news; redirects
  off. Files: `crates/api/src/egress.rs` (new), `state.rs`, `crates/llm/src/util.rs`,
  `crates/news/src/http.rs`.
- [x] T5 (FL-3) `GET /api/egress?since=` behind the session guard. Test in `crates/api/tests/http.rs`.
- [x] T6 (FL-4) Preflight checks as pure functions over a context (URL scanner, secret scanner,
  word overlap, verdict summary) with unit tests. Files: `crates/api/src/preflight.rs` (new).
- [x] T7 (FL-4) `jarvis-api preflight` subcommand wiring the real chains; latency and summary
  lines; exit code. Files: `cli.rs`, `main.rs`, `preflight.rs`.

### Checkpoint B: preflight runs against the stack; check 10 lists a cloud host when the default is cloud.

### Phase 3: voice
- [x] T8 (FL-5) `chunkForSpeech` + prefetching `speak()` + mute flag in `playBlob`. Vitest.
  Files: `web/src/lib/voice/{text,audio}.ts`, `web/src/lib/state/speech.svelte.ts`, tests.
- [x] T9 (FL-5) Server TTS cache and `X-Jarvis-Voice` / `X-Jarvis-Cache` headers. Files:
  `crates/api/src/voice.rs`, `crates/api/tests/voice.rs`.
- [x] T10 (FL-6) `VoiceStudioTts` (boot pin, profile lookup, speak, keep-warm) with Piper fallback;
  config keys; loopback-only URL. wiremock tests. Files: `crates/voice/src/{voicestudio,lib}.rs`,
  `crates/api/src/{voice,config}.rs`.

### Checkpoint C: `scripts/check.ps1` green (web check/unit/build/e2e included).

### Phase 4: docs
- [x] T11 (FL-7) `docs/local-mode.md`, ADR-0020, `docs/configuration.md`, `docs/api.md`, README,
  CHANGELOG, `config/jarvis.example.toml`.

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| VoiceStudio API differs from the pack | Med | Off by default; wiremock contract; verify against a real install before relying on it |
| Ollama not running on this PC, so no real timings yet | Med | Unit and wiremock tests now; the preflight gives real numbers when Ollama runs |
| Chunked playback breaks the read-back timing (ADR-0007) | High | `speak()` resolves only after the last chunk; e2e voice tests must stay green |
| Whisper feature not built (needs CMake/LLVM) | Low | Loop check reports WARN, not FAIL |
| Moving the clock changes model behaviour | Low | Same text, only its position changes; grounding tests stay green |
