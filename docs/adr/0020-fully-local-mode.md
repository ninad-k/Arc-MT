# ADR-0020: Fully local mode with a measured latency budget and an outbound-connection log

- Status: Accepted
- Date: 2026-11-11

## Context

ADR-0014 made local models the default: Ollama for the language model, Whisper for hearing and Piper for
speaking. That made "nothing leaves the PC" true by default, but it was never shown: no part of Arc MT
listed the connections it opened, and no command checked the live chains end to end. Speed was guessed,
too. The example config said "about 2 s per reply once loaded" and "about 100 s" for a cold start, with
no way for a user to measure their own machine.

A public guide on running a voice assistant fully locally ("Run Jarvis Fully Local", Zubair Trabzada,
October 2026) listed the lessons that matter for this kind of app: the prompt is the slow part, a model
that thinks silently can return nothing, a voice that starts fast and then pauses sounds broken, hearing
is the usual hidden cloud, and a preflight with real calls is worth more than unit tests for these chains.
Arc MT already avoided the hidden cloud (Whisper on the server, no browser speech service, no CDN), but
looking at the code against that list found real gaps:

- The system prompt started with the current time to the second. A local model can reuse its cache only
  for the unchanged start of a prompt, so every turn evaluated the whole prompt (about 2,000 tokens with
  the tools) again. The prompt also carried runs of spaces left over from source formatting.
- Ollama was not told to keep the model loaded, and nothing loaded it before the first question.
- A reply cut off by the token limit with no text came back as "I have no answer" with no retry.
- Speech was a single clip cut at 600 characters, so long replies were not read in full.
- Ollama "cloud" models (tags ending in `cloud`) are reached through the local Ollama at `127.0.0.1` but
  run on Ollama's servers, so a loopback address alone does not prove a prompt stays local.

## Decision

- **Stable prompt prefix.** The persona frame and the shared rules come first and do not change between
  turns. The clock is the last line. Per-line indentation and stray space runs are removed.
- **Ollama stays warm.** Every Ollama request carries `keep_alive` (default `30m`, per provider). At start-up
  the server loads the default model when it is Ollama (`[llm] warm_up`, default on), in the background,
  and never waits for it. A reply with no text, no tool calls and `done_reason = "length"` is retried once
  with twice the token budget, at most 4,096.
- **Outbound-connection log.** Every request the backend makes to another service (model providers, news
  providers) is recorded in memory with its host, purpose, duration and whether it is this machine, in a
  ring of 200 entries. Redirects are turned off on those HTTP clients so the recorded host is the one
  contacted. `GET /api/egress` returns the entries, with the same login rules as the audit API.
  VoiceStudio is not logged because its URL is refused unless it is loopback.
- **`jarvis-api preflight`.** One command runs ten checks with real calls (config, API, Ollama and the
  model, a two-request chat with the real prompt and tools, text-to-speech, a speech round trip, private
  files not served, no external loads and no `.env` value in the built web app, and the outbound log),
  prints a latency line (hear, think, speak, total) and a summary, and exits with 1 on a failure. Missing
  voice is a warning, not a failure. An Ollama cloud model counts as not local.
- **Chunked speech in the browser.** Replies are split into whole sentences packed to about 170
  characters, at most eight chunks; the next chunk is fetched while the current one plays, and `speak()`
  resolves after the last, so the confirmation window still starts when reading ends (ADR-0007). A
  `?mute=1` page never plays a sound. The server keeps the last 64 spoken lines in memory (never on disk,
  because spoken lines contain account figures) and labels replies with `X-Jarvis-Voice` and
  `X-Jarvis-Cache`.
- **Optional VoiceStudio voice.** `[voice] tts_engine = "voicestudio"` speaks through a local VoiceStudio
  app with the engine pinned on every (re)connection and profiles found by name; each persona falls back
  to its Piper voice, and a line spoken by the fallback is not cached under the VoiceStudio voice (the
  voice that spoke is returned with the audio, so overlapping requests cannot mix up the label). The
  connection is made in the background and retried at most every 30 seconds; start-up never waits for
  it. Off by default.

## Consequences

- Users can measure their own machine and prove that nothing left it. On the maintainer's Windows PC with
  `qwen3.5:9b`: the first chat request took 41.9 s (model load), the second 0.7 s; Piper took 0.7 s for a
  sentence. A direct A/B against Ollama with the same rules text measured 0.40 s of prompt evaluation per
  turn with the clock first and 0.24 s with the clock last.
- The status object gains `voice.tts_engine`, `voice.voicestudio_connected` and `voice.last_voice`; tests that compared the whole voice
  object now check its fields.
- `jarvis-api` and `jarvis-voice` depend on `reqwest` directly. It was already in the dependency tree
  through `jarvis-llm`; no new crate entered the lockfile.
- The VoiceStudio client follows that app's documented endpoints and is tested against a stand-in only. It
  must be checked against a real install before anyone relies on it. Its fast engine is Apple-only, and the
  licence of the engine chosen on Windows is the user's to check.
- The outbound log covers what the backend does. It cannot see what another local program does on its own
  (an Ollama cloud model, Ollama's update check); the preflight warns about the first and the docs mention
  the second.

## Alternatives considered

- **A Python preflight script**, as in the guide. It would need a second runtime and could not reuse the
  config loader, the prompt builder or the voice engines, so it would measure something close to, but not
  the same as, what the server does.
- **An operating-system firewall log for the drill.** It sees everything, but it needs administrator rights,
  differs between machines and cannot say why a request was made. The in-process log gives host and reason
  for the requests Arc MT makes; the Wi-Fi-off drill covers the rest.
- **Playing the first sentence as soon as it exists.** Faster to the first word, but the next chunk is not
  ready when the first ends, and the gaps sound broken. Prefetching whole sentences was chosen.
- **A disk cache for spoken lines.** It would survive restarts, but it would leave account figures in audio
  files on disk.
