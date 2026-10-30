// Packages the project imports that are not installed where its language server looks: which of the project's
// manifests list each one, where it was looked for, and the command the project's own tools would install what a
// manifest lists with. Read from files only. plainfold never runs that command: installing writes into the project
// and runs the packages' own install code, so the user runs it.
//
// The manifests read: `requirements*.txt`, `pyproject.toml`, the `install_requires` of `setup.py`, `package.json`
// and `pubspec.yaml`. A name is matched as the manifest writes it; Python names are compared as pip compares them (case, `-`, `_` and
// `.` alike). A package imported under another name than the one it is installed as (`import jose`, installed as
// `python-jose`) is found in no manifest, and that is said. A command is given only where the files show the tool:
// a lockfile, the tool that made the environment (`uv` writes itself into `pyvenv.cfg`), Flutter in `pubspec.yaml`.
use crate::model::NotInstalled;
use std::path::{Path, PathBuf};

/// How deep under the project's folder manifests are looked for.
const DEPTH: usize = 5;

/// The manifest files of the project, in path order, outside other people's code and environments.
fn manifests(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut dirs = vec![(root.to_path_buf(), 0)];
    while let Some((d, depth)) = dirs.pop() {
        let Ok(list) = std::fs::read_dir(&d) else { continue };
        for e in list.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                let skip = name.starts_with('.') || crate::source::NOT_OWN.contains(&name.as_str()) || p.join("pyvenv.cfg").exists();
                if !skip && depth < DEPTH {
                    dirs.push((p, depth + 1));
                }
            } else if kind(&p).is_some() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The language a manifest lists packages for, as its language server is named.
fn kind(p: &Path) -> Option<&'static str> {
    let name = p.file_name()?.to_str()?;
    match name {
        "pyproject.toml" | "setup.py" => Some("python"),
        "package.json" => Some("typescript"),
        "pubspec.yaml" => Some("dart"),
        _ if name.starts_with("requirements") && name.ends_with(".txt") => Some("python"),
        _ => None,
    }
}

/// A requirement's name: what it starts with (`pytest>=8` → `pytest`, `uvicorn[standard]` → `uvicorn`).
fn requirement(s: &str) -> Option<&str> {
    let s = s.trim();
    let end = s.find(|c: char| !(c.is_ascii_alphanumeric() || "._-".contains(c))).unwrap_or(s.len());
    (end > 0).then(|| &s[..end])
}

/// The strings written in a line, without their quotes.
fn strings(line: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut rest = line;
    while let Some(start) = rest.find(['"', '\'']) {
        let q = &rest[start..start + 1];
        let Some(len) = rest[start + 1..].find(q) else { break };
        out.push(&rest[start + 1..start + 1 + len]);
        rest = &rest[start + 1 + len + 1..];
    }
    out
}

/// The names a manifest lists as dependencies, as written.
fn names(p: &Path, text: &str) -> Vec<String> {
    let file = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match file {
        "package.json" => {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return vec![] };
            ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]
                .iter()
                .filter_map(|k| v.get(*k)?.as_object())
                .flat_map(|o| o.keys().cloned())
                .collect()
        }
        "pubspec.yaml" => {
            // the keys right under `dependencies:` and `dev_dependencies:`
            let mut out = vec![];
            let mut inside = false;
            for line in text.lines() {
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    continue;
                }
                if !line.starts_with([' ', '\t']) {
                    inside = line.starts_with("dependencies:") || line.starts_with("dev_dependencies:");
                } else if inside && line.starts_with("  ") && !line[2..].starts_with([' ', '\t']) {
                    out.extend(line.trim().split(':').next().map(str::to_string));
                }
            }
            out
        }
        "pyproject.toml" => {
            // requirement strings in a `dependencies` array or a dependency section's arrays, and the keys of
            // Poetry's dependency sections
            let mut out = vec![];
            let (mut section, mut in_array) = (String::new(), false);
            for line in text.lines() {
                let t = line.split(" #").next().unwrap_or("").trim();
                if in_array {
                    out.extend(strings(t).into_iter().filter_map(requirement).map(str::to_string));
                    in_array = !t.contains(']');
                    continue;
                }
                if t.starts_with('[') {
                    section = t.trim_matches(['[', ']']).trim().to_string();
                    continue;
                }
                let Some((key, value)) = t.split_once('=') else { continue };
                let key = key.trim().trim_matches(['"', '\'']);
                if section.starts_with("tool.poetry") && section.ends_with("dependencies") {
                    if key != "python" {
                        out.push(key.to_string());
                    }
                } else if key == "dependencies" || section.ends_with("dependencies") || section == "dependency-groups" {
                    out.extend(strings(value).into_iter().filter_map(requirement).map(str::to_string));
                    in_array = value.contains('[') && !value.contains(']');
                }
            }
            out
        }
        "setup.py" => {
            // the requirement strings of `install_requires=[…]`
            let mut out = vec![];
            let mut rest = text;
            while let Some(at) = rest.find("install_requires") {
                rest = &rest[at..];
                let (Some(open), close) = (rest.find('['), rest.find(']')) else { break };
                let end = close.filter(|c| *c > open).unwrap_or(rest.len());
                out.extend(strings(&rest[open..end]).into_iter().filter_map(requirement).map(str::to_string));
                rest = &rest[end..];
            }
            out
        }
        _ => text
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim())
            .filter(|l| !l.is_empty() && !l.starts_with('-'))
            .filter_map(requirement)
            .map(str::to_string)
            .collect(),
    }
}

/// A Python name as pip compares it.
fn python_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "-_.".contains(c) {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

fn same(lang: &str, listed: &str, package: &str) -> bool {
    if lang == "python" { python_name(listed) == python_name(package) } else { listed == package }
}

/// A path relative to the project, with `/`, quoted when it holds a space.
fn rel(root: &Path, p: &Path) -> String {
    let s = p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    if s.contains(' ') { format!("\"{s}\"") } else { s }
}

/// Whether `pattern` (a workspace's member pattern: `frontend`, `packages/*`, `apps/**`) matches the folder `path`,
/// both relative to the workspace's folder.
fn glob(pattern: &str, path: &str) -> bool {
    fn seg(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => seg(&p[1..], s) || (!s.is_empty() && seg(p, &s[1..])),
            (Some(a), Some(b)) if a == b => seg(&p[1..], &s[1..]),
            _ => false,
        }
    }
    fn all(p: &[&str], s: &[&str]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(&"**"), _) => all(&p[1..], s) || (!s.is_empty() && all(p, &s[1..])),
            (Some(a), Some(b)) => seg(a.as_bytes(), b.as_bytes()) && all(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let split = |x: &str| x.trim_start_matches("./").trim_end_matches('/').split('/').filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let (p, s) = (split(pattern), split(path));
    all(&p.iter().map(String::as_str).collect::<Vec<_>>(), &s.iter().map(String::as_str).collect::<Vec<_>>())
}

/// The member patterns a workspace's folder declares for a kind of manifest, and the ones it leaves out: npm, Yarn
/// and Bun in `package.json`, pnpm in `pnpm-workspace.yaml`, uv in `pyproject.toml` (`[tool.uv.workspace]`).
fn workspace(dir: &Path, file: &str) -> (Vec<String>, Vec<String>) {
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap_or_default();
    let (mut members, mut excluded) = (vec![], vec![]);
    match file {
        "package.json" => {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&read("package.json")) {
                let w = v.get("workspaces");
                let list = w.and_then(|w| w.as_array().or_else(|| w.get("packages")?.as_array()));
                members.extend(list.into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)));
            }
            let mut inside = false;
            for line in read("pnpm-workspace.yaml").lines() {
                if !line.starts_with([' ', '\t', '-']) {
                    inside = line.trim_end() == "packages:";
                } else if inside && let Some(item) = line.trim().strip_prefix('-') {
                    let item = item.trim().trim_matches(['"', '\'']);
                    match item.strip_prefix('!') {
                        Some(x) => excluded.push(x.to_string()),
                        None => members.push(item.to_string()),
                    }
                }
            }
        }
        "pyproject.toml" => {
            let (mut section, mut key) = (String::new(), String::new());
            for line in read("pyproject.toml").lines() {
                let t = line.trim();
                if t.starts_with('[') && key.is_empty() {
                    section = t.trim_matches(['[', ']']).trim().to_string();
                    continue;
                }
                if section != "tool.uv.workspace" {
                    continue;
                }
                let value = match t.split_once('=') {
                    Some((k, v)) if key.is_empty() => {
                        key = k.trim().to_string();
                        v
                    }
                    _ => t,
                };
                let list = if key == "members" { &mut members } else if key == "exclude" { &mut excluded } else { &mut vec![] };
                list.extend(strings(value).into_iter().map(str::to_string));
                if !value.contains('[') || value.contains(']') {
                    key.clear();
                }
            }
        }
        _ => {}
    }
    (members, excluded)
}

/// The folder whose lockfile installs what `manifest` lists, with the lockfile's name: the manifest's own folder,
/// else the nearest folder above it, inside the project, whose workspace has the manifest's folder as a member.
fn locked<'a>(root: &Path, manifest: &Path, locks: &[&'a str]) -> Option<(PathBuf, &'a str)> {
    let file = manifest.file_name()?.to_str()?;
    let dir = manifest.parent()?;
    let mut at = Some(dir);
    while let Some(d) = at.filter(|d| d.starts_with(root)) {
        if let Some(lock) = locks.iter().find(|l| d.join(l).exists()) {
            if d == dir {
                return Some((d.to_path_buf(), lock));
            }
            let (members, excluded) = workspace(d, file);
            let path = dir.strip_prefix(d).ok()?.to_string_lossy().replace('\\', "/");
            let member = members.iter().any(|m| glob(m, &path)) && !excluded.iter().any(|m| glob(m, &path));
            return member.then(|| (d.to_path_buf(), *lock));
        }
        at = d.parent();
    }
    None
}

/// The command the project's own tools would install what `manifest` lists with, run from the project's folder,
/// where the files show the tool. `env`: the project's Python environment, when it is inside the project.
fn install(root: &Path, manifest: &Path, env: Option<&Path>) -> Option<String> {
    let cd = |d: &Path| if d == root { String::new() } else { format!("cd {} && ", rel(root, d)) };
    let file = manifest.file_name()?.to_str()?;
    match file {
        "package.json" => {
            let (dir, lock) = locked(root, manifest, &["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb"])?;
            let tool = match lock {
                "package-lock.json" => "npm",
                "pnpm-lock.yaml" => "pnpm",
                "yarn.lock" => "yarn",
                _ => "bun",
            };
            Some(format!("{}{tool} install", cd(&dir)))
        }
        "pubspec.yaml" => {
            let text = std::fs::read_to_string(manifest).ok()?;
            let flutter = text.lines().any(|l| l.trim() == "sdk: flutter");
            Some(format!("{}{} pub get", cd(manifest.parent()?), if flutter { "flutter" } else { "dart" }))
        }
        "pyproject.toml" => {
            // every extra and group too: a package may be listed only there (`pytest` in a `dev` group)
            let (dir, lock) = locked(root, manifest, &["uv.lock", "poetry.lock"])?;
            let tool = if lock == "uv.lock" { "uv sync --all-packages --all-extras --all-groups" } else { "poetry install --all-extras" };
            Some(format!("{}{tool}", cd(&dir)))
        }
        // installing what `setup.py` lists runs it, and it is the project's own code: no command
        "setup.py" => None,
        _ => {
            // a requirements file goes into the project's environment, by the tool that made it
            let env = env?;
            let cfg = std::fs::read_to_string(env.join("pyvenv.cfg")).ok()?;
            if cfg.lines().any(|l| l.trim_start().starts_with("uv =")) {
                return Some(format!("uv pip install --python {} -r {}", rel(root, env), rel(root, manifest)));
            }
            let python = ["bin/python", "Scripts/python.exe"].into_iter().map(|p| env.join(p)).find(|p| p.exists())?;
            let pip = ["bin/pip", "Scripts/pip.exe"].iter().any(|p| env.join(p).exists());
            pip.then(|| format!("{} -m pip install -r {}", rel(root, &python), rel(root, manifest)))
        }
    }
}

/// Each package not installed (with the language server of the files that import it): where it is listed and how
/// the project's tools would install it. `python`: the interpreter of the project's Python environment, if any.
pub fn not_installed(root: &Path, missing: &[(String, &str)], python: Option<&Path>) -> Vec<NotInstalled> {
    if missing.is_empty() {
        return vec![];
    }
    let files: Vec<(PathBuf, &str, Vec<String>)> = manifests(root)
        .into_iter()
        .filter_map(|p| {
            let k = kind(&p)?;
            let text = std::fs::read_to_string(&p).ok()?;
            let n = names(&p, &text);
            Some((p, k, n))
        })
        .collect();
    // the environment's folder (`.venv/bin/python` → `.venv`), when it is the project's own
    let env = python.and_then(|p| p.parent()?.parent()).filter(|e| e.join("pyvenv.cfg").exists() && e.starts_with(root));
    missing
        .iter()
        .map(|(package, lang)| {
            let listed: Vec<&PathBuf> = files.iter().filter(|(_, k, n)| k == lang && n.iter().any(|x| same(lang, x, package))).map(|(p, _, _)| p).collect();
            let mut commands: Vec<String> = vec![];
            for m in &listed {
                if let Some(c) = install(root, m, env).filter(|c| !commands.contains(c)) {
                    commands.push(c);
                }
            }
            NotInstalled {
                package: package.clone(),
                language: lang.to_string(),
                listed: listed.iter().map(|p| rel(root, p)).collect(),
                env: if *lang == "python" { env.map(|e| rel(root, e)) } else { None },
                install: commands,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{glob, names, python_name};

    #[test]
    fn workspace_patterns_match_member_folders() {
        assert!(glob("frontend", "frontend"));
        assert!(glob("./packages/*", "packages/ui"));
        assert!(!glob("packages/*", "packages/ui/inner"));
        assert!(glob("apps/**", "apps/web/inner"));
        assert!(glob("pkg-*", "pkg-core"));
        assert!(!glob("backend", "examples/app"));
    }
    use std::path::Path;

    #[test]
    fn manifests_list_their_dependencies() {
        let req = "# tools\npytest>=8.0\nuvicorn[standard]==0.30\n-r base.txt\nsome_pkg ; python_version < '3.12'\n";
        assert_eq!(names(Path::new("requirements-test.txt"), req), ["pytest", "uvicorn", "some_pkg"]);
        let toml = "[project]\nname = \"app\"\ndependencies = [\n  \"fastapi>=0.1\",\n  'httpx',\n]\n\n[project.optional-dependencies]\ndev = [\"pytest\"]\n\n[tool.poetry.group.dev.dependencies]\npython = \"^3.11\"\nruff = \"*\"\n";
        assert_eq!(names(Path::new("pyproject.toml"), toml), ["fastapi", "httpx", "pytest", "ruff"]);
        let setup = "setup(\n    name='app',\n    install_requires=[\n        \"aiohttp\",\n        'fake-useragent>=1',\n    ],\n)\n";
        assert_eq!(names(Path::new("setup.py"), setup), ["aiohttp", "fake-useragent"]);
        let json = r#"{ "name": "app", "dependencies": { "react": "18" }, "devDependencies": { "@scope/tool": "1" } }"#;
        assert_eq!(names(Path::new("package.json"), json), ["react", "@scope/tool"]);
        let yaml = "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n  http: ^1.2.0\n\ndev_dependencies:\n  test: any\nflutter:\n  uses-material-design: true\n";
        assert_eq!(names(Path::new("pubspec.yaml"), yaml), ["flutter", "http", "test"]);
    }

    #[test]
    fn python_names_compare_as_pip_compares_them() {
        assert_eq!(python_name("Some_Pkg.Name"), python_name("some-pkg-name"));
        assert_ne!(python_name("python-jose"), python_name("jose"));
    }
}
