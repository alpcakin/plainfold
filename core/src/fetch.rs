// Downloads into the store: a language server as npm tarballs at pinned versions, or a program built for each system
// (Node.js) as one pinned file. Each download is checked against its pinned hash before anything is unpacked. Only
// plain files and folders are unpacked, and only inside the target.
// A half-finished download never counts: everything goes to a temporary place that is renamed into place at the end.
// Versions that are no longer pinned are removed (`prune`).
use crate::servers::{NODE, Program, RECIPES, Recipe};
use base64::Engine;
use sha2::{Digest, Sha256, Sha512};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Downloads and unpacks `r` into `store` unless it is already there. `on(done, total)` hears the bytes downloaded
/// so far and the total announced so far.
pub fn install(r: &Recipe, store: &Path, on: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    let dir = r.dir(store);
    if dir.join("node_modules").join(r.entry).is_file() {
        return Ok(dir);
    }
    let tmp = dir.with_extension(format!("part-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let (mut done, mut total) = (0u64, 0u64);
    for p in r.packages {
        // a scoped package "@scope/name" keeps its tarball as "@scope/name/-/name-1.0.0.tgz"
        let file = p.name.rsplit('/').next().unwrap_or(p.name);
        let url = format!("https://registry.npmjs.org/{}/-/{}-{}.tgz", p.name, file, p.version);
        let bytes = download(&url, &mut |d, t| {
            if t > 0 && d == 0 {
                total += t;
            }
            on(done + d, total);
        })?;
        done += bytes.len() as u64;
        check(&bytes, p.integrity).map_err(|e| format!("{} {}: {e}", p.name, p.version))?;
        unpack(&bytes, &tmp.join("node_modules").join(p.name)).map_err(|e| format!("{} {}: {e}", p.name, p.version))?;
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Downloads the build of `p` for this system into `store` unless it is already there, and returns the program.
pub fn install_program(p: &Program, store: &Path, on: &dyn Fn(u64, u64)) -> Result<PathBuf, String> {
    let exe = p.exe(store);
    if exe.is_file() {
        return Ok(exe);
    }
    let b = p.build().ok_or_else(|| format!("plainfold has no {} for this system", p.label))?;
    let bytes = download(b.url, &mut |d, t| on(d, t))?;
    let named = |e: String| format!("{} {}: {e}", p.label, p.version);
    let got: String = Sha256::digest(&bytes).iter().map(|x| format!("{x:02x}")).collect();
    if got != b.sha256 {
        return Err(named("the download does not match its pinned hash".into()));
    }
    let dir = exe.parent().ok_or("no folder for the program")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = exe.with_extension(format!("part-{}", std::process::id()));
    let written = match b.inside {
        Some(inside) => take(&bytes, inside, &tmp),
        None => std::fs::write(&tmp, &bytes).map_err(|e| e.to_string()),
    };
    if let Err(e) = written.and_then(|_| runnable(&tmp)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(named(e));
    }
    std::fs::rename(&tmp, &exe).map_err(|e| e.to_string())?;
    Ok(exe)
}

/// Removes from `store` what this version of plainfold no longer runs: every version of a server or of Node.js other
/// than the pinned one, and downloads left half-finished more than an hour ago (a newer one may still be under way in
/// another window). Folders it does not know are left alone. Returns what was removed.
pub fn prune(store: &Path) -> Vec<PathBuf> {
    let mut keep: Vec<(PathBuf, PathBuf)> = RECIPES.iter().map(|r| r.dir(store)).filter_map(|d| Some((d.parent()?.to_path_buf(), d))).collect();
    keep.extend(NODE.exe(store).parent().and_then(|d| Some((d.parent()?.to_path_buf(), d.to_path_buf()))));
    let mut removed = vec![];
    for (folder, current) in keep {
        for e in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let p = e.path();
            if p == current {
                continue;
            }
            let half = p.file_name().is_some_and(|n| n.to_string_lossy().contains(".part-"));
            let age = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).unwrap_or_default();
            if half && age < std::time::Duration::from_secs(3600) {
                continue;
            }
            let gone = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            if gone.is_ok() {
                removed.push(p);
            }
        }
    }
    removed
}

/// Writes the one file at `inside` of a `.tar.gz` to `to`.
fn take(tgz: &[u8], inside: &str, to: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.header().entry_type().is_file() && entry.path().is_ok_and(|p| p == Path::new(inside)) {
            let mut out = std::fs::File::create(to).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("{inside} is not in the download"))
}

#[cfg(unix)]
fn runnable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn runnable(_: &Path) -> Result<(), String> {
    Ok(())
}

fn download(url: &str, on: &mut dyn FnMut(u64, u64)) -> Result<Vec<u8>, String> {
    let mut resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    on(0, total);
    let mut reader = resp.body_mut().as_reader();
    let mut out = Vec::with_capacity(total as usize);
    let mut chunk = vec![0u8; 1 << 16];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
        on(out.len() as u64, total);
    }
    Ok(out)
}

/// npm's integrity field: "sha512-" and the base64 of the tarball's SHA-512.
fn check(bytes: &[u8], integrity: &str) -> Result<(), String> {
    let want = integrity.strip_prefix("sha512-").ok_or("the pinned hash is not a SHA-512")?;
    let got = base64::engine::general_purpose::STANDARD.encode(Sha512::digest(bytes));
    if got == want { Ok(()) } else { Err("the download does not match its pinned hash".into()) }
}

fn unpack(tgz: &[u8], into: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tgz));
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            continue; // links and devices are never unpacked
        }
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        // npm keeps a package under one top folder ("package/")
        let rel: PathBuf = path.components().skip(1).collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("unsafe path in the archive: {}", path.display()));
        }
        let to = into.join(&rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry.unpack(&to).map_err(|e| format!("{}: {e}", rel.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_hash_is_refused() {
        assert!(check(b"something else", "sha512-c5hEHM/7rdFRbQWoHXuddyJ53oF0M2dKeXQOYdwUek556oarltKeumKJGLp/ZMcGV7gW30mGn1MR3V5GDiLiKg==").is_err());
        assert!(check(b"x", "sha1-abc").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn only_the_pinned_versions_are_kept() {
        let store = std::env::temp_dir().join(format!("plainfold-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&store);
        let r = &RECIPES[0];
        let current = r.dir(&store);
        let old = current.with_file_name(format!("{}@0.0.1", r.packages[0].name));
        let half = current.with_extension("part-1");
        let fresh_half = current.with_extension(format!("part-{}", std::process::id()));
        let node = NODE.exe(&store).parent().unwrap().to_path_buf();
        let old_node = node.with_file_name("node@1.0.0");
        let unknown = store.join("something-else").join("x@1");
        for d in [&current, &old, &half, &fresh_half, &node, &old_node, &unknown] {
            std::fs::create_dir_all(d).unwrap();
        }
        // a half-finished download an hour old
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::File::open(&half).unwrap().set_modified(hour_ago).unwrap();
        let removed = prune(&store);
        assert!(current.exists() && node.exists() && unknown.exists() && fresh_half.exists());
        assert!(!old.exists() && !old_node.exists() && !half.exists());
        assert!(removed.contains(&old) && removed.contains(&old_node));
        let _ = std::fs::remove_dir_all(&store);
    }

    #[test]
    fn the_right_hash_is_accepted() {
        let h = base64::engine::general_purpose::STANDARD.encode(Sha512::digest(b"x"));
        assert!(check(b"x", &format!("sha512-{h}")).is_ok());
    }
}
