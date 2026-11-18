//! The notes index and search over a small fixture folder.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jarvis_notes::{Limits, NotesIndex, NotesService, stable_id};

fn vault() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault")
}

fn index() -> NotesIndex {
    NotesIndex::build(&vault(), &Limits::default()).unwrap()
}

fn id_of(ix: &NotesIndex, title: &str) -> u64 {
    ix.notes()
        .iter()
        .find(|n| n.title == title)
        .unwrap_or_else(|| panic!("no note titled {title}"))
        .id
}

#[test]
fn notes_have_titles_groups_and_ids_from_their_paths() {
    let ix = index();
    let titles: Vec<(&str, &str, &str)> = ix
        .notes()
        .iter()
        .map(|n| (n.path.as_str(), n.title.as_str(), n.group.as_str()))
        .collect();
    // Sorted by path. Hidden folders and non-markdown files are skipped.
    assert_eq!(
        titles,
        [
            ("broker-quirks.md", "broker-quirks", "notes"),
            ("lessons/revenge.md", "Revenge trading", "lessons"),
            ("plans/gold-news.md", "Gold news-day rule", "plans"),
            ("plans/risk.md", "Risk per trade", "plans"),
        ]
    );
    for n in ix.notes() {
        assert_eq!(n.id, stable_id(&n.path));
        assert!(n.id < (1 << 52), "exact as a JavaScript number");
        assert_eq!(ix.get(n.id).map(|x| &x.path), Some(&n.path));
    }
    assert!(ix.get(12345).is_none());
}

#[test]
fn excerpts_skip_front_matter_and_the_title_line() {
    let ix = index();
    let risk = ix.get(id_of(&ix, "Risk per trade")).unwrap();
    assert!(
        risk.excerpt.starts_with("Never risk more than 1%"),
        "{}",
        risk.excerpt
    );
    assert!(!risk.excerpt.contains("tags:"));
}

#[test]
fn wikilinks_and_title_mentions_become_links() {
    let ix = index();
    let gold = id_of(&ix, "Gold news-day rule");
    let risk = id_of(&ix, "Risk per trade");
    let quirks = id_of(&ix, "broker-quirks");
    let links = ix.links();
    let has = |a: u64, b: u64| links.iter().any(|&l| l == (a, b) || l == (b, a));
    assert!(has(gold, risk), "[[Risk per trade]]: {links:?}");
    // [[Broker quirks|my broker]] matches the file name, case-insensitively.
    assert!(has(gold, quirks), "[[Broker quirks]]: {links:?}");
    // broker-quirks.md mentions the title "Gold news-day rule": the same pair, counted once.
    let count = links
        .iter()
        .filter(|&&l| l == (gold, quirks) || l == (quirks, gold))
        .count();
    assert_eq!(count, 1);
    let revenge = id_of(&ix, "Revenge trading");
    assert!(links.iter().all(|&(a, b)| a != revenge && b != revenge));
    assert_eq!(ix.get(gold).unwrap().degree, 2);
}

#[test]
fn search_ranks_titles_above_bodies_and_skips_stop_words() {
    let ix = index();
    let hits = ix.search("what is my rule for gold on news days?", 6);
    assert_eq!(
        hits.first().map(|h| h.id),
        Some(id_of(&ix, "Gold news-day rule"))
    );
    let hits = ix.search("risk", 6);
    assert_eq!(
        hits[0].id,
        id_of(&ix, "Risk per trade"),
        "title match first"
    );
    assert!(ix.search("the and of", 6).is_empty(), "stop words only");
    assert!(ix.search("", 6).is_empty());
    assert!(ix.search("cryptocurrency staking", 6).is_empty());
    assert_eq!(ix.search("trade", 1).len(), 1, "k caps the results");
}

#[test]
fn limits_skip_large_files_and_cap_the_count() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), "# A\nsmall").unwrap();
    std::fs::write(dir.path().join("b.md"), "x".repeat(4 * 1024)).unwrap();
    std::fs::write(dir.path().join("c.md"), "# C\nsmall").unwrap();
    let limits = Limits {
        max_notes: 1,
        max_file_bytes: 1024,
    };
    let ix = NotesIndex::build(dir.path(), &limits).unwrap();
    assert_eq!(ix.notes().len(), 1);
    assert_eq!(ix.skipped(), 2, "one too large, one over the count");
}

#[test]
fn a_missing_folder_is_an_error_and_an_empty_one_is_empty() {
    assert!(NotesIndex::build(Path::new("no/such/folder"), &Limits::default()).is_err());
    let dir = tempfile::tempdir().unwrap();
    let ix = NotesIndex::build(dir.path(), &Limits::default()).unwrap();
    assert!(ix.notes().is_empty());
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "# Secret").unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
    let ix = NotesIndex::build(dir.path(), &Limits::default()).unwrap();
    assert!(ix.notes().is_empty());
}

#[cfg(windows)]
#[test]
fn junctions_are_not_followed() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "# Secret").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("link");
    // A directory junction needs no administrator rights.
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(made.status.success(), "mklink /J failed: {made:?}");
    assert!(link.join("secret.md").exists(), "the junction works");
    let ix = NotesIndex::build(dir.path(), &Limits::default()).unwrap();
    assert!(ix.notes().is_empty(), "{:?}", ix.notes());
}

#[test]
fn ids_stay_the_same_when_another_note_is_added() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("m.md"), "# M\nmiddle").unwrap();
    let before = NotesIndex::build(dir.path(), &Limits::default()).unwrap();
    let id = before.notes()[0].id;
    std::fs::write(dir.path().join("a.md"), "# A\nfirst by path").unwrap();
    let after = NotesIndex::build(dir.path(), &Limits::default()).unwrap();
    assert_eq!(after.get(id).unwrap().title, "M");
}

#[test]
fn the_service_picks_up_changes_after_the_interval() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), "# A\none").unwrap();
    let svc =
        NotesService::new(dir.path().to_path_buf(), Limits::default()).with_recheck(Duration::ZERO);
    assert_eq!(svc.index().unwrap().notes().len(), 1);
    std::fs::write(dir.path().join("b.md"), "# B\ntwo").unwrap();
    assert_eq!(svc.index().unwrap().notes().len(), 2);

    let slow = NotesService::new(dir.path().to_path_buf(), Limits::default())
        .with_recheck(Duration::from_secs(3600));
    assert_eq!(slow.index().unwrap().notes().len(), 2);
    std::fs::write(dir.path().join("c.md"), "# C\nthree").unwrap();
    assert_eq!(
        slow.index().unwrap().notes().len(),
        2,
        "inside the interval"
    );
    assert_eq!(
        slow.rescan().unwrap().notes().len(),
        3,
        "an explicit rescan"
    );
}

#[test]
fn a_rename_or_move_is_noticed_even_with_the_same_size_and_time() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("plans")).unwrap();
    std::fs::write(dir.path().join("a.md"), "# Rule\nsame text").unwrap();
    let svc =
        NotesService::new(dir.path().to_path_buf(), Limits::default()).with_recheck(Duration::ZERO);
    assert_eq!(svc.index().unwrap().notes()[0].group, "notes");
    // A move keeps the size and the modification time.
    std::fs::rename(dir.path().join("a.md"), dir.path().join("plans/a.md")).unwrap();
    let ix = svc.index().unwrap();
    assert_eq!(ix.notes()[0].group, "plans");
    assert_eq!(ix.notes()[0].path, "plans/a.md");
}

#[test]
fn note_text_comes_from_the_same_snapshot_and_respects_the_cap() {
    let svc = NotesService::new(vault(), Limits::default());
    let ix = svc.index().unwrap();
    let id = id_of(&ix, "Revenge trading");
    assert!(svc.text_of(&ix, id).unwrap().contains("After two losses"));
    assert!(svc.text(id).unwrap().contains("After two losses"));
    assert!(svc.text(999).is_none());

    // A file that grew past the cap after it was indexed is not read.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("g.md"), "# G\nsmall").unwrap();
    let limits = Limits {
        max_notes: 10,
        max_file_bytes: 64,
    };
    let svc = NotesService::new(dir.path().to_path_buf(), limits);
    let ix = svc.index().unwrap();
    let id = ix.notes()[0].id;
    std::fs::write(dir.path().join("g.md"), "x".repeat(1000)).unwrap();
    assert!(svc.text_of(&ix, id).is_none());
}

// --- Captures ("remember that ...", spec-assistant-mode) ---

use jarvis_notes::CaptureError;

fn capture_service() -> (tempfile::TempDir, NotesService) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rules.md"), "# Rules\nbe patient").unwrap();
    let svc = NotesService::new(dir.path().to_path_buf(), Limits::default())
        .with_recheck(Duration::from_secs(3600));
    (dir, svc)
}

#[test]
fn a_capture_writes_a_dated_note_and_is_searchable_at_once() {
    let (dir, svc) = capture_service();
    svc.index().unwrap();
    let c = svc
        .capture(
            "captures",
            "the finish window should be 900 milliseconds",
            "2026-11-11",
        )
        .unwrap();
    assert_eq!(
        c.path,
        "captures/2026-11-11-the-finish-window-should-be-900-milliseconds.md"
    );
    assert_eq!(c.title, "The finish window should be 900 milliseconds");
    assert_eq!(c.id, stable_id(&c.path));
    let text = std::fs::read_to_string(dir.path().join(&c.path)).unwrap();
    assert!(
        text.starts_with("# The finish window should be 900 milliseconds\n"),
        "{text}"
    );
    assert!(text.contains("the finish window should be 900 milliseconds"));
    assert!(text.contains("2026-11-11"));
    // Indexed without waiting for the recheck interval.
    let ix = svc.index().unwrap();
    assert_eq!(ix.get(c.id).unwrap().group, "captures");
    assert_eq!(ix.search("finish window", 3)[0].id, c.id);
}

#[test]
fn a_capture_never_overwrites_a_note() {
    let (dir, svc) = capture_service();
    let a = svc.capture("captures", "same words", "2026-11-11").unwrap();
    let b = svc.capture("captures", "same words", "2026-11-11").unwrap();
    assert_ne!(a.path, b.path);
    assert_eq!(b.path, "captures/2026-11-11-same-words-2.md");
    assert!(dir.path().join(&a.path).exists() && dir.path().join(&b.path).exists());
}

#[test]
fn capture_names_are_safe_whatever_the_text() {
    let (dir, svc) = capture_service();
    let c = svc
        .capture(
            "captures",
            "../../etc/passwd: <script>CON</script>",
            "2026-11-11",
        )
        .unwrap();
    assert!(c.path.starts_with("captures/2026-11-11-"), "{}", c.path);
    assert!(
        !c.path[9..].contains(['/', '\\', '<', '>', ':']),
        "{}",
        c.path
    );
    assert!(dir.path().join(&c.path).exists());
    // Punctuation-only text still gets a name.
    let c = svc.capture("captures", "!!!", "2026-11-11").unwrap();
    assert_eq!(c.path, "captures/2026-11-11-note.md");
}

#[test]
fn bad_captures_are_refused() {
    let (_dir, svc) = capture_service();
    assert!(matches!(
        svc.capture("captures", "   ", "2026-11-11"),
        Err(CaptureError::Empty)
    ));
    assert!(matches!(
        svc.capture("captures", &"x".repeat(5000), "2026-11-11"),
        Err(CaptureError::TooLong)
    ));
    for sub in ["../out", "a/b", "", ".hidden", "C:"] {
        assert!(
            matches!(
                svc.capture(sub, "fine", "2026-11-11"),
                Err(CaptureError::BadFolder)
            ),
            "{sub}"
        );
    }
    assert!(matches!(
        svc.capture("captures", "fine", "11/11/2026"),
        Err(CaptureError::BadDate)
    ));
}

#[test]
fn a_capture_needs_the_notes_folder() {
    let svc = NotesService::new(PathBuf::from("no/such/notes"), Limits::default());
    assert!(matches!(
        svc.capture("captures", "fine", "2026-11-11"),
        Err(CaptureError::NoFolder)
    ));
}

#[cfg(windows)]
#[test]
fn a_capture_folder_that_is_a_junction_is_refused() {
    let outside = tempfile::tempdir().unwrap();
    let (dir, svc) = capture_service();
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(dir.path().join("captures"))
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(made.status.success());
    assert!(matches!(
        svc.capture("captures", "fine", "2026-11-11"),
        Err(CaptureError::BadFolder)
    ));
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn capture_titles_keep_times_and_stop_at_a_whole_word() {
    let (_dir, svc) = capture_service();
    let c = svc
        .capture(
            "captures",
            "my broker closes gold trading at 23:55 server time",
            "2026-11-11",
        )
        .unwrap();
    assert_eq!(
        c.title,
        "My broker closes gold trading at 23:55 server time"
    );
    assert_eq!(
        c.path,
        "captures/2026-11-11-my-broker-closes-gold-trading-at-23-55-server-time.md"
    );
    let long =
        "remember to review the weekly plan every friday afternoon before the london close please";
    let c = svc.capture("captures", long, "2026-11-11").unwrap();
    assert!(c.title.chars().count() <= 60, "{}", c.title);
    assert!(
        long.to_lowercase().starts_with(&c.title.to_lowercase()),
        "{}",
        c.title
    );
    assert!(!c.title.ends_with(' '));
}
