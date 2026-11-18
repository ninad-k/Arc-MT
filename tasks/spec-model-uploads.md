# Spec: upload a file or an image to see it in 3D

Status: approved and built 2026-11-11. User guide: [docs/assistant-mode.md](../docs/assistant-mode.md#upload-a-file-or-a-picture).

**Live check (2026-11-11, Claude Opus 5.5):** $0.19 in all.
- A floor-plan picture became a 3D flat with labelled rooms, doors and furniture (40 s, $0.10).
- A chair sketch became a slatted wooden chair (19 s, $0.04).
- "Make the body of my uploaded test car red" found the upload and recoloured it (17 s, $0.05).

**Changed after code review:**
- Each assistant message carries the model on screen (`stage_model`); the server looks up its title and kind,
  and tells Claude, so "make it red" changes that model.
- A change to an upload shares the stored file (`file_id`) instead of copying it. Pruning counts a shared file
  once, keeps it while any kept model uses it, and sweeps leftover `.bin` and `.tmp` files.
- Uploads use the long server time limit, and `/file` is sent as an attachment.
- Pictures for Claude are capped at 3.75 MB (the base64 limit is 5 MB). The app re-encodes as JPEG when needed.
- A window-wide drop cover takes drops, even over a model. A timed-out picture build says it may still finish.
- Parsing a large upload may take up to 30 s before the watchdog steps in. Reliefs report their parts. PLY
  points have a fixed pixel size.
- Part names are flagged to Claude as data, not instructions.
- A browser test runs code inside the worker on a textured GLB. The embedded texture loads (`blob:`);
  `http:` to the API and to the app's own site is blocked; the app's blob URLs are unreachable.
- Not done: a limit on how many uploads can run at once. One local user makes it unlikely to matter. Builds on [spec-assistant-mode.md](spec-assistant-mode.md) and
[ADR-0022](../docs/adr/0022-assistant-mode.md).

User's choices (2026-11-11): all three kinds of upload, and **save them on the server** so they appear with the
other models.

## Objective

In Assistant mode, an **Upload** button next to the message box takes a file and shows it on the 3D stage:

1. **3D model files**, free: `.glb`, `.gltf` (single file, with its data embedded), `.obj`, `.stl` and `.ply`.
   The file is shown as it is, in the same sandboxed viewer. Nothing is sent to Claude.
2. **Image → Claude builds it**, paid: a photo, sketch or floor plan (`.png`, `.jpg`, `.webp`), with an
   optional note ("this is my 1 BHK floor plan; walls 3 m high"). Claude looks at the image and builds a 3D
   model of it, like any other build.
3. **Image → 3D relief**, free: the picture turned into a raised relief panel in the browser. Brighter parts
   stand higher, with the picture's colours on top. It is the picture in depth, not a model of the object.

All three are saved with the other models: they appear in the recent-models list and in "3D:" chips, and
reopen after a reload. "Make it red" or "put it on a table" works on any of them. For an uploaded file,
Claude writes code that changes the loaded object (colours, scale, added parts). It knows the object's part
names and size, not its exact shape.

Out of scope:
- `.fbx`, and `.gltf` files that point to separate `.bin` or texture files (use `.glb`);
- compressed glTF (Draco or Meshopt), which needs a decoder downloaded from the web;
- animations inside uploaded files (shown in their rest pose);
- searching the internet for models (Claude cannot download them; it builds them).

## Assumptions (correct me now)

1. Uploads go through the existing login rules.
   - Size caps: **25 MB** for a 3D file, **5 MB** for an image.
   - The browser shrinks an image to at most 1568 px on its long side before sending it, because larger
     images cost more and Claude scales them down anyway.
2. The server checks each file's first bytes (glTF binary header, PNG/JPEG/WebP signatures, readable
   text for OBJ/STL-ASCII/PLY/glTF JSON) and refuses anything else. A `.gltf` must not reference anything
   but embedded `data:` URIs.
3. Files are stored next to the models (`data/models3d/<id>.bin`), never run on the server, and served only
   to the app. The store keeps the newest 50 models and at most **500 MB** in all, the oldest going first.
4. **Files are parsed only inside the sandbox worker** (the same sealed worker as Claude's code), with the
   three.js loaders already in the app (GLTF, OBJ, STL, PLY). No new package.
   - The sandbox policy gains `connect-src blob: data:`: the glTF loader reads its embedded textures
     through local blob and data URLs.
   - That allows no network access: `http`, `https` and `ws` all stay blocked.
5. An image for Claude goes in **one builder request** (not through the chat model): the image, the optional
   note, and the builder prompt with a line about building from a picture. The reply lands in the transcript as
   "3D: <title> (from your image)". The butler does not chat about it, which saves a request.
6. The relief is built in the sandbox worker from the image's pixels: a grid of up to 256 × 256 points, with
   the image as its texture.

## Cost (approval needed only for image → Claude; the rest is free)

Claude Opus 5.5: $4 per million input tokens, $20 per million output tokens.

| What | Cost |
|---|---|
| Open a 3D file | free (nothing leaves the PC) |
| Image → 3D relief | free (nothing leaves the PC) |
| Image → Claude builds it | image about 1,600 input tokens ($0.006), plus the build: about 4,000–12,000 output tokens at medium effort, **about $0.08–0.25 a model** |
| "Make it red" on an uploaded file | about $0.02–0.10 (Claude writes a short change, no image sent) |

Every paid request goes through the monthly cap ($20; $2.60 counted so far this month).

**Live check after the build:** two image builds (a floor plan and a photo), plus one change to an uploaded
file. About **$0.30–0.60**, only on your yes.

## Contracts

### API (login rules as for the other routes)

- `POST /api/models/upload?name=<file name>`, raw body (`application/octet-stream`; limit 25 MB) →
  `{id, title, kind: "file", format}`. `400` for an unknown or invalid file.
- `POST /api/models/from-image?note=<text>`, raw image body (limit 5 MB) →
  `{id, title, kind: "code"}` after Claude builds it.
  - The reply says so in plain words when the cap is reached, Claude declines, or the build fails.
  - The image itself is not kept after the build.
- `POST /api/models/relief?name=<file name>`, raw image body (5 MB) → `{id, title, kind: "relief"}`.
  Stored with its image, free.
- `GET /api/models/{id}` gains `kind: "code" | "file" | "relief"`, `format` (for files), `info` (for files: part
  names and size, filled in by the browser after the first load) and `overlay` (a change written by Claude).
- `GET /api/models/{id}/file` → the stored bytes (`application/octet-stream`, `nosniff`).
- `build_3d_model(description, previous_id)` on an uploaded file or relief: Claude gets the part names and size,
  and writes code that changes `helpers.model` (the loaded object). That code is saved as the new model's
  `overlay`.

### Sandbox

- Two new messages from the app: `{type: "load", format, bytes: ArrayBuffer}`, sent with the bytes
  transferred, and `{type: "relief", bytes}`. An optional overlay is then run like Claude's code, with
  `helpers.model`.
- The worker reports `ready` with `triangles` and, for files, `info: {parts, size}`. The app saves that once
  (`PUT /api/models/{id}/info`), so a later change by Claude knows the parts.

### Web

- In Assistant mode, an **Upload** button (paperclip) beside the microphone accepts 3D files and images.
- For an image, a small sheet asks "Build a 3D model with Claude (about $0.10–0.25)" or "Make a 3D relief
  (free)", with an optional note.
- A progress line shows while Claude builds from an image.
- Dragging a file onto the stage does the same.

## Testing

| Part | Tests |
|---|---|
| Validation | each format's header check; a renamed `.exe` refused; oversize files refused; `.gltf` with an external URI refused |
| Store | files and models kept within 50 items and 500 MB; ids never escape the folder; `/file` serves exact bytes |
| Image build | fake Claude: the request carries the image block and the note; spend reserved and settled; cap, refusal and bad output handled; the image is not stored |
| Overlay | "make it red" on a file model: the builder gets part names and size, and the overlay is saved on a new model linked to the old one |
| Sandbox (Playwright) | a small GLB, OBJ, STL and PLY each render; a relief renders; a corrupt file shows an error and the app stays usable; no network request leaves the frame |
| Web | upload button, image choice sheet, drag and drop, chips and recent list, phone width |

## Boundaries

- Always: uploaded files are parsed only in the sandbox worker; every paid request goes through the cap;
  sizes are checked on both sides.
- Ask first: any spend beyond the $0.30–0.60 live check; any new dependency (none planned).
- Never: run or parse an uploaded file on the server; send a 3D file or a relief image to Anthropic; fetch
  anything from the internet for a model.

## Success criteria

1. Dropping a `.glb` of a car onto the stage shows the car, free, and it is in the recent list after a reload.
2. Uploading a 1 BHK floor plan image with "walls 3 m" gives a recognisable 3D flat in about 1–2 minutes, at
   about $0.10–0.25.
3. A photo as a relief shows the picture in raised 3D, free.
4. "Make it red" on an uploaded file recolours it.
5. A corrupt or hostile file shows an error without affecting the app.
6. All existing tests still pass.
