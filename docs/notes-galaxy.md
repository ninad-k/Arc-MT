# Notes galaxy

Your markdown notes as a 3D galaxy, and an assistant that answers from them. Decision record:
[ADR-0021](adr/0021-notes-galaxy.md).

## Set it up

1. Put `.md` files in `notes/` at the repository root (or point `[notes] dir` at another folder). Sub-folders
   become colour groups: `plans/`, `rules/`, `lessons/`, `checklists/`, anything you like.
2. Give each note a `# Title` on its first heading line (otherwise the file name is used).
3. Link notes with `[[Another note]]`, `[[Another note|label]]`, or just by writing another note's full
   title in the text.
4. Open **Menu → Notes galaxy** (or `/galaxy`). New and changed files are picked up within a few seconds;
   **Rescan** does it at once.

To try it first, copy the sample notes: `Copy-Item -Recurse examples\notes notes`.

```toml
[notes]
enabled = true
dir = "notes"          # read only; never written
max_notes = 2000       # files beyond this are skipped
max_file_kb = 256      # larger files are skipped
```

## Use it

- **Browse**: drag to orbit, scroll to zoom, click a star to open the note. Links in the note open the linked
  note. **Overview** flies back out; Esc closes the note.
- **Search**: the box at the top lights matching notes; Enter opens the first match.
- **Ask**: the box at the bottom asks the assistant, for example "what is my rule for gold on news days?" or
  "how much should I risk per trade?". It answers from your notes, names the note, and the camera flies to it
  (to the whole group when four or more notes were used). If your notes do not cover the question, it says
  so and nothing moves.
- **On the HUD**: a reply that used your notes shows a "from: <title>" chip; it opens the galaxy at that note.

## Good to know

- Search is by keywords, so use the words your notes use. Titles count three times as much as the text.
- Notes are read only. Hidden folders (like `.obsidian`) and symbolic links are skipped, and nothing outside
  the folder is read.
- With the local model, notes never leave the PC. With a cloud model they are sent to that provider as part
  of the conversation.
- A note cannot make the assistant trade: text in notes is treated as your information, and every trade still
  needs your confirmation.
