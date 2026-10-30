// Which language server answers for which language, and how each one is started.
// The core only needs "go to definition"; everything here is about getting a server into a state where its
// answers are the strong ones (the full project loaded, the project's own environment applied).
use crate::lsp::Client;
use crate::source::Project;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Default, Clone)]
pub struct Setup {
    /// a folder holding the server executables (for example a node_modules/.bin); otherwise PATH is searched
    pub servers: Option<PathBuf>,
    /// TypeScript to run when the project has none of its own (a folder with tsserver.js)
    pub tsserver: Option<PathBuf>,
    /// the Python interpreter of the project's environment, when it is not inside the project
    pub python: Option<PathBuf>,
    /// where downloaded language servers are kept; without it nothing can be downloaded
    pub store: Option<PathBuf>,
}

/// The names an executable has on this system (`x`, or `x.cmd` / `x.exe` on Windows).
fn exe_names(exe: &str) -> Vec<String> {
    if cfg!(windows) { vec![format!("{exe}.cmd"), format!("{exe}.exe"), exe.to_string()] } else { vec![exe.to_string()] }
}

fn in_dir(dir: &Path, exe: &str) -> Option<PathBuf> {
    exe_names(exe).into_iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

fn on_path(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).find_map(|d| in_dir(&d, exe)))
}

/// An npm package, pinned by version and by the registry's integrity hash.
pub struct Package {
    pub name: &'static str,
    pub version: &'static str,
    pub integrity: &'static str,
}

/// A language server plainfold can download: npm packages, run with the user's Node.
pub struct Recipe {
    pub kind: &'static str,
    pub label: &'static str,
    pub packages: &'static [Package],
    /// the server's script, under node_modules
    pub entry: &'static str,
    /// the oldest Node it runs on
    pub node: (u32, u32),
    /// megabytes, roughly: the tarballs, and unpacked
    pub download_mb: u32,
    pub disk_mb: u32,
}

pub const RECIPES: &[Recipe] = &[
    Recipe {
        kind: "typescript",
        label: "TypeScript language server",
        packages: &[
            Package {
                name: "typescript-language-server",
                version: "6.0.1",
                integrity: "sha512-c5hEHM/7rdFRbQWoHXuddyJ53oF0M2dKeXQOYdwUek556oarltKeumKJGLp/ZMcGV7gW30mGn1MR3V5GDiLiKg==",
            },
            // the TypeScript it runs when the project has none of its own
            Package {
                name: "typescript",
                version: "5.9.3",
                integrity: "sha512-jl1vZzPDinLr9eUt3J/t7V6FgNEw9QjvBPdysz9KfQDD41fQrC2Y4vKQdiaUpFT4bXlb1RHhLpp8wtm6M5TgSw==",
            },
        ],
        entry: "typescript-language-server/lib/cli.mjs",
        node: (22, 22),
        download_mb: 5,
        disk_mb: 25,
    },
    Recipe {
        kind: "python",
        label: "Python language server (pyright)",
        packages: &[Package {
            name: "pyright",
            version: "1.1.414",
            integrity: "sha512-FPZZb51jepDX4eP7TEYDeNFtmE3WgwkkEcJpvH3/QmUSsj0EAy3LXu+xB4T/FWDejtsXlCfFr/rbWFjwuwuXww==",
        }],
        entry: "pyright/langserver.index.js",
        node: (14, 0),
        download_mb: 4,
        disk_mb: 34,
    },
];

pub fn recipe(kind: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.kind == kind)
}

/// What a language's server is called, for people.
pub fn label(kind: &str) -> String {
    match (recipe(kind), kind) {
        (Some(r), _) => r.label.to_string(),
        (None, "dart") => "Dart analysis server (from the Dart SDK)".into(),
        (None, _) => format!("{kind} language server"),
    }
}

impl Recipe {
    /// The folder this exact set of versions is unpacked into.
    pub fn dir(&self, store: &Path) -> PathBuf {
        store.join(self.kind).join(self.packages.iter().map(|p| format!("{}@{}", p.name, p.version)).collect::<Vec<_>>().join("+"))
    }
}

/// A program plainfold can download, built separately for each system: one file per platform, pinned by its SHA-256.
pub struct Program {
    pub name: &'static str,
    pub label: &'static str,
    pub version: &'static str,
    /// where the pinned files come from, in words
    pub from: &'static str,
    pub builds: &'static [Build],
}

pub struct Build {
    /// as Rust names them (`std::env::consts`)
    pub os: &'static str,
    pub arch: &'static str,
    pub url: &'static str,
    /// of the file at `url`
    pub sha256: &'static str,
    /// the program's path inside the `.tar.gz` at `url`; without it, the file is the program
    pub inside: Option<&'static str>,
    /// megabytes, roughly: the file at `url`, and the program
    pub download_mb: u32,
    pub disk_mb: u32,
}

/// The Node.js that runs a downloaded language server when this machine has none new enough: the current long-term
/// support release, from nodejs.org's `SHASUMS256.txt` for it. Only the `node` program is kept.
pub const NODE: Program = Program {
    name: "node",
    label: "Node.js",
    version: "24.21.0",
    from: "nodejs.org",
    builds: &[
        Build {
            os: "macos",
            arch: "aarch64",
            url: "https://nodejs.org/dist/v24.21.0/node-v24.21.0-darwin-arm64.tar.gz",
            sha256: "bed7eea5325e1108f32ce5228ddd6a5f0f08a499ee42aa7442aea583702f6057",
            inside: Some("node-v24.21.0-darwin-arm64/bin/node"),
            download_mb: 53,
            disk_mb: 122,
        },
        Build {
            os: "macos",
            arch: "x86_64",
            url: "https://nodejs.org/dist/v24.21.0/node-v24.21.0-darwin-x64.tar.gz",
            sha256: "1462cb3b3046b815cf8ea436d3da450ec1a9f11dac7e5a46b0ada5305d7e8097",
            inside: Some("node-v24.21.0-darwin-x64/bin/node"),
            download_mb: 54,
            disk_mb: 125,
        },
        Build {
            os: "linux",
            arch: "x86_64",
            url: "https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-x64.tar.gz",
            sha256: "6e1db87ef58b8819e5d5402eff1536491b18edd8eb7bee5ef7897876e88dc5ff",
            inside: Some("node-v24.21.0-linux-x64/bin/node"),
            download_mb: 58,
            disk_mb: 127,
        },
        Build {
            os: "linux",
            arch: "aarch64",
            url: "https://nodejs.org/dist/v24.21.0/node-v24.21.0-linux-arm64.tar.gz",
            sha256: "724282c3b43aec998aa9527380465b45d229e021b58035f5f4f63095eabfe5d5",
            inside: Some("node-v24.21.0-linux-arm64/bin/node"),
            download_mb: 58,
            disk_mb: 123,
        },
        Build {
            os: "windows",
            arch: "x86_64",
            url: "https://nodejs.org/dist/v24.21.0/win-x64/node.exe",
            sha256: "ba4e6d110e8c1592a1ecd390f6b05f3da124b13871a5be62b341a07a853c6c32",
            inside: None,
            download_mb: 94,
            disk_mb: 94,
        },
        Build {
            os: "windows",
            arch: "aarch64",
            url: "https://nodejs.org/dist/v24.21.0/win-arm64/node.exe",
            sha256: "dff59da18b6ffe1bf1ca99e1d2af4906080c481740619f5b5098c0fca28bd9b7",
            inside: None,
            download_mb: 82,
            disk_mb: 82,
        },
    ],
};

impl Program {
    /// The build for the system plainfold runs on.
    pub fn build(&self) -> Option<&'static Build> {
        self.builds.iter().find(|b| b.os == std::env::consts::OS && b.arch == std::env::consts::ARCH)
    }

    /// Where this version is kept once downloaded.
    pub fn exe(&self, store: &Path) -> PathBuf {
        let file = if cfg!(windows) { format!("{}.exe", self.name) } else { self.name.to_string() };
        store.join(self.name).join(format!("{}@{}", self.name, self.version)).join(file)
    }
}

/// "v22.22.1" → (22, 22, 1)
fn version(v: &str) -> (u32, u32, u32) {
    let mut n = v.trim().trim_start_matches('v').split('.').map(|x| x.parse::<u32>().unwrap_or(0));
    (n.next().unwrap_or(0), n.next().unwrap_or(0), n.next().unwrap_or(0))
}

/// How a language server will be started.
pub struct Located {
    pub cmd: PathBuf,
    pub args: Vec<String>,
    /// where it came from, for the record
    pub how: String,
    /// the store folder, when plainfold downloaded it
    pub dir: Option<PathBuf>,
}

/// What stands between a project and one of its language servers.
pub enum Missing {
    /// plainfold can download what is missing; the user decides
    Download(Vec<Get>),
    /// it cannot run here, and why
    Unavailable(String),
}

/// One thing a language server still needs from a download.
pub enum Get {
    Server(&'static Recipe),
    /// Node.js, because this machine has none that runs the server: why, in words
    Node(String),
}

impl Missing {
    pub fn reason(&self) -> String {
        match self {
            Missing::Download(get) => get
                .iter()
                .map(|g| match g {
                    Get::Server(r) => format!("the {} is not downloaded", r.label),
                    Get::Node(why) => format!("{why}, and Node.js {} is not downloaded", NODE.version),
                })
                .collect::<Vec<_>>()
                .join("; "),
            Missing::Unavailable(why) => why.clone(),
        }
    }
}

/// The Node on PATH and its version.
pub fn node() -> Option<(PathBuf, (u32, u32, u32))> {
    let exe = on_path("node")?;
    let out = std::process::Command::new(&exe).arg("--version").output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout);
    v.trim().starts_with('v').then(|| (exe, version(&v)))
}

/// The Node that runs `r`, and how to say which one it is: the user's own when it is new enough, else the one
/// plainfold downloaded. Without either, why the user's own does not do.
fn node_for(r: &Recipe, store: &Path) -> Result<(PathBuf, String), String> {
    let mine = node();
    if let Some((exe, v)) = &mine
        && (v.0, v.1) >= r.node
    {
        return Ok((exe.clone(), format!("Node.js {}.{}.{} ({})", v.0, v.1, v.2, exe.display())));
    }
    let downloaded = NODE.exe(store);
    if downloaded.is_file() {
        return Ok((downloaded, format!("Node.js {} downloaded by plainfold", NODE.version)));
    }
    let has = match mine {
        Some((_, v)) => format!("this machine has Node.js {}.{}.{}", v.0, v.1, v.2),
        None => "this machine has no Node.js".into(),
    };
    Err(format!("{has}; the {} needs {}.{} or newer", r.label, r.node.0, r.node.1))
}

/// Finds the language server for `kind`: in a folder given for development, on PATH, or downloaded by plainfold earlier.
/// A server inside the project's own folder is not used: running it would run the project's code before it is trusted.
pub fn locate(kind: &str, setup: &Setup) -> Result<Located, Missing> {
    let exe = match kind {
        "typescript" => "typescript-language-server",
        "python" => "pyright-langserver",
        "dart" => return dart(setup),
        _ => return Err(Missing::Unavailable(format!("plainfold has no language server for {kind} yet"))),
    };
    let found = |cmd: PathBuf, how: String| Ok(Located { cmd, args: vec!["--stdio".into()], how, dir: None });
    if let Some(p) = setup.servers.as_deref().and_then(|d| in_dir(d, exe)) {
        return found(p.clone(), p.display().to_string());
    }
    if let Some(p) = on_path(exe) {
        return found(p.clone(), p.display().to_string());
    }
    let (Some(r), Some(store)) = (recipe(kind), &setup.store) else {
        return Err(Missing::Unavailable(format!("{exe} was not found")));
    };
    let mut get = vec![];
    let dir = r.dir(store);
    let script = dir.join("node_modules").join(r.entry);
    if !script.is_file() {
        get.push(Get::Server(r));
    }
    let node = match node_for(r, store) {
        Ok(node) => Some(node),
        Err(why) if NODE.build().is_none() => return Err(Missing::Unavailable(format!("{why}, and plainfold has no Node.js to download for this system"))),
        Err(why) => {
            get.push(Get::Node(why));
            None
        }
    };
    match node {
        Some((node, with)) if get.is_empty() => Ok(Located {
            cmd: node,
            args: vec![script.to_string_lossy().to_string(), "--stdio".into()],
            how: format!("downloaded by plainfold, run with {with}"),
            dir: Some(dir),
        }),
        _ => Err(Missing::Download(get)),
    }
}

/// Dart's analysis server comes with the Dart SDK, which Flutter brings: a heavy runtime that is the user's own, so it
/// is found, never downloaded.
fn dart(setup: &Setup) -> Result<Located, Missing> {
    let exe = setup.servers.as_deref().and_then(|d| in_dir(d, "dart")).or_else(|| on_path("dart"));
    let Some(exe) = exe else {
        return Err(Missing::Unavailable("the Dart SDK was not found (no `dart` on PATH); it comes with Flutter or Dart".into()));
    };
    let args = ["language-server", "--protocol=lsp", "--client-id=plainfold", &format!("--client-version={}", env!("CARGO_PKG_VERSION"))];
    Ok(Located { how: exe.display().to_string(), cmd: exe, args: args.iter().map(|a| a.to_string()).collect(), dir: None })
}

/// A Python environment that belongs to the project: a folder with `pyvenv.cfg`, at most two levels down.
pub fn project_python(root: &Path) -> Option<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    for depth in 0..2 {
        let mut next = vec![];
        for d in &dirs {
            for e in std::fs::read_dir(d).ok()?.flatten() {
                let p = e.path();
                if !p.is_dir() || p.file_name().is_some_and(|n| n == "node_modules" || n == ".git") {
                    continue;
                }
                if p.join("pyvenv.cfg").exists() {
                    for exe in ["bin/python", "Scripts/python.exe"] {
                        if p.join(exe).exists() {
                            return Some(p.join(exe));
                        }
                    }
                }
                if depth == 0 {
                    next.push(p);
                }
            }
        }
        dirs = next;
    }
    None
}

pub struct Started {
    pub client: Client,
    /// what this server was started with, for the record ("project TypeScript", "environment: …")
    pub with: String,
}

pub fn start(kind: &str, project: &Project, setup: &Setup) -> Result<Started, String> {
    match kind {
        "typescript" => {
            let at = locate(kind, setup).map_err(|m| m.reason())?;
            let own = project.root.join("node_modules/typescript/lib");
            let (tsserver, with) = if own.join("tsserver.js").exists() {
                (Some(own), "the project's TypeScript")
            } else if let Some(p) = setup.tsserver.clone() {
                (Some(p), "a TypeScript given for development (the project has none installed)")
            } else if let Some(d) = &at.dir {
                (Some(d.join("node_modules/typescript/lib")), "the TypeScript plainfold downloaded (the project has none installed)")
            } else {
                (None, "the TypeScript the server finds itself (the project has none installed)")
            };
            let with = format!("{with}; server: {}", at.how);
            let mut ts = json!({ "useSyntaxServer": "never" });
            if let Some(p) = tsserver {
                ts["path"] = json!(p);
            }
            let args: Vec<&str> = at.args.iter().map(String::as_str).collect();
            let client = Client::start("typescript-language-server", &at.cmd, &args, &project.root, json!({})).map_err(|e| e.to_string())?;
            // The syntax-only helper answers before the project has loaded, and its "definition" stops at the import.
            client.initialize(&project.root, json!({ "tsserver": ts }))?;
            Ok(Started { client, with })
        }
        "python" => {
            let at = locate(kind, setup).map_err(|m| m.reason())?;
            let python = setup.python.clone().or_else(|| project_python(&project.root));
            let mut settings = json!({ "python.analysis": { "diagnosticMode": "openFilesOnly" } });
            let with = match &python {
                Some(p) => {
                    settings["python"] = json!({ "pythonPath": p });
                    format!("environment {}; server: {}", p.display(), at.how)
                }
                None => format!("no project environment found (the default Python); server: {}", at.how),
            };
            let args: Vec<&str> = at.args.iter().map(String::as_str).collect();
            let client = Client::start("pyright", &at.cmd, &args, &project.root, settings).map_err(|e| e.to_string())?;
            client.initialize(&project.root, Value::Null)?;
            Ok(Started { client, with })
        }
        "dart" => {
            let at = locate(kind, setup).map_err(|m| m.reason())?;
            let args: Vec<&str> = at.args.iter().map(String::as_str).collect();
            let client = Client::start("dart", &at.cmd, &args, &project.root, json!({})).map_err(|e| e.to_string())?;
            // every package under the folder, not only those with a file open; none of the editor's extras
            let init = json!({ "onlyAnalyzeProjectsWithOpenFiles": false, "suggestFromUnimportedLibraries": false, "closingLabels": false, "outline": false, "flutterOutline": false });
            client.initialize(&project.root, init)?;
            Ok(Started { client, with: format!("server: {}", at.how) })
        }
        _ => Err(format!("no language server for {kind}")),
    }
}

/// What a server reads when it starts besides the project's source files, as one fingerprint: package manifests,
/// lock files, compiler and checker settings, and which packages are installed. A server does not notice these
/// changing by itself, so one kept running from an earlier read is used again only while this stays the same.
pub fn settings(kind: &str, root: &Path, setup: &Setup) -> String {
    let named: fn(&str) -> bool = match kind {
        "typescript" => |n| {
            matches!(n, "package.json" | "jsconfig.json" | "package-lock.json" | "npm-shrinkwrap.json" | "yarn.lock" | "pnpm-lock.yaml" | "bun.lock" | "bun.lockb")
                || (n.starts_with("tsconfig") && n.ends_with(".json"))
        },
        "python" => |n| {
            matches!(n, "pyproject.toml" | "setup.cfg" | "setup.py" | "pyrightconfig.json" | "Pipfile" | "Pipfile.lock" | "poetry.lock" | "uv.lock")
                || (n.starts_with("requirements") && n.ends_with(".txt"))
        },
        "dart" => |n| matches!(n, "pubspec.yaml" | "pubspec.lock" | "analysis_options.yaml"),
        _ => |_| false,
    };
    let mut lines: Vec<String> = vec![];
    let mut seen = |p: &Path, what: String| lines.push(format!("{} {what}", crate::source::rel_path(root, p)));
    let digest = |p: &Path| std::fs::read(p).map(|b| crate::changes::hash(&b).iter().map(|x| format!("{x:02x}")).collect::<String>()).unwrap_or_default();
    let mut dirs = vec![root.to_path_buf()];
    for e in ignore::WalkBuilder::new(root).hidden(true).filter_entry(|e| !crate::source::NOT_OWN.contains(&e.file_name().to_string_lossy().as_ref())).build().flatten() {
        let p = e.path();
        if e.file_type().is_some_and(|t| t.is_dir()) {
            dirs.push(p.to_path_buf());
        } else if p.file_name().is_some_and(|n| named(&n.to_string_lossy())) {
            seen(p, digest(p));
        }
    }
    // which packages are installed: what the package managers write when they install, and the environment's packages
    for d in &dirs {
        if kind == "typescript" {
            for marker in [".package-lock.json", ".modules.yaml", ".yarn-state.yml", ".yarn-integrity"] {
                let m = d.join("node_modules").join(marker);
                if m.is_file() {
                    seen(&m, digest(&m));
                }
            }
        }
        // what `pub get` wrote: where each package is
        if kind == "dart" {
            let m = d.join(".dart_tool/package_config.json");
            if m.is_file() {
                seen(&m, digest(&m));
            }
        }
    }
    if kind == "python" {
        let python = setup.python.clone().or_else(|| project_python(root));
        // <environment>/bin/python → <environment>/lib/python3.x/site-packages
        if let Some(env) = python.as_ref().and_then(|p| p.parent()).and_then(|b| b.parent()) {
            for lib in ["lib", "Lib"] {
                for v in std::fs::read_dir(env.join(lib)).into_iter().flatten().flatten() {
                    let sp = if lib == "Lib" { env.join(lib).join("site-packages") } else { v.path().join("site-packages") };
                    let mut names: Vec<String> = std::fs::read_dir(&sp).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
                    names.sort();
                    if !names.is_empty() {
                        seen(&sp, crate::changes::hash(names.join("\n").as_bytes()).iter().map(|x| format!("{x:02x}")).collect());
                    }
                }
            }
        }
    }
    lines.sort();
    lines.dedup();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_downloadable_node_runs_every_server() {
        let v = version(NODE.version);
        for r in RECIPES {
            assert!((v.0, v.1) >= r.node, "{} needs Node {}.{}", r.label, r.node.0, r.node.1);
        }
    }

    #[test]
    fn every_node_build_is_of_the_pinned_version() {
        for b in NODE.builds {
            assert!(b.url.contains(&format!("/v{}/", NODE.version)), "{}", b.url);
            assert!(b.inside.is_none_or(|p| p.contains(&format!("-v{}-", NODE.version))), "{}", b.url);
            assert_eq!(b.sha256.len(), 64);
        }
    }
}
