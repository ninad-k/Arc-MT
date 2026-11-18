# Fully local mode

Arc MT can run with nothing leaving your PC: the language model, speech-to-text and text-to-speech all
run on the machine, and the web app loads nothing from the internet. This page shows how to set that up,
how to prove it, and how to read your machine's speed.

Decision record: [ADR-0020](adr/0020-fully-local-mode.md). Spec: [tasks/spec-fully-local.md](../tasks/spec-fully-local.md).

## What runs where

| Part | Local engine | Cloud option |
|---|---|---|
| Thinking | Ollama with a local model (default `qwen3.5:9b`) | Anthropic or OpenAI, when you pick it |
| Hearing | Whisper (whisper.cpp, built with the `whisper` feature) | none |
| Speaking | Piper, one voice per persona; optionally VoiceStudio with a cloned voice | none |
| Web app | Built bundle served by the API; no CDN, no web fonts | none |
| Market news | off | Finnhub or Alpha Vantage, when you add a key |

## Set it up

1. **Ollama.** Install it from ollama.com, open it once, and pull a model that fits your memory:

   | Memory | Model | What to expect |
   |---|---|---|
   | 8 GB | `gemma2:2b` | Simple questions; keep heavy work for a cloud model |
   | 16 GB | `qwen3.5:9b` (default) | A capable assistant, about 6 GB on disk |
   | 24 to 36 GB | a 12B model | Smarter, slower first load |
   | 64 GB and up | a 30B mixture-of-experts or 70B-class model | Close to cloud quality |

   ```powershell
   ollama pull qwen3.5:9b
   ```

   Avoid models whose tag ends in `cloud` (for example `minimax-m3:cloud`). Ollama runs them on its own
   servers even though Arc MT talks to `127.0.0.1`. The preflight flags them.

2. **Whisper and Piper.** `pwsh scripts/fetch-models.ps1` downloads and verifies the pinned models once.
   Speech-to-text needs a build with the `whisper` feature (`scripts\setup.ps1 -Whisper`).

3. **Keep `[llm] default = "local"`** and leave `[market.news] provider = "none"`.

4. **Optional cloned voice (VoiceStudio).** See [VoiceStudio](#optional-voicestudio-cloned-voice) below.

## Prove it: the preflight

Start the stack (`scripts\run.ps1`), then in another terminal:

```powershell
target\release\jarvis-api.exe preflight
```

It runs every live chain once, with real calls, and prints one line per check:

| # | Check | Fails when |
|---|---|---|
| 1 | config | the config does not load |
| 2 | api | the server does not answer `/api/health` |
| 3 | ollama | Ollama is down or the model is not pulled (it prints the `ollama pull` command); a cloud model is a warning |
| 4 | model | two chat requests with the real prompt and tools give no answer |
| 5 | voice out | text-to-speech does not return a WAV (a warning: voice is optional) |
| 6 | voice loop | that WAV, transcribed, does not give back at least 80% of the words (a warning without Whisper) |
| 7 | private files | the server hands out `config/jarvis.toml`, `.env`, the database or the bridge key |
| 8 | web: no CDN | the built web files load a script, style or image from another machine |
| 9 | web: no secrets | a value from `.env` appears in the built web files |
| 10 | egress | in local mode, any host other than this machine was contacted during the run |

It ends with two lines and exits with code 1 when a check fails:

```
LATENCY  hear 0.8 s  think 0.7 s  speak 0.7 s  total 2.2 s
SUMMARY  10 pass, 0 fail, 0 warn
```

- **hear**: Whisper transcribing a short sentence. **think**: the second of two model requests with the
  full system prompt and tools (the first one may include loading the model; both are printed in check 4).
  **speak**: rendering one sentence.
- `--skip-chat` skips the model check, `--json` prints one JSON document, `--api <url>` points at a server
  that is not at the configured address.

## The Wi-Fi-off drill

The backend records every outbound request it makes (host, purpose, time) in memory. See them with
`GET /api/egress` ([api.md](api.md#outbound-connections)), and check 10 of the preflight reads them.

1. Turn off Wi-Fi (or unplug the network cable).
2. Reload the web app. It renders: nothing in it comes from the internet.
3. Ask a question by voice, for example "what's gold at?".
4. Run the preflight. Check 10 passes with an empty host list.

With a cloud provider as the default, check 10 lists that provider's host (for example
`api.anthropic.com`), which shows the log works.

Ollama itself checks for app updates in the background. That is Ollama, not Arc MT, and it does not see
your prompts; turn it off in Ollama's settings if you want the PC completely silent.

## What still needs the internet

Read from the code, not from memory. Nothing else in Arc MT opens a connection to another machine.

| Feature | When | Host |
|---|---|---|
| Cloud language models | only when the chosen provider is `anthropic` or `openai` | `api.anthropic.com`, `api.openai.com` or your `base_url` |
| Ollama cloud models | only when the model tag ends in `cloud` | Ollama's servers (through the local Ollama) |
| Market news | only with `[market.news] provider` set and a key | `finnhub.io`, `www.alphavantage.co` |
| Phone access through a tunnel | only while `scripts\remote-start.ps1` runs | Cloudflare (`trycloudflare.com`) |
| One-time downloads | `scripts\fetch-models.ps1`, `ollama pull`, `scripts\setup.ps1` (Rust and Node packages) | the model and package hosts |

## Speed: where the time goes

- **The prompt is the slow part, not the PC.** The system prompt with the tools is about 2,000 tokens.
  The clock is now the last line of it, so everything before it is the same on every turn and Ollama reuses
  its cache. On the maintainer's PC this cut prompt evaluation from 0.40 s to 0.24 s per turn; slower
  machines gain more.
- **Keep the model loaded.** Arc MT sends `keep_alive` (default `30m`) with every request and loads the
  default local model in the background at start-up (`[llm] warm_up`). A cold load takes about 40 to 60 s.
- **Thinking models can return nothing.** A model that spends its whole token budget reasoning returns an
  empty answer. Arc MT sends `think: false` and, if a reply still comes back empty and cut off, retries once
  with twice the budget (at most 4,096 tokens).
- **Speech starts after the first sentence, then flows.** Long replies are split into whole sentences of
  about 170 characters; the next one is fetched while the current one plays, so there is no gap and no
  sentence is cut in the middle. Repeated lines are served from a small in-memory cache.
- **Memory bandwidth is the spec that matters.** A local model writes as fast as the chip reads its own
  memory. A laptop is noticeably slower than a cloud model; a workstation-class machine can match it.

## Optional: VoiceStudio cloned voice

VoiceStudio is a separate local app that speaks with a voice cloned from a 10 to 20 second clip. It
listens on `http://127.0.0.1:3900`. Its fast engine (CSM on MLX) needs Apple Silicon; on Windows, check
which engine it offers and that engine's licence before you use it. Clone only your own voice or one you
have permission to use.

```toml
[voice]
tts_engine = "voicestudio"      # Piper stays the fallback

[voice.voicestudio]
url = "http://127.0.0.1:3900"   # must be this machine; anything else is refused
engine = "mlx-audio"
model_id = "mlx-community/csm-1b-8bit"
language = "en"
keep_warm_minutes = 10          # render "Standing by." after 10 idle minutes; 0 = off

[voice.voicestudio.profiles]    # persona id -> VoiceStudio profile name
jarvis = "Jarvis"
friday = "Friday"
```

- Arc MT connects in the background (start-up never waits): it checks `/health` (1 second), pins the
  engine with `/engines/select` so VoiceStudio never swaps in another model, and finds each profile by
  name. A persona without an entry uses a profile named after the persona.
- Until VoiceStudio answers, and whenever a line fails, that line is spoken by the persona's Piper voice
  and is not cached. Arc MT connects again (and pins the engine again) at most every 30 seconds, so a
  VoiceStudio you open after Arc MT is picked up without a restart. `GET /api/status` shows
  `voice.tts_engine`, `voice.voicestudio_connected` and `voice.last_voice`; every spoken reply carries
  `X-Jarvis-Voice`.
- The VoiceStudio API contract is covered by tests against a stand-in, not yet against a real install.
