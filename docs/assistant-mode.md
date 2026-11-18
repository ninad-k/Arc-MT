# Assistant mode

Arc MT has two modes. Switch between them with **Trading | Assistant** in the top bar.

| | Trading | Assistant |
|---|---|---|
| Who answers | Jarvis or Friday (your choice) | Jarvis, a dry British butler |
| Brain | the model picked in the top bar (local by default) | Claude Opus 5.5, always |
| Can it trade? | Yes, with your timed confirmation | **No.** It has no MT5 tools at all |
| Notes | answers from your notes (read only) | answers from your notes, and saves new ones |
| 3D | the orb, and the notes galaxy | builds a 3D model of anything you ask for |
| Cost | free with the local brain | paid per use (see below), with a monthly cap |

Each mode keeps its own conversation, so switching shows that mode's last chat.

Design and reasons: [ADR-0022](adr/0022-assistant-mode.md). Spec: `tasks/spec-assistant-mode.md`.

## Set it up

1. Get an API key from the Anthropic Console.
2. Put it in `.env` in the repository folder, next to the other secrets:
   ```
   ANTHROPIC_API_KEY=sk-ant-...
   ```
   Never put the key in the config file, and never paste it into the chat.
3. Restart Arc MT (`scripts\stop.ps1`, then `scripts\run.ps1`).
4. Run `jarvis-api preflight`. Check 11, `assistant`, should say `claude · claude-opus-5-5 ready`.

Without a key, Assistant mode shows one line saying so, and sends nothing anywhere.

## Use it

- **3D models:** "show me a 3D model of a jet engine", "show me the solar system", "build a DNA helix".
  - The model replaces the orb. Drag to rotate, scroll or pinch to zoom; **Reset view** re-frames it.
  - "Make the wings red" or "add a second rotor" changes the model on screen.
  - Each reply that built a model has a **3D:** chip that brings it back. The list at the top of the
    stage holds recent models (the newest 50 are kept in `data/models3d/`).
- **Notes:** "what is my rule for gold on news days?" answers from your notes and names the note, as in
  Trading mode.
- **Remember:** "remember that my broker closes gold at 23:55" (also "note to self", "don't let me
  forget", "log this").
  - It writes `notes/captures/2026-11-11-my-broker-closes-gold-at-23-55.md`.
  - The next question can use the note at once.
  - The reply's chip opens it in the galaxy. On the galaxy page itself, the new star appears and the
    camera flies to it.
  - It only ever creates files. It never edits or deletes a note.

## Upload a file or a picture

Use the paperclip next to the message box, or drop a file on the 3D stage.

| Upload | What happens | Cost |
|---|---|---|
| A 3D file: `.glb`, `.gltf`, `.obj`, `.stl`, `.ply`, up to 25 MB | Shown as it is. Never sent to Claude. | Free |
| A picture (PNG, JPEG, WebP), then **Make a 3D relief** | The picture raised in depth (brighter parts stand higher), made on this computer. | Free |
| A picture, then **Build a 3D model with Claude**, with an optional note ("my 1 BHK floor plan, walls 3 m") | Claude looks at the picture and builds what it shows: a floor plan becomes walls and rooms, a photo becomes the object. The picture goes to Anthropic and is not kept afterwards. | About $0.10–0.25 |

- **Saving:** everything is saved with the other models, so it appears in the list at the top of the stage and
  in "3D:" chips.
- **Changing an upload:** "make the body red" or "put it on a table" works on an uploaded file or a relief.
  Claude writes a change that runs on the loaded object. It knows the part names and size, not the exact shape.
- **Not supported:**
  - `.fbx`;
  - `.gltf` files that point to separate `.bin` or texture files (export as `.glb`);
  - compressed glTF (Draco or Meshopt);
  - animations inside a file;
  - models from the internet: Claude builds models from shapes, it cannot download them.
- **How it's opened:** uploaded files are checked by their first bytes on the server and opened only inside
  the 3D sandbox's worker. A broken or hostile file shows an error there, and the app is not affected.

## What it costs

Claude Opus 5.5 costs $4 per million input tokens and $20 per million output tokens (cache reads $0.20).
Check the current prices at https://www.anthropic.com/pricing.

| What | About |
|---|---|
| A chat reply | $0.01 to $0.02 |
| Building or changing a 3D model | $0.10 to $0.25 |
| A light month (10 replies and 1 model a day) | $5 to $10 |

- **Monthly cap:** `[assistant] monthly_budget_usd`, default **$20**.
  - Each request's worst case is set aside before it is sent, so a request that could pass the cap is
    not sent. Close to the cap, a 3D build (worst case about $0.33) may be refused while chat still works.
  - At 80% a notice appears above the chat.
  - At the cap, Assistant mode stops calling Claude until next month, or until you raise the cap and
    restart.
  - One 3D model per message.
- **Where to see the spend:** this month's total is in Settings → Model and in the preflight. Each
  request is in the audit log as `claude_usage`, with its tokens and cost.
- **How the cost is counted:** from the token counts Claude reports, multiplied by the prices in
  `[assistant]`. If Anthropic changes its prices, update those keys.

## Safety

- **No trading.** Assistant mode runs in its own loop, with four tools: search notes, remember a note,
  build a 3D model, list models. A request for a trade gets "not in this mode". There is no tool that
  could reach MT5.
- **3D code is sealed off.** A model is JavaScript that Claude writes. It runs only in a worker inside
  `/sandbox/model.html`, a sandboxed frame.
  - The frame's origin is opaque, and its policy blocks all network access.
  - It cannot see the app, your login, your notes or your keys, and it cannot navigate the frame.
  - It can only draw, and send back "ready", "error" or a heartbeat.
  - A model that never finishes is stopped after a few seconds, and the app stays usable meanwhile.
- **Privacy.** In Assistant mode, what you type, matching note excerpts and model requests go to
  Anthropic. Trading mode with the local brain still sends nothing out. The egress log
  (`GET /api/egress`) shows `api.anthropic.com` after Assistant turns.

## Settings

All in `[assistant]` (see [configuration.md](configuration.md#assistant)). The Claude provider entry in
`[[llm.providers]]` sets the model (`claude-opus-5-5`), `timeout_secs = 240`, and
`refusal_fallback = true`. That last one re-runs a request Claude declines on another Claude model,
billed at that model's rate; set it to `false` to turn it off.

## Troubleshooting

| You see | Do |
|---|---|
| "Claude is not set up" | Set `ANTHROPIC_API_KEY` in `.env` and restart. |
| "This month's Claude budget is used up" | Raise `monthly_budget_usd`, or wait for next month. |
| "This model stopped responding" or "it stopped responding (it may never finish)" | Ask again, or for a simpler model. Some embedded browsers block sandboxed frames; use Chrome, Edge or the desktop app. |
| "The model's code failed: ..." | Ask Claude to fix it: "that model failed, please build it again". |
| Very slow builds, or a build that times out | Ask for a simpler model, or lower `build_effort` to `low`. |
