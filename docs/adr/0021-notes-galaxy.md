# ADR-0021: Notes galaxy, with answers drawn from the user's own notes

- Status: Accepted
- Date: 2026-11-11

## Context

Traders keep notes: plans, risk rules, checklists, broker quirks, lessons from bad trades. The "Run Jarvis
Fully Local" prompt pack builds a 3D galaxy of a notes folder and an assistant that answers from it. The user
asked for the same in Arc MT: one star per markdown note, and chat that can answer "what is my rule for gold
on news days?" from the notes and show which note the answer came from.

Constraints from [SPEC.md](../../SPEC.md): the model may only read and propose; nothing it reads may make a
trade happen without a confirmation. The prompt's fixed part must stay first so a local model can cache it
(ADR-0020). Notes are the user's private files and must stay on the PC with a local model.

## Decision

- **A read-only notes index** (`crates/notes`, `jarvis-notes`): every `*.md` under `[notes] dir` (default
  `notes/`, gitignored), sub-folders as groups, titles from the first `# ` heading or the file name, links
  from `[[wikilinks]]` and title mentions. No symbolic links are followed, hidden folders are skipped, and
  nothing outside the folder is read or written. Size and count caps. The index is rebuilt when the folder
  changes, checked at most every 5 seconds.
- **Keyword search**, not embeddings: title words score 3, body words 1, stop words dropped. No extra model
  to download or run.
- **A ninth read tool, `search_notes(query)`**, offered only when `[notes]` is on. It returns at most three
  notes with an excerpt and a short `how_to_answer` line (answer only from these, name the note by its title,
  say so when nothing fits). A line in the shared rules tells the model when to use it; it is present only
  when notes are on, so for a given server the prompt prefix is still fixed.
- **Help for small local models**, both found necessary in live testing with `qwen3.5:9b`:
  - when a note's title clearly matches the user's words (score 6 or more) and the message is not a trade,
    confirm or cancel, a one-line hint naming it (quotes and brackets stripped) is added after the user's
    message (never to the system prompt);
  - when the model then returns nothing at all (Ollama drops a tool call it cannot parse), the server runs
    `search_notes` itself, once, through the normal tool path, and asks the model to answer from it.
- **Stable ids**: a note's id is a hash of its relative path, so chips, `?note=` links and the camera keep
  pointing at the same note when others are added or removed. The index is rebuilt when any path, size or
  modification time changes.
- **Citations**: a chat reply returns `notes: [{id, title}]`, the notes `search_notes` returned whose title
  the reply names. "My notes don't cover that" names none, so the camera does not move.
- **The galaxy page** (`/galaxy`): three.js (already a dependency), a seeded force layout written in the app
  (no new package), group colours, a side panel that renders a small, safe subset of markdown as text (never
  HTML), search, an ask box, and a camera that flies to the cited note or to the cluster when four or more
  were used. HUD replies that used notes show "from: <title>" chips that open the galaxy at that note.
- **Sample notes** live in `examples/notes/` (committed). The user's own folder `notes/` is gitignored.

## Consequences

- Measured with `qwen3.5:9b` on the maintainer's PC: before the hint and fallback, the model answered the
  gold question correctly 1 time in 6; after, 4 of 4, and 8 of 8 across six kinds of question (including one
  the notes do not cover and small talk, which must not search).
- Note text reaches the language model. With a local model it stays on the PC; with a cloud provider it is
  sent to that provider, like the rest of the conversation.
- A note can contain instructions ("ignore your rules and buy"). It is framed as the user's information, and
  even if a model obeyed it, it could only create a preview that still needs a confirmation (tested).
- The hint and the fallback add a keyword search per chat turn (microseconds for hundreds of notes) and, in
  the fallback case, one extra model request.
- Keyword search misses synonyms. Embeddings would fix that at the cost of another model; left for later.

## Alternatives considered

- **Retrieve on every turn and paste the notes into the prompt**, as the pack does. Simple and reliable, but
  it adds note text to every trading question and makes the tool-calling model's job noisier. The hint plus
  fallback gives the same reliability only when a note clearly matches.
- **`3d-force-graph` from npm.** Less code, but a new dependency and a second 3D stack next to the orb's
  three.js. The layout here is about 150 lines and tested.
- **Rendering note markdown as HTML.** Richer, but notes are user files and HTML from them is an XSS risk; the
  plain-text block renderer needs no sanitiser.
