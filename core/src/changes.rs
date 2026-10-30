// Noticing that the code changed since the project was read. Only noticing: reading the project again is a separate,
// slower step the user starts, because a save may hold half-written code.
// A file counts as changed when its content does (SHA-256), not when it was only touched or saved unchanged, so a
// change that is undone stops counting. The files are the ones the core reads (`source::own_sources`).
use crate::lang::lang_for;
use crate::source::{NOT_OWN, own_sources, rel_path};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{MAIN_SEPARATOR, Path};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// The content of each of the project's own source files: path relative to the root → SHA-256.
pub type Hashes = BTreeMap<String, [u8; 32]>;

pub fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The files the core reads, hashed as they are now.
pub fn hashes(root: &Path) -> Hashes {
    own_sources(root).into_iter().filter_map(|(abs, _)| Some((rel_path(root, &abs), hash(&std::fs::read(&abs).ok()?)))).collect()
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum How {
    Added,
    Changed,
    Removed,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub file: String,
    pub how: How,
}

/// What differs from `old` to `new`, by file.
pub fn compare(old: &Hashes, new: &Hashes) -> Vec<Change> {
    let mut out: Vec<Change> = old
        .iter()
        .filter_map(|(file, h)| match new.get(file) {
            None => Some(How::Removed),
            Some(n) if n != h => Some(How::Changed),
            Some(_) => None,
        }
        .map(|how| Change { file: file.clone(), how }))
        .chain(new.keys().filter(|f| !old.contains_key(*f)).map(|f| Change { file: f.clone(), how: How::Added }))
        .collect();
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

/// How long a burst of file events (a formatter, an agent writing several files) may run before the files are hashed.
const SETTLE: Duration = Duration::from_millis(300);

/// Watches a project. Stops when dropped.
pub struct Watch {
    _watcher: notify::RecommendedWatcher,
}

/// Watches `root` and calls `on` with every change since `since` whenever that list changes: once at the start if
/// something already changed while the project was being read, and an empty list when the files are back to `since`.
pub fn watch(root: &Path, since: Hashes, on: impl Fn(Vec<Change>) + Send + 'static) -> notify::Result<Watch> {
    use notify::Watcher;
    let root = std::fs::canonicalize(root)?;
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(&root, notify::RecursiveMode::Recursive)?;
    std::thread::spawn(move || {
        let mut told = compare(&since, &hashes(&root));
        if !told.is_empty() {
            on(told.clone());
        }
        // the channel closes when the watcher is dropped, which ends this thread
        while let Ok(event) = rx.recv() {
            if !matters(&root, &since, &event) {
                continue;
            }
            let until = Instant::now() + SETTLE;
            loop {
                match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                    Ok(_) => continue,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            let now = compare(&since, &hashes(&root));
            if now != told {
                on(now.clone());
                told = now;
            }
        }
    });
    Ok(Watch { _watcher: watcher })
}

/// Whether a file event may concern a file the core reads: a source file, or a folder that holds or may hold some
/// (renamed, moved in, deleted). Events in dependency folders and hidden folders do not; an error might.
fn matters(root: &Path, since: &Hashes, event: &notify::Result<notify::Event>) -> bool {
    let Ok(event) = event else { return true };
    if matches!(event.kind, notify::EventKind::Access(_)) {
        return false;
    }
    event.paths.iter().any(|p| {
        let Ok(rel) = p.strip_prefix(root) else { return false };
        let skipped = rel.components().any(|c| {
            let n = c.as_os_str().to_string_lossy();
            n.starts_with('.') || NOT_OWN.contains(&n.as_ref())
        });
        if skipped {
            return false;
        }
        let dir = format!("{}{MAIN_SEPARATOR}", rel.to_string_lossy());
        lang_for(p).is_some() || p.is_dir() || since.range(dir.clone()..).next().is_some_and(|(f, _)| f.starts_with(&dir))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(pairs: &[(&str, u8)]) -> Hashes {
        pairs.iter().map(|(f, b)| (f.to_string(), hash(&[*b]))).collect()
    }

    #[test]
    fn compares_by_content() {
        let old = h(&[("a.ts", 1), ("b.ts", 2), ("c.ts", 3)]);
        let new = h(&[("a.ts", 1), ("b.ts", 9), ("d.ts", 4)]);
        let got = compare(&old, &new);
        let got: Vec<(&str, How)> = got.iter().map(|c| (c.file.as_str(), c.how)).collect();
        assert_eq!(got, vec![("b.ts", How::Changed), ("c.ts", How::Removed), ("d.ts", How::Added)]);
        assert!(compare(&old, &old).is_empty());
    }

    #[test]
    fn watch_tells_a_change_and_its_undo() {
        let root = std::env::temp_dir().join(format!("plainfold-watch-{}", std::process::id()));
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        let file = root.join("a.ts");
        std::fs::write(&file, "export const a = 1;\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = watch(&root, hashes(&root), move |c| tx.send(c).unwrap()).unwrap();
        let wait = || rx.recv_timeout(Duration::from_secs(10)).expect("no change was told");
        // a dependency folder is not the project's code
        std::fs::write(root.join("node_modules/b.ts"), "x").unwrap();
        std::fs::write(&file, "export const a = 2;\n").unwrap();
        assert_eq!(wait(), vec![Change { file: "a.ts".into(), how: How::Changed }]);
        std::fs::write(&file, "export const a = 1;\n").unwrap();
        assert_eq!(wait(), vec![]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
