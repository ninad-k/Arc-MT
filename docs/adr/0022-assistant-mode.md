# ADR-0022: Two modes, with an Assistant on Claude that builds 3D models in a sandbox

- Status: Accepted
- Date: 2026-11-11

## Context

The user asked for two modes:
- Jarvis/Friday trading on MT5, as today;
- a second Jarvis built on the "Run Jarvis Fully Local" prompt pack, connected to Anthropic and able to
  render 3D models.

The pack itself has no 3D models: its only 3D view is the notes galaxy (ADR-0021). The user chose:
- Claude builds a model from a description;
- a Trading | Assistant switch in the top bar;
- Claude Opus 5.5 for every assistant turn;
- the pack's "remember that ..." note capture.

Constraints:
- The SPEC core rules: the model only reads and proposes, and nothing trades without a timed
  confirmation.
- The prompt prefix stays fixed (ADR-0020).
- The user's global rule that nothing may add cost without approval: the spec's prices and a monthly
  estimate were approved.

## Decision

- **A separate agent loop for Assistant mode** (`crates/api/src/assistant.rs`), reached with
  `POST /api/chat {"mode": "assistant"}`. It holds no bridge, trading or market service, so no MT5 tool
  can be offered or run, and an unknown tool call is refused. It has no trade-intent parser and no
  grounding check. Those protect trading answers; they would block ordinary facts in a general
  assistant.
- **Four tools:** `search_notes`, `remember_note`, `build_3d_model` and `list_3d_models`.
- **A butler persona** under 300 words (Prompt 04 of the pack), fixed first in every request and changing
  only once a day (the date line). It is always Jarvis: the persona toggle belongs to Trading mode.
- **3D models are code, run only in a sandbox.**
  - `build_3d_model` makes one separate Claude request (medium effort by default) with a fixed builder prompt. It
    answers with a title and the body of a JavaScript function that builds a three.js scene from
    primitives and procedural geometry.
  - The server stores that text (`data/models3d/`, newest 50) and never runs it.
  - The browser runs it only in `/sandbox/model.html`, loaded in `<iframe sandbox="allow-scripts">`.
    The server sends that page its own policy: `connect-src 'none'`, no forms, no plugins,
    `frame-ancestors 'self'`, `X-Frame-Options: SAMEORIGIN` and a forced `sandbox allow-scripts`.
  - The page's origin is opaque. It cannot read the app, its cookies or its storage, and cannot reach
    the network. It talks to the app only through `postMessage`, with four message types, from and to
    its own window.
  - The page refuses to start unless its origin is opaque and it is framed, so another site cannot
    drive it on the app's origin. The Vite dev and preview servers send it the same policy as the Rust
    server.
  - **The model's code never runs on a page's thread.** The page starts a Web Worker from a blob (it
    keeps the page's policy), hands it an OffscreenCanvas, and forwards dragging, pinching and scrolling.
    If the worker stops sending its heartbeat (a model that never finishes), the page terminates it.
    So a bad model can neither freeze the app nor navigate the frame: a worker's `location` is
    read-only.
  - The app's own watchdog closes a frame that goes quiet. It pauses while the tab is hidden, and it
    closes the frame if it ever loads a second page.
  - three.js is bundled into the worker at build time (`web/scripts/build-sandbox.mjs`, no new package),
    so models render offline once built.
- **Captures:** `NotesService::capture` writes `captures/YYYY-MM-DD-<slug>.md` inside the notes folder.
  - It creates files only, with `create_new`, so it never overwrites.
  - It refuses a captures folder that is a link or a junction.
  - It rebuilds the index, so the next question finds the note.
  - Trading mode keeps notes read-only.
- **Claude settings:** prompt caching on the system prompt and the last tool (all Anthropic requests);
  effort per request (`low` for chat, `medium` for builds); `stop_reason: "refusal"` mapped to a plain
  reply; and an optional `refusal_fallback` that sends `fallbacks: "default"`.
- **A monthly cap** (`[assistant] monthly_budget_usd`, default $20), paid in advance.
  - Before each Claude request, its worst case is set aside: the estimated input plus the whole output
    budget. A request that could take the month past the cap is not sent.
  - When the reply arrives, the reservation is replaced by the real cost from its token usage, with
    cache reads and writes at their own rates.
  - A request that never comes back (a timeout, a dropped connection) stays counted at its worst case.
    A request the API refused (4xx) is given back.
  - The total is kept in `data/assistant-spend.json`, and each request is logged as `claude_usage`.
  - The turn runs in its own task, so a dropped HTTP request cannot cut the accounting short.
  - One 3D build per message keeps a turn within the request time limit.
  - Only the assistant's fixed prompts are marked for prompt caching. Trading mode's prompt ends with a
    clock, so marking it would only pay the cache-write price.

## Consequences

- Assistant mode costs money: about $0.01 to $0.02 a chat reply and $0.10 to $0.25 a 3D model on Opus 5.5
  ($4 / $20 per million tokens). The cap bounds the monthly total.
- Text typed in Assistant mode, matching note excerpts and model requests go to Anthropic.
  - The egress log shows `api.anthropic.com` for those turns.
  - The fully-local drill fails while that is in the log, which is the honest result.
  - Trading mode with the local brain still sends nothing out.
- Building a model takes from several seconds to about a minute, so the Claude provider's timeout is 240 s,
  the server allows 300 s for a chat request and the web client waits up to 320 s.
- Some embedded browsers refuse to load a sandboxed frame by URL (the Claude desktop app's preview pane
  reports `ERR_BLOCKED_BY_CLIENT`). The model then shows a "stopped responding" message. Chrome, Edge,
  the Tauri desktop app (WebView2) and Playwright's Chromium load it.
- A model whose code loops forever only stops its own worker, which the sandbox page terminates after a
  few seconds; the app stays responsive. This was measured: before the worker, such a model froze the
  whole tab in Chromium, kill switch included.
- Costs from a refusal fallback are counted at the configured (Opus) prices, which may differ slightly
  from the fallback model's.
- Keyword note search and the pack's other prompts (spoken brain switch, local ears) are unchanged or out
  of scope.

## Alternatives considered

- **A JSON scene description instead of code.** Safer to render, but far less expressive: a jet engine or
  a DNA helix needs curves, lathes and instancing. The sandbox makes code acceptable.
- **An `srcdoc` iframe.** It would need no extra page, but it inherits the app's policy, which forbids
  `eval`. It cannot run the model, and loosening the app's policy is worse.
- **A flag on the trading agent.** Less code, but the trading tools, intent parser and grounding check
  would all need mode switches, and one mistake would expose a trading tool. A separate loop with its own
  registry makes that impossible by construction.
- **Paid text-to-3D services (Meshy, Tripo).** Realistic meshes, but another paid account, another
  provider and slower. Left for later.

## Addendum: uploads (2026-11-11, tasks/spec-model-uploads.md)

- **3D files** (`.glb`, `.gltf`, `.obj`, `.stl`, `.ply`):
  - checked by their first bytes on the server and stored as they are;
  - parsed only in the sandbox worker, with the three.js loaders already in the app.
- **Reliefs** are built in the worker from the picture's pixels.
- **"Build from a picture"** is one builder request with an image block: it costs about the same as any
  other build, and the picture is not kept.
- **Changing an upload:** Claude writes an `overlay`, code that runs on the loaded object (`helpers.model`).
  It is given the part names and size the sandbox reported.
- **Sandbox policy:** it now allows `connect-src blob: data:`, so the glTF loader can read embedded
  textures. Those are local reads; every network scheme stays blocked. A browser test checks this from
  inside the worker: the texture loads, `http:` to the API and the site is blocked, and the app's own blob
  URLs are unreachable.
- **Which model "it" is:** every assistant message carries the model on screen, and the server tells
  Claude its id, title and kind.
- **Storage:** a change to an upload shares the stored file instead of copying it.

## Live check (2026-11-11, Claude Opus 5.5)

- Chat: small talk, a notes question with the right citation, "remember that" with a follow-up question that
  found the new note, and a trade request refused with no tool call. Each took 2–4 s and cost about $0.01.
- 3D models at `high` effort: a turbofan cutaway with labelled stages, a solar system with Saturn's rings and an
  asteroid belt, a caffeine molecule and a labelled DNA helix. They took 45–115 s and 5,000–12,000 output tokens
  each, mostly thinking, at $0.10–0.25 each. Three builds passed the 150 s timeout, and the cap counted each at
  its worst case (about $0.32). So builds now default to `medium` effort, the Claude timeout is 240 s and
  `[server] long_request_timeout_secs` is 300.
