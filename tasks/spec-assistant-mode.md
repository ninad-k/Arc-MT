# Spec: two modes, Trading and Assistant (Claude, notes, memory, 3D models)

Status: approved 2026-11-11 with the defaults for the open questions ($20 cap, refusal fallback on, Jarvis
only), and built. Decision record: [ADR-0022](../docs/adr/0022-assistant-mode.md). User guide:
[docs/assistant-mode.md](../docs/assistant-mode.md). Plan: [plan-assistant-mode.md](plan-assistant-mode.md).

Changes made while building:
- Assistant mode is its own agent loop (`assistant.rs`), not a flag on the trading agent. That loop has no
  intent parser and no grounding check, and its registry holds no bridge.
- The model id lives on the provider entry (`[[llm.providers]] claude`), not in `[assistant]`. A missing
  provider is reported by the mode itself, not a startup error.
- Models are kept in `data/models3d/`, so they can't be confused with the voice models in `models/`.
- The sandbox page carries its runtime inline. The server sends it its own policy (`SANDBOX_CSP`), with
  `unsafe-inline` and `unsafe-eval` only there, and forces `sandbox allow-scripts`.
  - An `srcdoc` frame was rejected: it would inherit the app's policy, which forbids `eval`.
- The spend total is in Settings → Model, not a new Settings section.
- Preflight check 11 reports Assistant mode without calling Claude.
- After code review:
  - The model's code runs in a Web Worker inside the sandbox page, drawing to an OffscreenCanvas, with a
    small orbit control. The builder gets `(THREE, scene, camera, helpers)`, with no renderer and no
    OrbitControls. Measured: an endless loop in the frame froze the whole tab in Chromium; in a worker
    it is terminated and the app stays usable.
  - The page refuses to run outside an opaque-origin frame, and the dev and preview servers send its
    policy.
  - Spending is reserved at the worst case before each request and settled on the reply. The turn
    runs in its own task, and there is one build per message.
  - Each mode has its own pending state, so a slow build never locks Trading.
  - Only fixed prompts are marked for caching.
  - The assistant's provider must be of kind `anthropic`.
  - Captures report success once the file is written.
  - A failure late in a turn keeps what tools already did.
  - Trading mode shows a plain line for a refusal.
  - `Cross-Origin-Resource-Policy` was dropped: it does not apply to a framed page.

Source: the user's request ("two modes: Jarvis/Friday on MT5, and another Jarvis built on the PDF,
connected to Anthropic, that can render any 3D model") and the "Run Jarvis Fully Local" prompt pack.
Choices the user made on 2026-11-11:

- **3D models:** Claude builds them from a description ("show me a 3D model of a jet engine").
- **Switching:** a **Trading | Assistant** switch in the top bar, in the same app.
- **Brain:** **Claude Opus 5.5 for every assistant turn** (paid).
- **Memory:** add the pack's **"remember that ..."** (writes new notes into `notes/captures/`).

Note: the PDF has no pictures of 3D models. Its only 3D feature is the notes galaxy (already built,
ADR-0021). Rendering any 3D model on request is new, and goes beyond the pack.

## Objective

1. **Trading mode** is today's app, unchanged: Jarvis or Friday, the MT5 tools, the confirmation gate,
   the kill switch, whatever brain the top-bar picker selects.
2. **Assistant mode** is a general Jarvis built on the pack:
   - a dry British butler persona (Prompt 04);
   - answers from your notes (Prompt 01), with the galaxy flying to the source note;
   - "remember that ..." writes a note, and a new star is born live in the galaxy (Prompt 05);
   - Claude Opus 5.5 as the brain, with a badge that always shows the brain and the voice (Prompt 06);
   - **"show me a 3D model of X"**: Claude writes the 3D scene, and the app shows it on the main
     stage, where you can rotate and zoom it and ask Claude to change it ("make the wings red").
3. **The modes cannot leak into each other.** Assistant mode has **no MT5 tools at all**: no prices,
   no account, no orders. Nothing said or built in Assistant mode can reach the trading gate.

Out of scope (later candidates):
- the pack's spoken brain switch ("switch to your local brain"), since you chose Claude always;
- local Whisper ears (whisper still doesn't build on this PC);
- loading your own .glb/.obj files;
- paid text-to-3D services (Meshy, Tripo);
- exporting models as files;
- web search.

## Assumptions (correct me now)

1. **One app, one switch.** The switch sits next to the persona toggle and is remembered per browser.
   - In Assistant mode the persona toggle is hidden: the assistant is always "Jarvis (butler)", a new
     third persona pack that only Assistant mode uses.
   - Each mode keeps its own chat session. Switching shows that mode's last conversation.
2. **The brain is fixed in Assistant mode**: the `claude` provider with model `claude-opus-5-5`. The
   top-bar model picker shows it and is disabled there.
   - If `ANTHROPIC_API_KEY` is not set, Assistant mode shows one clear line saying so and how to set it,
     and sends nothing anywhere. You set the key yourself in your environment. I will never ask for it
     in chat.
3. **Claude settings.** Effort `low` for chat turns (fast, cheap) and `high` for building a 3D model.
   - Prompt caching is on for the fixed system prompt and tools.
   - Refusals are handled: if Claude declines, you see one plain line, and nothing is retried silently.
   - The API's server-side refusal fallback (`fallbacks: "default"`) is on. It re-runs a declined
     request on another Claude model, billed at that model's rate. Say if you want it off.
4. **How a 3D model is made.** The chat model calls a tool, `build_3d_model(description, previous_id?)`.
   The server then makes **one separate Claude call** with a fixed, cacheable "3D builder" prompt.
   - That call returns `{title, code}`: a JavaScript function body that builds a three.js scene from
     primitives, procedural geometry, materials, lights and simple animation.
   - The server checks the size (at most 64 KB), saves the model, and returns its id.
   - "Make the wings red" passes `previous_id`, so Claude edits the last model instead of starting over.
5. **The model's code runs in a sealed sandbox, never in the app.**
   - It runs in an `<iframe sandbox="allow-scripts">` with an opaque origin. So it cannot read the
     app's pages, cookies, login token or storage, and cannot call the API.
   - A strict Content-Security-Policy blocks all network access (`connect-src 'none'`, no outside
     scripts or images), so the code cannot send anything anywhere.
   - three.js and OrbitControls are bundled into the sandbox page (no CDN), so models also render
     offline once built.
   - The sandbox talks to the app through `postMessage` only: `ready`, `error`, `stats`. The app
     ignores every other message, and every message from any other source.
   - A watchdog shows "this model stopped responding" and tears the iframe down if the frame rate
     drops to zero for 5 seconds (for example, an endless loop in the code).
6. **Where models show.** In Assistant mode the main stage shows the current model in place of the
   orb. The orb comes back with "Close model".
   - Models are saved as JSON under `data/models/` (gitignored), last 50 kept.
   - Each assistant reply that built a model shows a chip, "3D: <title>", that reopens it.
7. **"Remember that ..."** (and "note to self", "don't let me forget") writes
   `notes/captures/YYYY-MM-DD-<slug>.md`.
   - The note's title comes from its first words, with the date inside the file.
   - It creates files only: never edits or deletes, and never writes outside `notes/captures/`.
   - The notes index is refreshed at once, so the very next question can use the note.
   - On the galaxy page the new star appears next to its most related note, pulses, and the camera
     flies to it.
   - A failed write is said out loud, never silent.
   - Trading mode stays read-only for notes.
8. **Privacy, stated in the app and the docs.** In Assistant mode your words, matching note excerpts and
   model requests go to Anthropic. The egress log shows `api.anthropic.com` for those turns. Trading
   mode with the local brain still sends nothing out (the local-mode drill still passes there).

## Cost (your approval needed; nothing is charged until you set the key and use Assistant mode)

Prices are from Anthropic's price list (checked 2026-11-11): **Claude Opus 5.5 costs $4 per million input
tokens and $20 per million output tokens**, cache reads $0.20 per million, cache writes about $5 per
million. Thinking tokens count as output. To confirm, see https://www.anthropic.com/pricing.

| What | Estimate per use (on Opus 5.5) |
|---|---|
| A chat reply (about 3,000 tokens in, mostly cached, 300 to 600 out) | about $0.01 to $0.02 |
| Building or changing a 3D model (about 2,000 in, 4,000 to 10,000 out, with thinking) | about $0.10 to $0.25 |
| "Remember that ..." or a notes answer | same as a chat reply |

| Your month | Estimate |
|---|---|
| Light: 10 chat replies and 1 model a day | about $5 to $10 |
| Regular: 30 replies and 5 models a day | about $30 to $50 |
| Heavy: 100 replies and 20 models a day | about $100 to $180 |

**Safety cap (new):**
- `[assistant] monthly_budget_usd`, default **$20**. The server adds up each Claude reply's real token
  usage multiplied by the configured prices.
- At 80% of the cap a notice appears. At 100%, Assistant mode stops calling Claude until the next
  month or until you raise the cap. The running total is shown in Settings.
- The prices sit in the config, so you can update them.

**Building and testing it costs nothing:** every automated test uses a fake Anthropic server.
**Live check after the build:** about 10 chat turns and 5 models, roughly **$1 to $2**, only once you have
set the key and said yes.

## Contracts

### Config

```toml
[assistant]
enabled = true
provider = "claude"             # an [[llm.providers]] entry of kind "anthropic"
model = "claude-opus-5-5"
chat_effort = "low"
build_effort = "high"
monthly_budget_usd = 20.0
price_input_per_mtok = 4.0      # update if Anthropic's prices change
price_output_per_mtok = 20.0
price_cache_read_per_mtok = 0.20
price_cache_write_per_mtok = 5.0
models_dir = "data/models"
max_model_kb = 64
captures_dir = "captures"       # inside [notes] dir
```

### API

- `POST /api/chat` gains `mode: "trading" | "assistant"` (default `"trading"`, so old clients behave
  as today).
  - The response gains `models: [{id, title}]` (models built this turn) and `brain: {provider, model}`.
  - `notes` keeps today's meaning; a captured note is cited the same way.
- `GET /api/models` lists saved models `{id, title, created_at, description}`, newest first.
- `GET /api/models/{id}` returns `{id, title, description, code, created_at}`.
- `GET /api/assistant/status` returns `{available, reason?, brain, spent_usd, budget_usd, month}`.
- `POST /api/notes/capture` takes `{text}` and returns `{id, title, path}`. The same "one door" the
  assistant's tool uses.
- Login rules match the other routes. Every Claude call is in the audit log, with its token usage and
  cost.

### Assistant tools (the whole list; no MT5 tool is offered)

- `search_notes(query)` (exists).
- `remember_note(text)`: creates a capture note. Its result is the receipt (`{id, title, path}`). The
  persona rule says "never say a note was saved unless remember_note returned it".
- `build_3d_model(description, previous_id?)`: runs the 3D builder call and returns `{id, title}`.
  On failure it returns a plain error that the model passes on.
- `list_3d_models()`: titles and ids of recent models, so "show me the jet engine again" works.

### Sandbox page

- `web/static/sandbox/model.html` plus a bundled script, served with headers:
  - `Content-Security-Policy: default-src 'none'; script-src 'self' 'unsafe-eval'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'`;
  - `Cross-Origin-Resource-Policy: same-origin`.
- `unsafe-eval` is limited to this page: the model's code body becomes a `Function`. It runs only
  inside the opaque-origin iframe.
- The code receives `(THREE, scene, camera, renderer, helpers)` and may return an
  `update(dt)` function for animation.
- `helpers` covers common shapes: tube along a curve, lathe, extrude, text sprite.

### Persona: "Jarvis (butler)", under 300 words, fixed first in the prompt

The Prompt 04 rules, plus:
- for 3D requests, call `build_3d_model` and describe the result in one line;
- for "remember that", call `remember_note`;
- never claim a model or note exists without the tool's receipt;
- say plainly that it has no access to trading in this mode.

## Commands

```
cargo test -p jarvis-llm -p jarvis-notes -p jarvis-api
pnpm -C web check; pnpm -C web test; pnpm -C web build; pnpm -C web test:e2e
pwsh scripts/check.ps1
```

## Testing strategy

| Part | Tests |
|---|---|
| Anthropic provider | prompt-cache markers on system and tools; effort per call; `refusal` maps to a plain message; usage incl. cache tokens is reported (fake server) |
| Mode isolation | assistant turns offer exactly the 4 assistant tools; a scripted model that calls `propose_order` in Assistant mode is refused as an unknown tool; trading turns are unchanged |
| Budget | cost adds up from usage; 80% notice; at 100% no request is sent and the reply says why; month rollover |
| 3D builder | fake Claude returns code: saved, size cap enforced, `previous_id` sends the old code; invalid output gives a clear error |
| Captures | file written only under `notes/captures/`, slug safe (no `..`, no reserved names), never overwrites (adds `-2`), index refreshed, next search finds it, write failure reported |
| Sandbox | Playwright: a model renders (canvas has pixels); code that calls `fetch` or reads `parent.document`/`localStorage` fails inside the sandbox and the app is untouched; an endless loop triggers the watchdog; messages from other sources ignored |
| Web | the switch changes mode and session; the badge shows the brain; the model chip reopens a model; no-key message; galaxy shows a captured star live |
| Live (after approval, about $1 to $2) | 5 models (jet engine, solar system, caffeine molecule, a house, a DNA helix), one edit, 3 notes questions, one capture then a question that finds it |

## Boundaries

- **Always:**
  - keep every SPEC.md core rule;
  - Assistant mode never gets MT5 tools;
  - the model's code only ever runs in the sandbox;
  - the budget cap is checked before every Claude request.
- **Ask first:**
  - any spend beyond the $1 to $2 live check;
  - raising the default cap;
  - any new npm or Rust dependency (the plan needs none: three.js is already in the app, and the
    Anthropic calls use the existing HTTP client, since Rust has no official Anthropic SDK).
- **Never:**
  - ask for or print the API key;
  - run model code in the app's own page;
  - let captures edit or delete existing notes;
  - send anything to Anthropic in Trading mode unless you pick Claude in the picker, as today.

## Success criteria

1. The top-bar switch changes between Trading (today's app, unchanged) and Assistant (butler Jarvis on
   Claude Opus 5.5, with the badge showing it).
2. "Show me a 3D model of a jet engine" puts a recognisable, rotatable model on the stage in under
   about 60 seconds. "Make it red" changes that model.
3. A model whose code tries to reach the network or the app fails inside the sandbox, and nothing
   else is affected.
4. "Remember that my broker closes at 23:55" creates a note, a star appears in the galaxy, and the
   next question finds it.
5. In Assistant mode, asking for a trade gets a polite "not in this mode" and no tool call.
6. The spend shown in Settings matches the audit log, and the cap stops requests.
7. All existing tests still pass. The local-mode drill still passes in Trading mode with the local
   brain.

## Open questions

1. Is the **$20 monthly cap** right? (You can change it in the config at any time.)
2. Keep the server-side **refusal fallback** on (assumption 3)?
3. Should Assistant mode also have **Friday** as a butler-style option, or Jarvis only?
