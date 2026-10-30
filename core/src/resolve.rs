// Which function a call reaches. One "go to definition" per name, through the language server, plus a few hops
// that follow a value to where it was made (a local alias, a key of a returned object, a destructured result).
// The shapes come from tree-sitter through the language tables; nothing here names a language.
use crate::Progress;
use crate::lang::{Expr, Lang};
use crate::lsp::{Client, Loc};
use crate::servers::{self, Setup};
use crate::source::{At, NodeKey, Project, SourceFile, one_line};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tree_sitter::Node;

/// Framework rules that change what a value is (supplied by adapters).
pub trait ValueRules {
    /// `x = wrap(f)` where the value of `x` is `f` itself (a memoizing wrapper, for example).
    fn wrapped<'p>(&self, _file: &'p SourceFile, _call: Node<'p>) -> Option<Node<'p>> {
        None
    }
    /// A callee whose target is found by resolving another expression instead, and how that expression is reached
    /// (`the function useRef was made with`), said in the link.
    fn redirect<'p>(&self, _res: &Resolver<'p>, _file: usize, _callee: Node<'p>) -> Option<(At<'p>, &'static str)> {
        None
    }
    /// The values a call gives, where a framework decides them (a shared value read where it is provided, what a
    /// memoizing call's function returns), and how they are reached, said in the link.
    fn gives<'p>(&self, _res: &Resolver<'p>, _file: usize, _call: Node<'p>) -> Option<(Vec<At<'p>>, &'static str)> {
        None
    }
    /// The values at one place of the list a call gives (`[state, dispatch] = useReducer(reducer, init)`: the function
    /// the dispatcher runs), and how they are reached.
    fn gives_at<'p>(&self, _res: &Resolver<'p>, _file: usize, _call: Node<'p>, _place: usize) -> Option<(Vec<At<'p>>, &'static str)> {
        None
    }
}

/// How long a server must be quiet before it is asked, between two rounds of questions, and before the second.
type Pace = (Duration, Duration, Duration);
const STARTING: Pace = (Duration::from_millis(1500), Duration::from_millis(800), Duration::from_millis(500));
const KEPT: Pace = (Duration::from_millis(300), Duration::from_millis(200), Duration::from_millis(300));

/// A language server that is ready: its answers had settled before anything was asked of it.
pub struct Server {
    pub client: Client,
    pub with: String,
    /// how readiness was decided, for the record
    pub readiness: String,
    /// what it was given of each source file: the version and the content's hash
    files: HashMap<PathBuf, (i32, [u8; 32])>,
    /// what it read at start besides the source files (`servers::settings`)
    settings: String,
    /// when it started, and how many reads it has answered
    pub started: std::time::SystemTime,
    pub reads: usize,
}

pub enum Target<'p> {
    /// a function in the project's own code. `via`: the call of a project function that made it (`save = guard(check,
    /// f)`: the call), where the function is what that call returns; what the call is given is what the function's
    /// free names stand for
    Own { func: At<'p>, hops: Vec<String>, via: Option<At<'p>> },
    /// outside the project's own code (a library or the platform), as the server reported it
    Outside(Loc),
    /// a name given its value by a pattern that is not followed further (`[items, setItems] = useState()`): an
    /// adapter may know what it is
    Declared(At<'p>),
    Unresolved(String),
}

pub struct Resolver<'p> {
    pub project: &'p Project,
    pub servers: HashMap<&'static str, Server>,
    /// languages whose server could not start, and why
    pub failed: Vec<(String, String)>,
    cache: Mutex<HashMap<(usize, usize, usize), Vec<Loc>>>,
    hovers: Mutex<HashMap<(usize, usize, usize), Option<String>>>,
    /// What the names a function is given stand for while it is walked for one call: the function, the parameter's
    /// name, and the value the call gave (its node), innermost last
    bound: Mutex<Vec<(NodeKey, String, NodeKey)>>,
}

/// The name node "go to definition" should be asked about for a callee expression.
pub fn callee_name<'t>(n: Node<'t>, l: &Lang) -> Option<Node<'t>> {
    let n = l.unwrap(n);
    if l.is_name(n.kind()) {
        return Some(n);
    }
    let (_, prop) = l.member_fields(n.kind())?;
    callee_name(n.child_by_field_name(prop)?, l)
}

/// Every call site and element in a file: (callee name node).
pub fn call_names<'t>(f: &'t SourceFile) -> Vec<Node<'t>> {
    let l = f.lang;
    let mut out = vec![];
    let mut stack = vec![f.tree.root_node()];
    while let Some(n) = stack.pop() {
        let field = l.call_field(n.kind()).or_else(|| l.element_field(n.kind()));
        if let Some(name) = field.and_then(|fd| n.child_by_field_name(fd)).and_then(|c| callee_name(c, l)) {
            out.push(name);
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out
}

impl<'p> Resolver<'p> {
    pub fn start(project: &'p Project, setup: &Setup) -> Resolver<'p> {
        Self::start_with(project, setup, &|_| {}, &|_| false)
    }

    /// Starts a server for every language in the project and waits until each one is ready, however long that takes:
    /// a large project can keep a server busy for minutes. `on` hears each step; `stop(kind)` says the user stopped
    /// waiting for that language, which then has no server.
    pub fn start_with(project: &'p Project, setup: &Setup, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool) -> Resolver<'p> {
        Self::start_kept(project, setup, on, stop, HashMap::new())
    }

    /// The same, using the servers `kept` running from an earlier read of this project where they can still be used:
    /// one is brought up to date with the source files that changed since, and asked again until its answers settle.
    /// A server that stopped, or whose project settings changed (`servers::settings`), is replaced by a new one.
    pub fn start_kept(project: &'p Project, setup: &Setup, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool, mut kept: HashMap<&'static str, Server>) -> Resolver<'p> {
        let mut r = Resolver { project, servers: HashMap::new(), failed: vec![], cache: Mutex::new(HashMap::new()), hovers: Mutex::new(HashMap::new()), bound: Mutex::new(vec![]) };
        let kinds: BTreeSet<&'static str> = project.files.iter().map(|f| f.lang.server).collect();
        for kind in kinds {
            let settings = servers::settings(kind, &project.root, setup);
            let reuse = kept.remove(kind).and_then(|s| {
                if s.client.ended().is_none() && s.settings == settings {
                    Some(s)
                } else {
                    s.client.stop();
                    None
                }
            });
            let ready = match reuse {
                Some(s) => r.refresh(kind, s, on, stop),
                None => {
                    on(Progress::ServerStarting { kind: kind.to_string() });
                    servers::start(kind, project, setup).and_then(|s| r.ready_up(kind, s, settings, on, stop))
                }
            };
            match ready {
                Ok(server) => {
                    on(Progress::ServerReady { kind: kind.to_string(), name: server.client.name.clone(), readiness: server.readiness.clone() });
                    r.servers.insert(kind, server);
                }
                Err(why) => {
                    on(Progress::ServerFailed { kind: kind.to_string(), why: why.clone() });
                    r.failed.push((kind.to_string(), why));
                }
            }
        }
        // kept for a language the project no longer has
        for s in kept.values() {
            s.client.stop();
        }
        r
    }

    /// The servers, to keep running for the next read of this project (`keep.rs`).
    pub fn into_servers(self) -> HashMap<&'static str, Server> {
        self.servers
    }

    /// The project's files in one server's language, by path.
    fn files_of(&self, kind: &str) -> Vec<usize> {
        (0..self.project.files.len()).filter(|&i| self.project.files[i].lang.server == kind).collect()
    }

    /// A kept server brought up to date: files new since it last read are opened, changed ones sent whole, gone ones
    /// closed. Then the same wait as at start, unless nothing changed.
    fn refresh(&self, kind: &str, mut s: Server, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool) -> Result<Server, String> {
        let t0 = Instant::now();
        let files = self.files_of(kind);
        let now: HashMap<&PathBuf, &crate::source::SourceFile> = files.iter().map(|&i| (&self.project.files[i].abs, &self.project.files[i])).collect();
        // what differs from what it was given: new files (1), changed ones (2), gone ones (3), as the protocol numbers them
        let mut diffs: Vec<(PathBuf, u8)> = vec![];
        for (p, f) in &now {
            match s.files.get(*p) {
                None => diffs.push(((*p).clone(), 1)),
                Some(&(_, old)) if old != crate::changes::hash(f.text.as_bytes()) => diffs.push(((*p).clone(), 2)),
                Some(_) => {}
            }
        }
        diffs.extend(s.files.keys().filter(|p| !now.contains_key(p)).map(|p| (p.clone(), 3)));
        diffs.sort();
        // first what is on disk now, so the server's view of the folders is right before it reads the new text
        s.client.files_changed(&diffs.iter().map(|(p, t)| (p.as_path(), *t)).collect::<Vec<_>>());
        for (p, t) in &diffs {
            match (t, now.get(p)) {
                (1, Some(f)) => {
                    s.client.open(p, f.lang.lsp_id, &f.text);
                    s.files.insert(p.clone(), (1, crate::changes::hash(f.text.as_bytes())));
                }
                (2, Some(f)) => {
                    let v = s.files.get(p).map(|&(v, _)| v + 1).unwrap_or(1);
                    s.client.change(p, v, &f.text);
                    s.files.insert(p.clone(), (v, crate::changes::hash(f.text.as_bytes())));
                }
                _ => {
                    s.client.close(p);
                    s.files.remove(p);
                }
            }
        }
        let changed = diffs.len();
        on(Progress::ServerKept { kind: kind.to_string(), name: s.client.name.clone(), changed });
        let how = if changed == 0 {
            "nothing changed".to_string()
        } else {
            let settled = self.settle(kind, &s.client, &files, KEPT, on, stop)?;
            format!("{changed} file(s) changed; {settled}")
        };
        s.reads += 1;
        s.readiness = format!("kept running from an earlier read ({} so far); {how}; {} ms", s.reads, t0.elapsed().as_millis());
        Ok(s)
    }

    /// Opens the project's files, waits for the server to finish its own work, then checks that its answers
    /// have stopped changing.
    /// There is no time limit: the wait ends when the answers have settled, when the server exits, or when the user
    /// stops it.
    fn ready_up(&self, kind: &str, s: servers::Started, settings: String, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool) -> Result<Server, String> {
        let client = s.client;
        let files = self.files_of(kind);
        let mut given = HashMap::new();
        for &i in &files {
            let f = &self.project.files[i];
            client.open(&f.abs, f.lang.lsp_id, &f.text);
            given.insert(f.abs.clone(), (1, crate::changes::hash(f.text.as_bytes())));
        }
        let t0 = Instant::now();
        let settled = self.settle(kind, &client, &files, STARTING, on, stop)?;
        let readiness = format!("{settled}; {} ms", t0.elapsed().as_millis());
        Ok(Server { with: s.with, readiness, client, files: given, settings, started: std::time::SystemTime::now(), reads: 1 })
    }

    /// Waits for a server to finish its own work, then until the same questions get the same answers twice.
    /// `pace`: how long it must be quiet first, the pause between two rounds of questions, and how long it must be
    /// quiet before the second. A server starting has no sign of being ready but going quiet; one kept running has
    /// loaded the project already and takes the changes sent to it before any question asked after them.
    fn settle(&self, kind: &str, client: &Client, files: &[usize], pace: Pace, on: &dyn Fn(Progress), stop: &dyn Fn(&str) -> bool) -> Result<String, String> {
        let t0 = Instant::now();
        // waits until the server is quiet, telling the caller what it is doing about once a second
        let wait = |quiet: Duration| -> Result<(), String> {
            while !client.settle(quiet, Duration::from_secs(1)) {
                if stop(kind) {
                    client.stop();
                    return Err("reading was stopped before its server was ready".into());
                }
                if let Some(why) = client.ended() {
                    return Err(format!("{why} (before it was ready)"));
                }
                on(Progress::ServerWaiting { kind: kind.to_string(), seconds: t0.elapsed().as_secs(), doing: client.doing().unwrap_or_default() });
            }
            Ok(())
        };
        wait(pace.0)?;
        // probe: a spread of call sites across the project, asked twice
        let mut probe = vec![];
        let step = (files.len() / 24).max(1);
        for &i in files.iter().step_by(step) {
            let f = &self.project.files[i];
            if let Some(n) = call_names(f).into_iter().find(|n| f.import(f.text_of(*n)).is_some()).or_else(|| call_names(f).into_iter().next()) {
                probe.push(Client::definition_params(&f.abs, n.start_position().row as u32, f.utf16_col(n.start_position().row, n.start_position().column)));
            }
        }
        let ask = |c: &Client| c.request_many("textDocument/definition", probe.clone(), Duration::from_secs(60)).into_iter().map(|r| r.map(|v| Client::locations(&v)).unwrap_or_default()).collect::<Vec<_>>();
        let mut rounds = 0;
        let mut before = ask(client);
        loop {
            rounds += 1;
            std::thread::sleep(pace.1);
            wait(pace.2)?;
            let now = ask(client);
            // a server that has stopped gives the same empty answers every time
            if let Some(why) = client.ended() {
                return Err(format!("{why} (before it was ready)"));
            }
            if now == before {
                break;
            }
            before = now;
            on(Progress::ServerWaiting { kind: kind.to_string(), seconds: t0.elapsed().as_secs(), doing: "checking that its answers have stopped changing".into() });
        }
        Ok(format!(
            "{}; {} probe answers stable after {} round(s)",
            if client.reported_progress() { "server reported its progress" } else { "server reported no progress, waited for it to go quiet" },
            probe.len(),
            rounds
        ))
    }

    pub fn server_for(&self, file: usize) -> Option<&Server> {
        self.servers.get(self.project.files[file].lang.server)
    }

    pub fn ready(&self, file: usize) -> bool {
        self.server_for(file).is_some()
    }

    /// Asks for many names at once (pipelined) and keeps the answers.
    pub fn prefetch(&self, names: &[(usize, Node)]) {
        let mut by_kind: HashMap<&str, Vec<(usize, usize, usize)>> = HashMap::new();
        {
            let cache = self.cache.lock().unwrap();
            for (file, n) in names {
                let key = (*file, n.start_position().row, n.start_position().column);
                if !cache.contains_key(&key) {
                    by_kind.entry(self.project.files[*file].lang.server).or_default().push(key);
                }
            }
        }
        for (kind, keys) in by_kind {
            let Some(s) = self.servers.get(kind) else { continue };
            let params = keys.iter().map(|&(f, row, col)| {
                let file = &self.project.files[f];
                Client::definition_params(&file.abs, row as u32, file.utf16_col(row, col))
            });
            let answers = s.client.request_many("textDocument/definition", params.collect(), Duration::from_secs(120));
            let mut cache = self.cache.lock().unwrap();
            for (k, a) in keys.into_iter().zip(answers) {
                cache.insert(k, a.map(|v| Client::locations(&v)).unwrap_or_default());
            }
        }
    }

    /// "Go to definition" for the name at a node's start.
    pub fn definition(&self, file: usize, n: Node) -> Vec<Loc> {
        self.definition_at(file, n.start_position().row, n.start_position().column)
    }

    pub fn definition_at(&self, file: usize, row: usize, byte_col: usize) -> Vec<Loc> {
        let key = (file, row, byte_col);
        if let Some(v) = self.cache.lock().unwrap().get(&key) {
            return v.clone();
        }
        let f = &self.project.files[file];
        let v = match self.servers.get(f.lang.server) {
            Some(s) => s
                .client
                .request("textDocument/definition", Client::definition_params(&f.abs, row as u32, f.utf16_col(row, byte_col)), Duration::from_secs(60))
                .map(|v| Client::locations(&v))
                .unwrap_or_default(),
            None => vec![],
        };
        self.cache.lock().unwrap().insert(key, v.clone());
        v
    }

    /// The signature the server shows on hover for the name at a node (the code part only).
    pub fn hover(&self, file: usize, n: Node) -> Option<String> {
        let key = (file, n.start_position().row, n.start_position().column);
        if let Some(v) = self.hovers.lock().unwrap().get(&key) {
            return v.clone();
        }
        let f = &self.project.files[file];
        let s = self.servers.get(f.lang.server)?;
        let params = Client::definition_params(&f.abs, key.1 as u32, f.utf16_col(key.1, key.2));
        let v = s.client.request("textDocument/hover", params, Duration::from_secs(30)).ok().and_then(|v| hover_code(&v));
        self.hovers.lock().unwrap().insert(key, v.clone());
        v
    }

    pub fn at_loc(&self, loc: &Loc) -> Option<(usize, Node<'p>)> {
        let i = self.project.file_index(&loc.file)?;
        let f = &self.project.files[i];
        Some((i, f.node_at(loc.line as usize, loc.col)?))
    }

    /// How many bindings there are now (to return to with `unbind`).
    pub fn bindings(&self) -> usize {
        self.bound.lock().map(|b| b.len()).unwrap_or(0)
    }
    /// Parameter `name` of `func` stands for `value` from now until `unbind`.
    pub fn bind(&self, func: NodeKey, name: String, value: At<'_>) {
        if let Ok(mut b) = self.bound.lock() {
            b.push((func, name, value.key()));
        }
    }
    pub fn unbind(&self, to: usize) {
        if let Ok(mut b) = self.bound.lock() {
            b.truncate(to);
        }
    }
    /// What the parameter `name` of `func` stands for now.
    pub fn bound_to(&self, func: NodeKey, name: &str) -> Option<At<'p>> {
        let key = self.bound.lock().ok()?.iter().rev().find(|(f, n, _)| *f == func && n == name).map(|(_, _, v)| *v)?;
        let file = &self.project.files[key.file];
        let mut n = file.tree.root_node().descendant_for_byte_range(key.start, key.end)?;
        loop {
            if n.start_byte() == key.start && n.end_byte() == key.end && n.kind_id() == key.kind {
                return Some(At { file: key.file, node: n });
            }
            n = n.parent()?;
        }
    }

    /// The functions a name stands for when a framework makes it from a choice between them
    /// (`[state, act] = useActionState(ok ? save : send, init)`: `save` and `send`). Empty for any other name.
    pub fn choices(&self, file: usize, name: Node<'p>, rules: &dyn ValueRules) -> Vec<(At<'p>, Option<At<'p>>)> {
        let Some((df, dn)) = self.definition(file, name).into_iter().next().and_then(|loc| self.at_loc(&loc)) else { return vec![] };
        let f = &self.project.files[df];
        let l = f.lang;
        let Some(pattern) = dn.parent().filter(|p| l.follow.list.is_some_and(|(patterns, _)| patterns.contains(&p.kind()))) else { return vec![] };
        let Some(place) = named(pattern).iter().position(|x| *x == dn) else { return vec![] };
        let source = pattern.parent().and_then(|x| x.child_by_field_name(l.follow.declarator(x.kind())?.1)).map(|v| l.unwrap(v));
        let Some((values, _)) = source.and_then(|s| rules.gives_at(self, df, s, place)) else { return vec![] };
        // each side of a choice, down to the functions
        let mut out = vec![];
        let mut stack: Vec<At<'p>> = values;
        while let Some(v) = stack.pop() {
            let vf = &self.project.files[v.file];
            let node = vf.lang.unwrap(v.node);
            if let Expr::Cond { then, els, .. } = (vf.lang.expr)(node, &vf.text) {
                stack.extend(els.into_iter().chain([then]).map(|n| At { file: v.file, node: n }));
            } else if let Target::Own { func, via, .. } = self.target(v.file, node, rules) {
                out.push((func, via));
            }
        }
        out.sort_by_key(|(a, via)| (a.file, a.node.start_byte(), via.map(|v| (v.file, v.node.start_byte()))));
        out.dedup_by_key(|(a, via)| (a.key(), via.map(|v| v.key())));
        out
    }

    /// The function a callee expression finally reaches.
    pub fn target(&self, file: usize, callee: Node<'p>, rules: &dyn ValueRules) -> Target<'p> {
        if let Some((r, how)) = rules.redirect(self, file, callee) {
            let mut t = self.target(r.file, r.node, rules);
            if let Target::Own { hops, .. } = &mut t {
                hops.insert(0, how.to_string());
            }
            return t;
        }
        let l = self.project.files[file].lang;
        let callee = l.unwrap(callee);
        if l.is_function(callee.kind()) {
            return Target::Own { func: At { file, node: callee }, hops: vec![], via: None };
        }
        let Some(name) = callee_name(callee, l) else { return Target::Unresolved("no name to resolve".into()) };
        let Some(first) = self.definition(file, name).into_iter().next() else { return Target::Unresolved("no definition".into()) };
        if !self.project.is_own(&first.file) || self.project.file_index(&first.file).is_none() {
            return Target::Outside(first);
        }
        self.follow(first, 0, vec![], rules)
    }

    /// The function a definition location names, if it names one.
    pub fn function_at(&self, file: usize, n: Node<'p>, rules: &dyn ValueRules) -> Option<At<'p>> {
        let f = &self.project.files[file];
        let l = f.lang;
        if l.is_function(n.kind()) {
            return Some(At { file, node: n });
        }
        if let Some(func) = l.function_of(n) {
            return Some(At { file, node: func });
        }
        if let Some(member) = (l.abstract_member)(n) {
            return Some(At { file, node: member });
        }
        let p = n.parent()?;
        let value_fn = |v: Node<'p>| -> Option<At<'p>> {
            let v = l.unwrap(v);
            if l.is_function(v.kind()) {
                return Some(At { file, node: v });
            }
            if l.call_field(v.kind()).is_some() {
                let inner = l.unwrap(rules.wrapped(f, v)?);
                if l.is_function(inner.kind()) {
                    return Some(At { file, node: inner });
                }
            }
            None
        };
        if let Some((_, v)) = l.declared(n) {
            return value_fn(v?);
        }
        if let Some((_, pair, key, value, _)) = l.follow.object {
            if p.kind() == pair && p.child_by_field_name(key).is_some_and(|x| x == n) {
                return value_fn(p.child_by_field_name(value)?);
            }
        }
        None
    }

    fn follow(&self, loc: Loc, depth: usize, hops: Vec<String>, rules: &dyn ValueRules) -> Target<'p> {
        if depth > 6 {
            return Target::Unresolved("too many hops".into());
        }
        let Some((file, node)) = self.at_loc(&loc) else { return Target::Outside(loc) };
        if let Some(func) = self.function_at(file, node, rules) {
            return Target::Own { func, hops, via: None };
        }
        let f = &self.project.files[file];
        let l = f.lang;
        let fl = &l.follow;
        let jump = |n: Node<'p>, why: &str| -> Target<'p> {
            let next = self.definition(file, n).into_iter().next();
            match next {
                Some(next) if next != loc => {
                    if !self.project.is_own(&next.file) || self.project.file_index(&next.file).is_none() {
                        return Target::Outside(next);
                    }
                    let mut h = hops.clone();
                    h.push(why.to_string());
                    self.follow(next, depth + 1, h, rules)
                }
                _ => Target::Unresolved(format!("{why}: no further definition")),
            }
        };
        let Some(p) = node.parent() else { return Target::Unresolved("no node".into()) };

        // `name = other` / `name = obj.member`: a plain alias, keep going
        if let Some((_, v)) = l.declared(node) {
            if let Some(v) = v.map(|v| l.unwrap(v)) {
                if l.call_field(v.kind()).is_none() {
                    if let Some(n) = callee_name(v, l) {
                        return jump(n, "alias");
                    }
                }
                // `name = make()`: the function that the project's `make` returns; what a package's call makes is the
                // package's
                if let Some(callee) = l.call_field(v.kind()).and_then(|cf| v.child_by_field_name(cf)) {
                    match self.target(file, callee, rules) {
                        Target::Own { func, .. } => {
                            let made = format!("returned by {}", callee_name(callee, l).map(|n| f.text_of(n)).unwrap_or("the call"));
                            for (rf, rv) in self.returned_objects(func) {
                                let rl = self.project.files[rf].lang;
                                if rl.is_function(rv.kind()) {
                                    let mut h = hops.clone();
                                    h.push(made);
                                    return Target::Own { func: At { file: rf, node: rv }, hops: h, via: Some(At { file, node: v }) };
                                }
                            }
                        }
                        Target::Outside(loc) => return Target::Outside(loc),
                        Target::Unresolved(why) if why == "an import not followed" => return Target::Unresolved("made by an import not followed".into()),
                        _ => {}
                    }
                }
            }
            return Target::Unresolved("a value made here, not a function".into());
        }

        // `{ key }` / `{ key: value }` in an object literal: the value
        if let Some((obj, pair, key, value, shorthand)) = fl.object {
            if node.kind() == shorthand && p.kind() == obj {
                return jump(node, "property");
            }
            if p.kind() == pair && p.child_by_field_name(key).is_some_and(|x| x == node) {
                if let Some(n) = p.child_by_field_name(value).and_then(|v| callee_name(v, l)) {
                    return jump(n, "property");
                }
            }
        }

        // `{ key } = source` / `{ key: name } = source`: the value of `key` in what the source is
        if let Some(d) = &fl.destructure {
            let mut key = None;
            let mut pattern = None;
            if node.kind() == d.shorthand {
                key = Some(f.text_of(node).to_string());
                pattern = Some(p);
            } else if p.kind() == d.pair.0 && p.child_by_field_name(d.pair.2).is_some_and(|x| x == node) {
                key = p.child_by_field_name(d.pair.1).map(|k| f.text_of(k).trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string());
                pattern = p.parent();
            }
            if let (Some(key), Some(pattern)) = (key, pattern.filter(|x| x.kind() == d.pattern)) {
                if pattern.parent().is_some_and(|x| fl.parameter.contains(&x.kind())) {
                    return Target::Unresolved("parameter (value comes from the caller)".into());
                }
                let source = pattern.parent().and_then(|x| x.child_by_field_name(fl.declarator(x.kind())?.1)).map(|v| l.unwrap(v));
                let Some(source) = source else { return Target::Unresolved("destructured from nothing".into()) };
                let mut h = hops.clone();
                h.push(format!("returned as {key}"));
                return match self.member_of(file, source, &key, depth, h, rules) {
                    Target::Unresolved(why) => Target::Unresolved(format!("taken from {}: {why}", one_line(f.text_of(source), 30))),
                    t => t,
                };
            }
        }

        // `[a, b] = source`: the value at that place in the list the project's function returns
        if let Some((_, lists)) = fl.list.filter(|(patterns, _)| patterns.contains(&p.kind())) {
            let place = named(p).iter().position(|x| *x == node);
            let source = p.parent().and_then(|x| x.child_by_field_name(fl.declarator(x.kind())?.1)).map(|v| l.unwrap(v));
            let callee = source.and_then(|v| v.child_by_field_name(l.call_field(v.kind())?));
            if let (Some(i), Some(callee)) = (place, callee) {
                // what a framework says a call's list holds at this place: followed when it is one value; several
                // (a choice between two functions) are told apart where they are used
                if let Some((values, how)) = source.and_then(|s| rules.gives_at(self, file, s, i)) {
                    let mut h = hops.clone();
                    h.push(how.to_string());
                    if let [v] = values.as_slice() {
                        let vl = self.project.files[v.file].lang;
                        let item = vl.unwrap(v.node);
                        if vl.is_function(item.kind()) {
                            return Target::Own { func: At { file: v.file, node: item }, hops: h, via: None };
                        }
                        if let Some(next) = callee_name(item, vl).and_then(|n| self.definition(v.file, n).into_iter().next()) {
                            return self.follow(next, depth + 1, h, rules);
                        }
                    }
                    return Target::Declared(At { file, node });
                }
                if let Target::Own { func, .. } = self.target(file, callee, rules) {
                    for (rf, rv) in self.returned_objects(func) {
                        let rl = self.project.files[rf].lang;
                        let Some(item) = Some(rl.unwrap(rv)).filter(|v| lists.contains(&v.kind())).and_then(|v| named(v).get(i).map(|x| rl.unwrap(*x))) else { continue };
                        let mut h = hops.clone();
                        h.push(format!("item {} of what {} returns", i + 1, callee_name(callee, l).map(|n| f.text_of(n)).unwrap_or("the call")));
                        if rl.is_function(item.kind()) {
                            return Target::Own { func: At { file: rf, node: item }, hops: h, via: None };
                        }
                        if let Some(next) = callee_name(item, rl).and_then(|n| self.definition(rf, n).into_iter().next()) {
                            return self.follow(next, depth + 1, h, rules);
                        }
                    }
                }
            }
            return Target::Declared(At { file, node });
        }
        if fl.parameter.contains(&p.kind()) {
            return Target::Unresolved("parameter (value comes from the caller)".into());
        }
        // a name the server leaves at an import it cannot follow further: what it is is not in sight
        if std::iter::successors(Some(p), |x| x.parent()).any(|x| l.reach.not_uses.contains(&x.kind())) {
            return Target::Unresolved("an import not followed".into());
        }
        // a class's own name, or that of one of its constructors (or members) with no body: calling it makes an object
        let mut up = Some(p);
        while let Some(x) = up.filter(|x| !l.is_function(x.kind()) && !fl.declares(x.kind())) {
            if l.reach.classes.contains(&x.kind()) {
                return Target::Unresolved("a class".into());
            }
            up = x.parent();
        }
        Target::Declared(At { file, node })
    }

    /// The function stored under `key` in the value of `source`: in the object a called function returns,
    /// or in an object literal a name points at.
    fn member_of(&self, file: usize, source: Node<'p>, key: &str, depth: usize, hops: Vec<String>, rules: &dyn ValueRules) -> Target<'p> {
        let l = self.project.files[file].lang;
        let mut hops = hops;
        let objects: Vec<(usize, Node<'p>)> = if let Some(field) = l.call_field(source.kind()) {
            let Some(callee) = source.child_by_field_name(field) else { return Target::Unresolved("call without callee".into()) };
            // `await import("./store")`: the module loaded, and the name it declares at its top
            if l.dynamic_import == Some(l.unwrap(callee).kind()) {
                let module = source.child_by_field_name("arguments").and_then(|a| named(a).into_iter().next());
                let Some(loc) = module.and_then(|m| self.definition(file, m).into_iter().next()) else { return Target::Unresolved("a module loaded at runtime, not found".into()) };
                let Some(mf) = self.project.file_index(&loc.file) else { return Target::Outside(loc) };
                let Some(at) = self.declared_at_top(mf, key) else { return Target::Unresolved(format!("no \"{key}\" declared in the module loaded")) };
                hops.push("loaded by import(…)".into());
                return self.follow(at, depth + 1, hops, rules);
            }
            match rules.gives(self, file, source) {
                Some((values, how)) => {
                    hops.push(how.to_string());
                    values.into_iter().map(|v| (v.file, v.node)).collect()
                }
                None => match self.target(file, callee, rules) {
                    Target::Own { func, .. } => self.returned_objects(func),
                    Target::Outside(loc) => return Target::Outside(loc),
                    Target::Declared(_) => return Target::Unresolved("returning function: a declaration".into()),
                    Target::Unresolved(why) => return Target::Unresolved(format!("returning function: {why}")),
                },
            }
        } else if let Some(n) = callee_name(source, l) {
            let Some(loc) = self.definition(file, n).into_iter().next() else { return Target::Unresolved("source not resolved".into()) };
            let Some((sf, sn)) = self.at_loc(&loc) else { return Target::Outside(loc) };
            let sl = self.project.files[sf].lang;
            sn.parent().and_then(|d| d.child_by_field_name(sl.follow.declarator(d.kind())?.1)).map(|v| vec![(sf, sl.unwrap(v))]).unwrap_or_default()
        } else {
            vec![]
        };
        // a value that is not written as an object here: where it is made (`return ctx`, `ctx = useContext(…)`)
        let mut found = vec![];
        for (of, obj) in objects {
            self.objects_of(of, obj, rules, &mut hops, &mut found, 0);
        }
        for (of, obj) in found {
            let ol = self.project.files[of].lang;
            let Some((oty, pair, kf, vf, shorthand)) = ol.follow.object else { continue };
            if obj.kind() != oty {
                continue;
            }
            let mut c = obj.walk();
            for prop in obj.named_children(&mut c) {
                let f = &self.project.files[of];
                if prop.kind() == shorthand && f.text_of(prop) == key {
                    if let Some(next) = self.definition(of, prop).into_iter().next() {
                        return self.follow(next, depth + 1, hops, rules);
                    }
                }
                if prop.kind() == pair && prop.child_by_field_name(kf).is_some_and(|k| f.text_of(k).trim_matches(|c| c == '"' || c == '\'') == key) {
                    let Some(v) = prop.child_by_field_name(vf).map(|v| ol.unwrap(v)) else { continue };
                    if ol.is_function(v.kind()) {
                        return Target::Own { func: At { file: of, node: v }, hops, via: None };
                    }
                    if let Some(n) = callee_name(v, ol) {
                        if let Some(next) = self.definition(of, n).into_iter().next() {
                            return self.follow(next, depth + 1, hops, rules);
                        }
                    }
                }
            }
        }
        Target::Unresolved(format!("no \"{key}\" found in the value"))
    }

    /// The object literals a value is: one written there, else where a name is given its value and what a call gives
    /// (as a framework says, else what the project's function returns), a few steps deep. How a framework's call is
    /// followed is added to `hops`.
    fn objects_of(&self, file: usize, n: Node<'p>, rules: &dyn ValueRules, hops: &mut Vec<String>, out: &mut Vec<(usize, Node<'p>)>, depth: usize) {
        let l = self.project.files[file].lang;
        let n = l.unwrap(n);
        let Some((object, ..)) = l.follow.object else { return };
        if n.kind() == object {
            out.push((file, n));
            return;
        }
        if depth > 3 {
            return;
        }
        if let Some(field) = l.call_field(n.kind()) {
            if let Some((values, how)) = rules.gives(self, file, n) {
                if !hops.iter().any(|h| h == how) {
                    hops.push(how.to_string());
                }
                for v in values {
                    self.objects_of(v.file, v.node, rules, hops, out, depth + 1);
                }
            } else if let Some(Target::Own { func, .. }) = n.child_by_field_name(field).map(|c| self.target(file, c, rules)) {
                for (rf, rv) in self.returned_objects(func) {
                    self.objects_of(rf, rv, rules, hops, out, depth + 1);
                }
            }
            return;
        }
        if !l.is_name(n.kind()) {
            return;
        }
        let Some(loc) = self.definition(file, n).into_iter().next() else { return };
        let Some((df, dn)) = self.at_loc(&loc) else { return };
        if (df, dn) == (file, n) {
            return;
        }
        if let Some((_, Some(v))) = self.project.files[df].lang.declared(dn) {
            self.objects_of(df, v, rules, hops, out, depth + 1);
        }
    }

    /// Where a module declares `name` at its top (a function, or a name given a value), as a location.
    fn declared_at_top(&self, file: usize, name: &str) -> Option<Loc> {
        let f = &self.project.files[file];
        let l = f.lang;
        let mut stack = vec![f.tree.root_node()];
        while let Some(n) = stack.pop() {
            if l.is_name(n.kind()) && f.text_of(n) == name && (l.function_of(n).is_some() || l.declared(n).is_some()) {
                let p = n.start_position();
                return Some(Loc { file: f.abs.clone(), line: p.row as u32, col: f.utf16_col(p.row, p.column) });
            }
            // what is written inside a function or a class is not the module's own
            if l.is_function(n.kind()) || l.reach.classes.contains(&n.kind()) {
                if let Some(x) = (l.function_name)(n).filter(|x| f.text_of(*x) == name && l.is_function(n.kind())) {
                    let p = x.start_position();
                    return Some(Loc { file: f.abs.clone(), line: p.row as u32, col: f.utf16_col(p.row, p.column) });
                }
                continue;
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c).collect::<Vec<_>>().into_iter().rev());
        }
        None
    }

    /// Values of the return statements that belong to a function (not to functions nested in it).
    pub fn returned_objects(&self, func: At<'p>) -> Vec<(usize, Node<'p>)> {
        let f = &self.project.files[func.file];
        let l = f.lang;
        let mut out = vec![];
        let Some(body) = (l.function_body)(func.node) else { return out };
        if !body.kind().contains("block") && !l.follow.returns.is_empty() && body.kind() != l.follow.returns {
            // an expression body is what the function returns
            out.push((func.file, l.unwrap(body)));
        }
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                if l.is_function(ch.kind()) {
                    continue;
                }
                if ch.kind() == l.follow.returns {
                    if let Some(v) = ch.named_child(0) {
                        out.push((func.file, l.unwrap(v)));
                    }
                }
                stack.push(ch);
            }
        }
        out
    }

    /// Imported packages the server cannot find, each with the language server of the files that import it. Asked at
    /// each imported name and where its module is written: no definition outside the importing file means the module
    /// is not installed (or does not exist).
    pub fn missing_modules(&self) -> Vec<(String, &'static str)> {
        let mut names = vec![];
        for (i, f) in self.project.files.iter().enumerate() {
            for imp in &f.imports {
                if !imp.is_path() {
                    names.push((i, imp.line, imp.col, imp.module.clone()));
                    names.push((i, imp.module_at.0, imp.module_at.1, imp.module.clone()));
                }
            }
        }
        let nodes: Vec<(usize, Node)> = names
            .iter()
            .filter_map(|&(i, row, col, _)| {
                let p = tree_sitter::Point { row, column: col };
                self.project.files[i].tree.root_node().named_descendant_for_point_range(p, p).map(|n| (i, n))
            })
            .collect();
        self.prefetch(&nodes);
        // a package counts as missing only if none of its imports reach anything: one module can re-export a name
        // from another package, and then it is that other package that is missing
        let mut found: std::collections::BTreeMap<(String, &'static str), bool> = Default::default();
        for (i, row, col, module) in names {
            // a server that stopped gives no answers, which would make every package look missing
            if self.server_for(i).is_none_or(|s| s.client.ended().is_some()) {
                continue;
            }
            let reached = self.definition_at(i, row, col).into_iter().any(|l| l.file != self.project.files[i].abs);
            let l = self.project.files[i].lang;
            *found.entry((package_of(&module, l.package_sep), l.server)).or_default() |= reached;
        }
        let missing: BTreeSet<(String, &'static str)> = found.into_iter().filter(|(_, reached)| !reached).map(|(p, _)| p).collect();
        missing.into_iter().collect()
    }

    /// The servers that stopped after they were ready, and why. Their answers after that are missing, so what was
    /// built from them is incomplete.
    pub fn ended(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.servers.iter().filter_map(|(k, s)| Some((k.to_string(), s.client.ended()?))).collect();
        out.sort();
        out
    }

    pub fn stop(&self) {
        for s in self.servers.values() {
            s.client.stop();
        }
    }
}

/// A node's named children, without comments.
fn named(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).filter(|x| x.kind() != "comment").collect()
}

/// The installable package a module name belongs to: `@scope/name/sub` → `@scope/name`, `a/b` → `a`, `a.b` → `a`.
pub fn package_of(module: &str, sep: char) -> String {
    if module.contains("://") {
        return module.to_string();
    }
    // a specifier scheme (`npm:pkg@2`) is not part of the package name, nor is a version after it
    let module = module.rsplit_once(':').map(|(_, m)| m).unwrap_or(module);
    let module = if module.len() > 1 { module[1..].split_once('@').map(|(m, _)| &module[..m.len() + 1]).unwrap_or(module) } else { module };
    if module.starts_with('@') {
        return module.splitn(3, sep).take(2).collect::<Vec<_>>().join(&sep.to_string());
    }
    module.split(sep).next().unwrap_or(module).to_string()
}

/// The code shown in a hover answer (the first code block, or the plain value).
fn hover_code(v: &serde_json::Value) -> Option<String> {
    let c = v.get("contents")?;
    let raw = match c {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => a.first().and_then(|x| x.get("value").and_then(|v| v.as_str()).or(x.as_str())).unwrap_or("").to_string(),
        o => o.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    };
    let code = match raw.find("```") {
        Some(i) => {
            let after = &raw[i + 3..];
            let body = after.split_once('\n').map(|(_, b)| b).unwrap_or(after);
            body.split("```").next().unwrap_or(body).trim().to_string()
        }
        None => raw.trim().to_string(),
    };
    if code.is_empty() { None } else { Some(code) }
}
