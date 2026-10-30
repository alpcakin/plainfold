// plainfold core as a library: the desktop app and the command line both call `needs`, `install` and `extract`.
pub mod adapters;
pub mod changes;
pub mod deps;
pub mod diff;
pub mod english;
pub mod fetch;
pub mod keep;
pub mod lang;
pub mod lsp;
pub mod model;
pub mod renames;
pub mod resolve;
pub mod servers;
pub mod source;
pub mod story;

use serde::Serialize;
use std::path::Path;
use std::time::Instant;

/// What the extraction is doing right now, for a view that shows progress while a project is read.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "step", rename_all = "kebab-case")]
pub enum Progress {
    /// the project's files are read and parsed
    Loaded { name: String, files: usize },
    /// the framework adapters whose conventions the project uses
    Adapters { names: Vec<String> },
    /// a language server is starting and the core waits for its answers to settle
    ServerStarting { kind: String },
    /// still waiting, about once a second: how long so far, and what the server says it is doing
    ServerWaiting { kind: String, seconds: u64, doing: String },
    ServerReady { kind: String, name: String, readiness: String },
    /// a server kept running from an earlier read is used again: `changed` source files are sent to it first
    ServerKept { kind: String, name: String, changed: usize },
    /// a language has no server; nothing in that language is shown
    ServerFailed { kind: String, why: String },
    /// a language server is being downloaded
    Downloading { kind: String, label: String, done: u64, total: u64 },
    /// stories are being built
    Building,
}

/// A language the project uses, and whether its server can run.
#[derive(Serialize, Clone, Debug)]
pub struct Need {
    pub kind: String,
    /// "ready" | "download" | "unavailable"
    pub state: &'static str,
    /// where the server is, what would be downloaded, or why it cannot run
    pub detail: String,
    pub label: String,
    /// for a download, what it brings; two languages may need the same part (Node.js)
    pub parts: Vec<Part>,
}

/// Something a download brings.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub label: String,
    /// exactly what is downloaded
    pub detail: String,
    /// why it is needed, when that is not plain from the label
    pub why: Option<String>,
    /// where it is downloaded from
    pub from: &'static str,
    /// megabytes, roughly: downloaded, and kept on disk
    pub download_mb: u32,
    pub disk_mb: u32,
}

fn part(get: &servers::Get) -> Part {
    match get {
        servers::Get::Server(r) => Part {
            label: r.label.to_string(),
            detail: r.packages.iter().map(|p| format!("{} {}", p.name, p.version)).collect::<Vec<_>>().join(", "),
            why: None,
            from: "the npm registry",
            download_mb: r.download_mb,
            disk_mb: r.disk_mb,
        },
        servers::Get::Node(why) => {
            let b = servers::NODE.build();
            Part {
                label: format!("{} {}", servers::NODE.label, servers::NODE.version),
                detail: b.map(|b| b.url.rsplit('/').next().unwrap_or(b.url).to_string()).unwrap_or_default(),
                why: Some(format!("It runs the language server: {why}.")),
                from: servers::NODE.from,
                download_mb: b.map_or(0, |b| b.download_mb),
                disk_mb: b.map_or(0, |b| b.disk_mb),
            }
        }
    }
}

/// The languages a project uses and the state of each one's server. Reads file names only.
pub fn needs(root: &Path, setup: &servers::Setup) -> Vec<Need> {
    source::server_kinds(root)
        .into_iter()
        .map(|kind| {
            let label = servers::label(kind);
            let (state, detail, parts) = match servers::locate(kind, setup) {
                Ok(at) => ("ready", at.how, vec![]),
                Err(servers::Missing::Download(get)) => {
                    let parts: Vec<Part> = get.iter().map(part).collect();
                    let what = get.iter().zip(&parts).map(|(g, p)| if matches!(g, servers::Get::Node(_)) { p.label.clone() } else { p.detail.clone() });
                    ("download", what.collect::<Vec<_>>().join(", "), parts)
                }
                Err(servers::Missing::Unavailable(why)) => ("unavailable", why, vec![]),
            };
            Need { kind: kind.to_string(), state, detail, label, parts }
        })
        .collect()
}

/// Downloads what the server for `kind` still needs into the store: the server, and Node.js when this machine has none
/// that runs it.
pub fn install(kind: &str, setup: &servers::Setup, on: &dyn Fn(Progress)) -> Result<(), String> {
    let get = match servers::locate(kind, setup) {
        Ok(_) => return Ok(()),
        Err(servers::Missing::Download(get)) => get,
        Err(servers::Missing::Unavailable(why)) => return Err(why),
    };
    let store = setup.store.as_deref().ok_or("no folder to keep downloaded language servers in")?;
    let label = |what: String| move |done, total| on(Progress::Downloading { kind: kind.to_string(), label: what.clone(), done, total });
    for g in &get {
        match g {
            servers::Get::Node(_) => {
                fetch::install_program(&servers::NODE, store, &label(format!("{} {}", servers::NODE.label, servers::NODE.version)))?;
            }
            servers::Get::Server(r) => {
                fetch::install(r, store, &label(r.label.to_string()))?;
            }
        }
    }
    // a new version is in place: the one it replaces is no longer run
    fetch::prune(store);
    Ok(())
}

/// Removes the downloaded servers and Node.js versions this version of plainfold no longer runs (`fetch::prune`).
pub fn prune(setup: &servers::Setup) -> Vec<std::path::PathBuf> {
    setup.store.as_deref().map(fetch::prune).unwrap_or_default()
}

/// A language that had no server, so nothing in it is shown.
#[derive(Serialize, Clone, Debug)]
pub struct NoServer {
    pub kind: String,
    pub why: String,
}

/// A language server as it was when the stories were built (always ready: an unready one is never used).
#[derive(Serialize, Clone, Debug)]
pub struct ServerState {
    pub kind: String,
    pub name: String,
    pub with: String,
    pub readiness: String,
}

pub struct Extracted {
    pub model: model::ProjectModel,
    pub adapters: Vec<String>,
    pub servers: Vec<ServerState>,
    /// languages whose server could not start; their stories are left out
    pub failed: Vec<NoServer>,
    /// imported modules that are not installed, so calls into them are shown by name only
    pub missing: Vec<String>,
    /// time until every server was ready, from the start
    pub ready_ms: u128,
    /// the files the stories were built from, as they were read: what `changes::watch` compares with
    pub hashes: changes::Hashes,
}

/// Reads a project and builds all of its stories. `on` hears each step as it starts. `stop(kind)` says the user
/// stopped waiting for that language's server; its stories are then left out like those of a missing server.
/// The language servers are stopped at the end.
pub fn extract(root: &Path, setup: &servers::Setup, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool) -> std::io::Result<Extracted> {
    read(root, setup, on, stop, None).map(|(x, _servers_stopped_here)| x)
}

/// The same, using the language servers `kept` running from an earlier read of this project where they can still be
/// used (`keep.rs`), and handing back the servers it used, still running. Servers kept for another project are
/// stopped first.
pub fn read(root: &Path, setup: &servers::Setup, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool, kept: Option<keep::Kept>) -> std::io::Result<(Extracted, keep::Kept)> {
    let t0 = Instant::now();
    let project = source::Project::load(root)?;
    on(Progress::Loaded { name: project.name.clone(), files: project.files.len() });
    let adapters = adapters::Adapters::for_project(&project);
    on(Progress::Adapters { names: adapters.names().iter().map(|n| n.to_string()).collect() });
    let kept = match kept {
        Some(k) if k.root == project.root => k.take(),
        _ => Default::default(),
    };
    let res = resolve::Resolver::start_kept(&project, setup, on, stop, kept);
    let ready_ms = t0.elapsed().as_millis();
    // the packages that are not installed: where each is listed, said in the stories that reach it
    let modules = res.missing_modules();
    let python = setup.python.clone().or_else(|| servers::project_python(&project.root));
    let not_installed = deps::not_installed(&project.root, &modules, python.as_deref());
    let missing: Vec<String> = modules.into_iter().map(|(p, _)| p).collect();
    on(Progress::Building);
    let world = story::World::new(&project, &res, &adapters);
    let mut model = story::discover(&world, not_installed);
    // A server that stopped while the stories were built left its answers incomplete, so its language counts as
    // having no server.
    let ended = res.ended();
    let mut failed = res.failed.clone();
    failed.extend(ended.iter().map(|(kind, why)| (kind.clone(), format!("{why} (while reading)"))));
    // Without its language server a language gives only guesses, so its stories are not shown at all.
    let kind_of = |file: &str| project.files.iter().find(|f| f.rel == file).map(|f| f.lang.server);
    // (How many there would be is not said either: counting them without the server would be a guess too.)
    for (kind, why) in &failed {
        model.stories.retain(|s| kind_of(&s.trigger.at.file) != Some(kind.as_str()));
        model.setup.push(format!("no {kind} language server ({why}): stories that start in {kind} code are not shown"));
    }
    model.stats.ms = t0.elapsed().as_millis();
    let mut servers: Vec<ServerState> = res
        .servers
        .iter()
        .filter(|(k, _)| !ended.iter().any(|(e, _)| e == *k))
        .map(|(k, s)| ServerState { kind: k.to_string(), name: s.client.name.clone(), with: s.with.clone(), readiness: s.readiness.clone() })
        .collect();
    servers.sort_by(|a, b| a.kind.cmp(&b.kind));
    let failed = failed.into_iter().map(|(kind, why)| NoServer { kind, why }).collect();
    let mut running = res.into_servers();
    running.retain(|kind, s| {
        let alive = !ended.iter().any(|(e, _)| e == kind);
        if !alive {
            s.client.stop();
        }
        alive
    });
    let hashes = project.hashes.clone();
    let x = Extracted { model, adapters: adapters.names().iter().map(|n| n.to_string()).collect(), servers, failed, missing, ready_ms, hashes };
    Ok((x, keep::Kept::new(project.root.clone(), running)))
}
