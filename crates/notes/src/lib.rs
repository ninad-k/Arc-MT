//! The user's markdown notes (ADR-0021): an index of a folder of `.md` files, with titles,
//! groups (sub-folders), links ([[wikilinks]] and title mentions) and keyword search.
//!
//! The index only reads. The one write is [`NotesService::capture`] ("remember that ...",
//! Assistant mode): it creates a new file in one sub-folder and never edits, renames or deletes
//! a note. Symbolic links are not followed and nothing outside the folder is read or written.
//! Hidden folders (`.obsidian`, `.git`) are skipped.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Serialize;

/// About how many characters of a note go into its excerpt.
pub const EXCERPT_CHARS: usize = 700;
/// How often [`NotesService::index`] looks at the folder for changes, at most.
pub const RECHECK_EVERY: Duration = Duration::from_secs(5);
/// A title shorter than this is not looked for in other notes' text (too many false links).
const MIN_MENTION_CHARS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum NotesError {
    #[error("the notes folder {0} does not exist or is not a folder")]
    NoFolder(PathBuf),
    #[error("cannot read the notes folder: {0}")]
    Io(#[from] std::io::Error),
}

/// The longest text a capture may hold.
pub const MAX_CAPTURE_CHARS: usize = 4000;
/// Words of a capture that make its title.
const CAPTURE_TITLE_WORDS: usize = 10;
/// The longest title of a capture, in characters.
const CAPTURE_TITLE_CHARS: usize = 60;
/// The longest file-name part made from a title.
const MAX_SLUG_CHARS: usize = 50;

/// Why a capture was not written.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("there is nothing to remember")]
    Empty,
    #[error("that is too long to remember (at most {MAX_CAPTURE_CHARS} characters)")]
    TooLong,
    #[error("the notes folder does not exist")]
    NoFolder,
    #[error("the captures folder is not a plain folder inside the notes folder")]
    BadFolder,
    #[error("the date must look like 2026-11-11")]
    BadDate,
    #[error("could not write the note: {0}")]
    Io(#[from] std::io::Error),
    #[error("the note was written but the index could not be rebuilt: {0}")]
    Index(#[from] NotesError),
}

/// A note written by [`NotesService::capture`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Captured {
    pub id: u64,
    pub title: String,
    /// Relative to the notes folder, with `/`.
    pub path: String,
}

/// Caps that keep a huge folder from slowing the server down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_notes: usize,
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_notes: 2000,
            max_file_bytes: 256 * 1024,
        }
    }
}

/// A note's id: a hash of its relative path, so it stays the same when other notes are added,
/// removed or edited (links and `?note=` URLs keep pointing at the same note). 52 bits, so it is
/// exact as a JavaScript number.
pub fn stable_id(rel_path: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rel_path.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h & ((1 << 52) - 1)
}

/// One note.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Note {
    /// [`stable_id`] of the path.
    pub id: u64,
    pub title: String,
    /// The first sub-folder, or `notes` for a file at the top of the folder.
    pub group: String,
    /// Relative path with `/` separators.
    pub path: String,
    pub excerpt: String,
    /// Number of links to other notes.
    pub degree: usize,
    #[serde(skip)]
    title_terms: HashSet<String>,
    #[serde(skip)]
    body_terms: HashSet<String>,
}

/// A search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub id: u64,
    pub score: u32,
}

/// The notes of one folder at one moment.
#[derive(Debug, Clone, Default)]
pub struct NotesIndex {
    /// Sorted by path.
    notes: Vec<Note>,
    by_id: HashMap<u64, usize>,
    /// Undirected, between positions in `notes`, `(smaller, larger)`, sorted, no duplicates.
    edges: Vec<(usize, usize)>,
    skipped: usize,
}

impl NotesIndex {
    /// Read every `*.md` under `dir`.
    pub fn build(dir: &Path, limits: &Limits) -> Result<Self, NotesError> {
        if !dir.is_dir() {
            return Err(NotesError::NoFolder(dir.to_path_buf()));
        }
        let mut files = Vec::new();
        let mut skipped = 0;
        walk(dir, dir, &mut files, &mut skipped)?;
        files.sort();

        let mut raw: Vec<(String, String)> = Vec::new(); // (relative path, text)
        for rel in files {
            if raw.len() >= limits.max_notes {
                skipped += 1;
                continue;
            }
            let path = dir.join(&rel);
            let too_big =
                std::fs::metadata(&path).map_or(true, |m| m.len() > limits.max_file_bytes);
            if too_big {
                skipped += 1;
                continue;
            }
            match read_capped(&path, limits.max_file_bytes) {
                Some(text) => raw.push((rel, text)),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "some notes were skipped (too large, unreadable or over max_notes)"
            );
        }

        let mut notes = Vec::with_capacity(raw.len());
        let mut bodies = Vec::with_capacity(raw.len());
        for (rel, text) in &raw {
            let (title, body) = split_title(rel, text);
            notes.push(Note {
                id: stable_id(rel),
                group: group_of(rel),
                path: rel.clone(),
                excerpt: excerpt(&body, EXCERPT_CHARS),
                degree: 0,
                title_terms: terms(&title).collect(),
                body_terms: terms(&body).collect(),
                title,
            });
            bodies.push(body);
        }
        let edges = find_links(&notes, &bodies);
        for &(a, b) in &edges {
            notes[a].degree += 1;
            notes[b].degree += 1;
        }
        let by_id = notes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        Ok(Self {
            notes,
            by_id,
            edges,
            skipped,
        })
    }

    /// Every note, sorted by path.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The note with this id.
    pub fn get(&self, id: u64) -> Option<&Note> {
        self.by_id.get(&id).map(|&i| &self.notes[i])
    }

    /// Links as `(id, id)` pairs, each once.
    pub fn links(&self) -> Vec<(u64, u64)> {
        self.edges
            .iter()
            .map(|&(a, b)| (self.notes[a].id, self.notes[b].id))
            .collect()
    }

    /// Files left out (too large, unreadable, or over `max_notes`).
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The `k` best notes for `query`: 3 points per query word in the title, 1 per word in the
    /// body; stop words ignored; ties go to the shorter path. Notes that score 0 are left out.
    pub fn search(&self, query: &str, k: usize) -> Vec<Hit> {
        let words: BTreeSet<String> = terms(query).collect();
        if words.is_empty() || k == 0 {
            return Vec::new();
        }
        let mut scored: Vec<(usize, u32)> = self
            .notes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let score = words
                    .iter()
                    .map(|w| {
                        3 * u32::from(n.title_terms.contains(w))
                            + u32::from(n.body_terms.contains(w))
                    })
                    .sum();
                (i, score)
            })
            .filter(|&(_, score)| score > 0)
            .collect();
        scored.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| self.notes[a.0].path.len().cmp(&self.notes[b.0].path.len()))
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
            .into_iter()
            .map(|(i, score)| Hit {
                id: self.notes[i].id,
                score,
            })
            .collect()
    }
}

/// Every `*.md` below `dir`, relative to `root`, with `/` separators. Symbolic links, junctions
/// and hidden entries are skipped. Only the top folder must be readable: an unreadable entry
/// below it is counted in `skipped` and left out.
fn walk(
    root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
    skipped: &mut usize,
) -> Result<(), NotesError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if dir == root => return Err(e.into()),
        Err(_) => {
            *skipped += 1;
            return Ok(());
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *skipped += 1;
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        // Does not follow symbolic links; a Windows junction also reports as a symbolic link.
        let Ok(kind) = entry.file_type() else {
            *skipped += 1;
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            walk(root, &path, out, skipped)?;
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push(rel.join("/"));
        }
    }
    Ok(())
}

fn group_of(rel: &str) -> String {
    match rel.split_once('/') {
        Some((first, _)) => first.to_owned(),
        None => "notes".to_owned(),
    }
}

fn file_stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.strip_suffix(".md")
        .or_else(|| name.strip_suffix(".MD"))
        .unwrap_or(name)
}

/// The title (first `# ` heading, else the file name) and the body without front matter and
/// without the title line.
fn split_title(rel: &str, text: &str) -> (String, String) {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines: Vec<&str> = text.lines().collect();
    // YAML front matter.
    if lines.first().is_some_and(|l| l.trim() == "---")
        && let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---")
    {
        lines.drain(..end + 2);
    }
    let heading = lines
        .iter()
        .position(|l| l.starts_with("# ") && !l[2..].trim().is_empty());
    let title = match heading {
        Some(i) => lines.remove(i)[2..].trim().to_owned(),
        None => file_stem(rel).to_owned(),
    };
    (title, lines.join("\n"))
}

/// Plain text of the body, whitespace collapsed, cut at a word near `max` characters.
fn excerpt(body: &str, max: usize) -> String {
    let flat = body
        .lines()
        .map(|l| l.trim_start_matches(['#', '>', ' ']).trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    match cut.rfind(' ') {
        Some(i) if i > max / 2 => format!("{}…", &cut[..i]),
        _ => format!("{cut}…"),
    }
}

const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has", "have",
    "how", "i", "if", "in", "into", "is", "it", "its", "me", "my", "no", "not", "of", "on", "or",
    "our", "please", "should", "so", "tell", "than", "that", "the", "their", "them", "then",
    "there", "these", "this", "to", "up", "was", "we", "were", "what", "when", "where", "which",
    "who", "why", "will", "with", "would", "you", "your",
];

/// Lower-case words of `text`, stop words dropped, a plural `s` removed.
fn terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .map(|w| {
            if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
                w[..w.len() - 1].to_owned()
            } else {
                w
            }
        })
}

/// Lower case, `-` and `_` as spaces, whitespace collapsed: how titles and link targets match.
fn normal(s: &str) -> String {
    s.to_lowercase()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `[[Target]]`, `[[Target|label]]` and `[[Target#heading]]` targets in `body`.
fn wikilinks(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else { break };
        let inner = &rest[..end];
        let target = inner.split(['|', '#']).next().unwrap_or_default().trim();
        if !target.is_empty() && !target.contains('\n') {
            out.push(normal(target));
        }
        rest = &rest[end + 2..];
    }
    out
}

fn find_links(notes: &[Note], bodies: &[String]) -> Vec<(usize, usize)> {
    // Names a note answers to: its title and its file name.
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, n) in notes.iter().enumerate() {
        by_name.entry(normal(file_stem(&n.path))).or_insert(i);
    }
    for (i, n) in notes.iter().enumerate() {
        by_name.insert(normal(&n.title), i);
    }
    let mut links = BTreeSet::new();
    let mut add = |a: usize, b: usize| {
        if a != b {
            links.insert((a.min(b), a.max(b)));
        }
    };
    let lowered: Vec<String> = bodies.iter().map(|b| normal(b)).collect();
    for (i, body) in bodies.iter().enumerate() {
        for target in wikilinks(body) {
            if let Some(&j) = by_name.get(&target) {
                add(i, j);
            }
        }
    }
    // Mentions of another note's title. A cheap word check first, the substring search only
    // for the notes that have every word of the title.
    for (ti, target) in notes.iter().enumerate() {
        let title = normal(&target.title);
        if title.chars().count() < MIN_MENTION_CHARS || target.title_terms.is_empty() {
            continue;
        }
        for (si, source) in notes.iter().enumerate() {
            if si == ti
                || !target
                    .title_terms
                    .iter()
                    .all(|t| source.body_terms.contains(t))
            {
                continue;
            }
            if contains_phrase(&lowered[si], &title) {
                add(si, ti);
            }
        }
    }
    links.into_iter().collect()
}

/// `phrase` occurs in `text` on word boundaries.
pub fn contains_phrase(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + phrase.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// What changes when a note is added, removed, renamed, moved or edited: a hash over every
/// note's path, size and modification time.
fn signature(dir: &Path) -> Option<u64> {
    let mut files = Vec::new();
    let mut skipped = 0;
    walk(dir, dir, &mut files, &mut skipped).ok()?;
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for rel in &files {
        feed(rel.as_bytes());
        if let Ok(m) = std::fs::metadata(dir.join(rel)) {
            feed(&m.len().to_le_bytes());
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            feed(&t.to_le_bytes());
        }
        feed(b"\n");
    }
    feed(&skipped.to_le_bytes());
    Some(h)
}

/// The file's text when it is at most `max` bytes (checked on the open file, and again while
/// reading, so a file that grew is never read whole).
fn read_capped(path: &Path, max: u64) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > max {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

struct Cached {
    checked: Instant,
    signature: Option<u64>,
    index: Arc<NotesIndex>,
}

/// The index of one folder, rebuilt when the folder changes (checked at most every
/// [`RECHECK_EVERY`]). Cheap to share.
pub struct NotesService {
    dir: PathBuf,
    limits: Limits,
    recheck: Duration,
    cache: Mutex<Option<Cached>>,
}

impl NotesService {
    pub fn new(dir: PathBuf, limits: Limits) -> Self {
        Self {
            dir,
            limits,
            recheck: RECHECK_EVERY,
            cache: Mutex::new(None),
        }
    }

    /// Another check interval (tests).
    pub fn with_recheck(mut self, recheck: Duration) -> Self {
        self.recheck = recheck;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The current index, rebuilt first when the folder changed since the last look.
    pub fn index(&self) -> Result<Arc<NotesIndex>, NotesError> {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = cache.as_mut() {
            if c.checked.elapsed() < self.recheck {
                return Ok(c.index.clone());
            }
            let now = signature(&self.dir);
            c.checked = Instant::now();
            if now == c.signature {
                return Ok(c.index.clone());
            }
        }
        self.rebuild(&mut cache)
    }

    /// Rebuild now, whatever the interval.
    pub fn rescan(&self) -> Result<Arc<NotesIndex>, NotesError> {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        self.rebuild(&mut cache)
    }

    fn rebuild(&self, cache: &mut Option<Cached>) -> Result<Arc<NotesIndex>, NotesError> {
        let signature = signature(&self.dir);
        let index = Arc::new(NotesIndex::build(&self.dir, &self.limits)?);
        *cache = Some(Cached {
            checked: Instant::now(),
            signature,
            index: index.clone(),
        });
        Ok(index)
    }

    /// The full text of note `id` in `index`, or `None`. Takes the index snapshot the caller
    /// already has, so a rebuild in between cannot pair one note's title with another's text.
    pub fn text_of(&self, index: &NotesIndex, id: u64) -> Option<String> {
        let note = index.get(id)?;
        // The path came from walking the folder (no `..`, no links) and is joined to it again.
        read_capped(&self.dir.join(&note.path), self.limits.max_file_bytes)
    }

    /// The full text of note `id` in the current index, or `None`.
    pub fn text(&self, id: u64) -> Option<String> {
        let index = self.index().ok()?;
        self.text_of(&index, id)
    }

    /// "Remember that ...": write `text` as a new note `<sub>/<date>-<slug>.md` and rebuild the
    /// index so the very next search finds it. `sub` is one plain folder name inside the notes
    /// folder; `date` is `YYYY-MM-DD`. Never overwrites: a taken name gets `-2`, `-3`, ...
    pub fn capture(&self, sub: &str, text: &str, date: &str) -> Result<Captured, CaptureError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(CaptureError::Empty);
        }
        if text.chars().count() > MAX_CAPTURE_CHARS {
            return Err(CaptureError::TooLong);
        }
        let plain_name = !sub.is_empty()
            && !sub.starts_with('.')
            && sub
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !plain_name {
            return Err(CaptureError::BadFolder);
        }
        let date_ok = date.len() == 10
            && date.char_indices().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            });
        if !date_ok {
            return Err(CaptureError::BadDate);
        }
        if !self.dir.is_dir() {
            return Err(CaptureError::NoFolder);
        }
        let folder = self.dir.join(sub);
        match std::fs::symlink_metadata(&folder) {
            // A symbolic link or a Windows junction (reported as a link) could point anywhere.
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                return Err(CaptureError::BadFolder);
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(&folder)?,
            Err(e) => return Err(e.into()),
        }

        let title = capture_title(text);
        let slug = slug_of(&title);
        let body = format!("# {title}\n\n{text}\n\nCaptured on {date} by Jarvis.\n");
        for n in 1..=99u32 {
            let name = if n == 1 {
                format!("{date}-{slug}.md")
            } else {
                format!("{date}-{slug}-{n}.md")
            };
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(folder.join(&name));
            match file {
                Ok(mut f) => {
                    use std::io::Write;
                    let written = f.write_all(body.as_bytes()).and_then(|()| f.sync_all());
                    drop(f);
                    if let Err(e) = written {
                        // No half-written note left behind.
                        let _ = std::fs::remove_file(folder.join(&name));
                        return Err(e.into());
                    }
                    let path = format!("{sub}/{name}");
                    // The note is saved. If the index cannot be rebuilt now, the next look at
                    // the folder will pick it up: do not report the save as failed.
                    if let Err(e) = self.rescan() {
                        tracing::warn!(
                            path,
                            "the note was saved but the index was not rebuilt: {e}"
                        );
                    }
                    return Ok(Captured {
                        id: stable_id(&path),
                        title,
                        path,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(CaptureError::Io(std::io::Error::other(
            "too many notes with the same name today",
        )))
    }
}

/// The first words of a capture, without path or markup characters: "My broker closes gold
/// trading at 23:55 server time". At most [`CAPTURE_TITLE_WORDS`] words and
/// [`CAPTURE_TITLE_CHARS`] characters, cut at a whole word. "Note" when nothing readable is left.
fn capture_title(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            // `:` keeps times like 23:55 whole; the file name drops it.
            if c.is_alphanumeric() || matches!(c, '\'' | '-' | ',' | '.' | ':') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut title = String::new();
    for word in cleaned
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .take(CAPTURE_TITLE_WORDS)
    {
        let extra = usize::from(!title.is_empty()) + word.chars().count();
        if !title.is_empty() && title.chars().count() + extra > CAPTURE_TITLE_CHARS {
            break;
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    let title: String = title.chars().take(CAPTURE_TITLE_CHARS).collect();
    let title = title.trim_end_matches(['.', ',', '-', ':']);
    let mut chars = title.chars();
    match chars.next() {
        None => "Note".to_owned(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

/// Lowercase ASCII letters and digits joined by `-`, at most [`MAX_SLUG_CHARS`]; "note" when empty.
fn slug_of(title: &str) -> String {
    let mut slug = String::new();
    for word in title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let word = word.to_ascii_lowercase();
        if slug.len() + word.len() + 1 > MAX_SLUG_CHARS {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&word);
    }
    if slug.is_empty() {
        "note".to_owned()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wikilink_targets_are_normalised() {
        assert_eq!(
            wikilinks("see [[Risk per trade]], [[Broker-Quirks|broker]] and [[Plan#Exits]] [[]]"),
            ["risk per trade", "broker quirks", "plan"]
        );
    }

    #[test]
    fn phrases_match_on_word_boundaries_only() {
        assert!(contains_phrase("the gold rule applies", "gold rule"));
        assert!(!contains_phrase("the goldrule applies", "gold rule"));
        assert!(!contains_phrase("marigold rules", "gold rule"));
    }

    #[test]
    fn terms_drop_stop_words_and_plurals() {
        let t: Vec<String> = terms("What are my RULES for the Gold trades?").collect();
        assert_eq!(t, ["rule", "gold", "trade"]);
    }

    #[test]
    fn long_excerpts_are_cut_at_a_word() {
        let body = "word ".repeat(300);
        let e = excerpt(&body, 50);
        assert!(e.ends_with('…') && e.chars().count() <= 51, "{e}");
    }
}
