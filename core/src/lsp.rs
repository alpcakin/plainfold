// A small blocking LSP client over stdio, and readiness detection.
//
// LSP has no standard "ready" signal, and a server asked too early gives answers that look valid but are weaker
// (it has not loaded the project, or not applied the environment yet). So the client tracks what the server says
// about its own work (progress reports, settings requests, log traffic) and the caller confirms with a probe:
// the same questions asked twice must get the same answers. Answers are only called exact once the server is ready.
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Loc {
    pub file: PathBuf,
    pub line: u32, // 0-based
    pub col: u32,  // 0-based, UTF-16
}

#[derive(Default)]
struct Activity {
    last: Option<Instant>,
    active: HashSet<String>, // progress tokens begun and not ended
    /// what the server last said it is doing (a progress title and message), while some work is under way
    doing: Option<String>,
    progress_seen: bool,
    settings_asked: usize,
    trace: Vec<String>,
}

/// Questions waiting for an answer, and why no more will come once the server's output has closed.
#[derive(Default)]
struct Waiting {
    by_id: HashMap<i64, Sender<Result<Value, String>>>,
    ended: Option<String>,
}

type Pending = Arc<Mutex<Waiting>>;

/// How many of the last lines of the server's error output are kept, to say why it stopped.
const STDERR_TAIL: usize = 20;

pub struct Client {
    pub name: String,
    stdin: Arc<Mutex<ChildStdin>>,
    child: Arc<Mutex<Child>>,
    next: Mutex<i64>,
    pending: Pending,
    activity: Arc<Mutex<Activity>>,
    opened: Mutex<HashSet<PathBuf>>,
}

fn write_msg(stdin: &Mutex<ChildStdin>, v: &Value) {
    let s = v.to_string();
    let mut w = stdin.lock().unwrap();
    let _ = write!(w, "Content-Length: {}\r\n\r\n{}", s.len(), s);
    let _ = w.flush();
}

pub fn uri(p: &Path) -> String {
    let s = p.to_string_lossy();
    let mut out = String::from("file://");
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn path_of(uri: &str) -> PathBuf {
    let s = uri.strip_prefix("file://").unwrap_or(uri);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).to_string())
}

impl Client {
    pub fn start(name: &str, cmd: &Path, args: &[&str], cwd: &Path, settings: Value) -> std::io::Result<Client> {
        let mut child = Command::new(cmd).args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let child = Arc::new(Mutex::new(child));
        let pending: Pending = Arc::new(Mutex::new(Waiting::default()));
        let activity = Arc::new(Mutex::new(Activity::default()));
        let debug = std::env::var("PLAINFOLD_LSP_DEBUG").is_ok();
        let started = Instant::now();
        // the server's error output, read line by line so it never blocks on it; the last lines are kept
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let (stderr_done, stderr_closed) = channel::<()>();
        {
            let tail = tail.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(stderr);
                let mut line = Vec::new();
                while r.read_until(b'\n', &mut line).unwrap_or(0) > 0 {
                    let text = String::from_utf8_lossy(&line).trim_end().to_string();
                    if debug {
                        eprintln!("[{:>6} ms] stderr {text}", started.elapsed().as_millis());
                    }
                    let mut t = tail.lock().unwrap();
                    if !text.trim().is_empty() {
                        t.push_back(text);
                    }
                    if t.len() > STDERR_TAIL {
                        t.pop_front();
                    }
                    line.clear();
                }
                let _ = stderr_done.send(());
            });
        }
        {
            let (stdin, pending, activity, child) = (stdin.clone(), pending.clone(), activity.clone(), child.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(stdout);
                'read: loop {
                    let mut len = 0usize;
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if r.read_line(&mut line).unwrap_or(0) == 0 {
                            break 'read;
                        }
                        let t = line.trim();
                        if t.is_empty() {
                            break;
                        }
                        if let Some(v) = t.strip_prefix("Content-Length:") {
                            len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut buf = vec![0u8; len];
                    if r.read_exact(&mut buf).is_err() {
                        break;
                    }
                    let Ok(msg) = serde_json::from_slice::<Value>(&buf) else { continue };
                    on_message(&msg, &stdin, &pending, &activity, &settings, debug, started);
                }
                // No more answers can come. The error output and the exit status say why; both can trail the
                // closed output a little, so wait for them briefly.
                let _ = stderr_closed.recv_timeout(Duration::from_secs(2));
                let deadline = Instant::now() + Duration::from_secs(2);
                let status = loop {
                    match child.lock().unwrap().try_wait() {
                        Ok(Some(s)) => break Some(s),
                        Ok(None) if Instant::now() < deadline => {}
                        _ => break None,
                    }
                    std::thread::sleep(Duration::from_millis(20));
                };
                let why = ended_because(status, &tail.lock().unwrap());
                let mut p = pending.lock().unwrap();
                p.ended = Some(why.clone());
                for (_, tx) in p.by_id.drain() {
                    let _ = tx.send(Err(why.clone()));
                }
            });
        }
        Ok(Client { name: name.to_string(), stdin, child, next: Mutex::new(0), pending, activity, opened: Mutex::new(HashSet::new()) })
    }

    fn send_request(&self, method: &str, params: Value) -> (i64, std::sync::mpsc::Receiver<Result<Value, String>>) {
        let id = {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            *n
        };
        let (tx, rx) = channel();
        {
            let mut p = self.pending.lock().unwrap();
            match &p.ended {
                Some(why) => {
                    let _ = tx.send(Err(why.clone()));
                    return (id, rx);
                }
                None => p.by_id.insert(id, tx),
            };
        }
        write_msg(&self.stdin, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        (id, rx)
    }

    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let (id, rx) = self.send_request(method, params);
        let r = rx.recv_timeout(timeout).map_err(|_| format!("timeout: {method}"));
        self.pending.lock().unwrap().by_id.remove(&id);
        r?
    }

    /// Sends many requests before waiting for any answer; answers come back in the same order.
    pub fn request_many(&self, method: &str, params: Vec<Value>, timeout: Duration) -> Vec<Result<Value, String>> {
        let mut out = Vec::with_capacity(params.len());
        for chunk in params.chunks(256) {
            let sent: Vec<_> = chunk.iter().map(|p| self.send_request(method, p.clone())).collect();
            for (id, rx) in sent {
                out.push(rx.recv_timeout(timeout).map_err(|_| format!("timeout: {method}")).and_then(|r| r));
                self.pending.lock().unwrap().by_id.remove(&id);
            }
        }
        out
    }

    pub fn notify(&self, method: &str, params: Value) {
        write_msg(&self.stdin, &json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    pub fn initialize(&self, root: &Path, init: Value) -> Result<(), String> {
        let u = uri(root);
        self.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": u,
                "workspaceFolders": [{ "uri": u, "name": root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() }],
                "capabilities": {
                    "textDocument": { "definition": { "linkSupport": false }, "synchronization": {} },
                    "workspace": { "configuration": true, "workspaceFolders": true },
                    "window": { "workDoneProgress": true }
                },
                "initializationOptions": init
            }),
            Duration::from_secs(120),
        )?;
        self.notify("initialized", json!({}));
        Ok(())
    }

    pub fn open(&self, file: &Path, lsp_id: &str, text: &str) {
        if !self.opened.lock().unwrap().insert(file.to_path_buf()) {
            return;
        }
        self.notify("textDocument/didOpen", json!({ "textDocument": { "uri": uri(file), "languageId": lsp_id, "version": 1, "text": text } }));
    }

    /// The whole new text of an open file; `version` must grow with every change.
    pub fn change(&self, file: &Path, version: i32, text: &str) {
        self.notify("textDocument/didChange", json!({ "textDocument": { "uri": uri(file), "version": version }, "contentChanges": [{ "text": text }] }));
    }

    /// Files created (1), changed (2) or deleted (3) on disk. Servers keep their own view of the folders (which
    /// modules exist), and this is how they hear that it changed.
    pub fn files_changed(&self, changes: &[(&Path, u8)]) {
        if !changes.is_empty() {
            let list: Vec<Value> = changes.iter().map(|(p, t)| json!({ "uri": uri(p), "type": t })).collect();
            self.notify("workspace/didChangeWatchedFiles", json!({ "changes": list }));
        }
    }

    /// A file that is gone: the server reads it from disk again, or forgets it.
    pub fn close(&self, file: &Path) {
        if self.opened.lock().unwrap().remove(file) {
            self.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri(file) } }));
        }
    }

    /// The server's process, to measure what it holds.
    pub fn pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }

    pub fn definition_params(file: &Path, line: u32, col: u32) -> Value {
        json!({ "textDocument": { "uri": uri(file) }, "position": { "line": line, "character": col } })
    }

    pub fn locations(v: &Value) -> Vec<Loc> {
        let list: Vec<&Value> = match v {
            Value::Null => vec![],
            Value::Array(a) => a.iter().collect(),
            o => vec![o],
        };
        list.into_iter()
            .filter_map(|l| {
                let u = l.get("targetUri").or_else(|| l.get("uri"))?.as_str()?;
                let r = l.get("targetSelectionRange").or_else(|| l.get("range"))?;
                let s = r.get("start")?;
                Some(Loc { file: path_of(u), line: s.get("line")?.as_u64()? as u32, col: s.get("character")?.as_u64()? as u32 })
            })
            .collect()
    }

    /// What the server says it is doing right now, if it reported work that has not ended.
    pub fn doing(&self) -> Option<String> {
        self.activity.lock().unwrap().doing.clone()
    }

    /// Why the server can no longer answer, once its output has closed (it exited, crashed or ran out of memory).
    pub fn ended(&self) -> Option<String> {
        self.pending.lock().unwrap().ended.clone()
    }

    /// Waits until the server has stopped reporting work: every progress report it began has ended, and it has
    /// been quiet for `quiet`. Returns false if that did not happen within `max`.
    pub fn settle(&self, quiet: Duration, max: Duration) -> bool {
        let start = Instant::now();
        loop {
            {
                let a = self.activity.lock().unwrap();
                let idle = a.last.map(|l| l.elapsed() >= quiet).unwrap_or(start.elapsed() >= quiet);
                if a.active.is_empty() && idle {
                    return true;
                }
            }
            if start.elapsed() >= max {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn reported_progress(&self) -> bool {
        self.activity.lock().unwrap().progress_seen
    }

    pub fn stop(&self) {
        let _ = self.request("shutdown", Value::Null, Duration::from_secs(5));
        self.notify("exit", Value::Null);
        let _ = self.child.lock().unwrap().kill();
    }
}

fn on_message(msg: &Value, stdin: &Mutex<ChildStdin>, pending: &Pending, activity: &Mutex<Activity>, settings: &Value, debug: bool, started: Instant) {
    let method = msg.get("method").and_then(|m| m.as_str());
    let id = msg.get("id");
    let mut a = activity.lock().unwrap();
    a.last = Some(Instant::now());
    let log = |a: &mut Activity, s: String| {
        if debug {
            eprintln!("[{:>6} ms] {s}", started.elapsed().as_millis());
        }
        a.trace.push(format!("{:>6} ms {s}", started.elapsed().as_millis()));
    };
    match (method, id) {
        (None, Some(id)) => {
            drop(a);
            if let Some(tx) = id.as_i64().and_then(|i| pending.lock().unwrap().by_id.remove(&i)) {
                let r = match msg.get("error") {
                    Some(e) => Err(e.get("message").and_then(|m| m.as_str()).unwrap_or("error").to_string()),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(r);
            }
        }
        (Some(m), Some(id)) => {
            // a request from the server: answer the ones that matter, null for the rest
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let result = match m {
                "workspace/configuration" => {
                    a.settings_asked += 1;
                    let items = params.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
                    Value::Array(items.iter().map(|i| i.get("section").and_then(|s| s.as_str()).and_then(|s| settings.get(s)).cloned().unwrap_or(json!({}))).collect())
                }
                "window/workDoneProgress/create" => {
                    if let Some(t) = params.get("token") {
                        a.active.insert(t.to_string());
                        a.progress_seen = true;
                    }
                    Value::Null
                }
                _ => Value::Null,
            };
            log(&mut a, format!("server asks {m} {}", short(&params)));
            drop(a);
            write_msg(stdin, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
        (Some(m), None) => {
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            if m == "$/progress" {
                let token = params.get("token").map(|t| t.to_string()).unwrap_or_default();
                let kind = params.pointer("/value/kind").and_then(|k| k.as_str()).unwrap_or("");
                let title = params.pointer("/value/title").or_else(|| params.pointer("/value/message")).and_then(|t| t.as_str()).unwrap_or("");
                let message = params.pointer("/value/message").and_then(|t| t.as_str()).unwrap_or("");
                match kind {
                    "begin" => {
                        a.active.insert(token.clone());
                        a.progress_seen = true;
                        a.doing = Some(if message.is_empty() || message == title { title.to_string() } else { format!("{title} · {message}") });
                    }
                    "report" if !message.is_empty() => {
                        let head = a.doing.as_deref().and_then(|d| d.split(" · ").next()).unwrap_or("").to_string();
                        a.doing = Some(if head.is_empty() || head == message { message.to_string() } else { format!("{head} · {message}") });
                    }
                    "end" => {
                        a.active.remove(&token);
                        if a.active.is_empty() {
                            a.doing = None;
                        }
                    }
                    _ => {}
                }
                log(&mut a, format!("progress {kind} {token} {title}"));
            } else {
                log(&mut a, format!("{m} {}", short(&params)));
            }
        }
        _ => {}
    }
}

/// Why a server stopped, in words: how it exited, and the last lines of its error output.
fn ended_because(status: Option<std::process::ExitStatus>, tail: &VecDeque<String>) -> String {
    let said = tail.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n");
    let lower = said.to_lowercase();
    let mut why = if lower.contains("out of memory") || lower.contains("outofmemory") {
        "the server ran out of memory".to_string()
    } else {
        match status {
            Some(s) if s.success() => "the server exited".to_string(),
            Some(s) => match (s.code(), signal(&s)) {
                (Some(code), _) => format!("the server exited with code {code}"),
                (None, Some(9)) => "the server was killed (signal 9), for example by the system when memory runs low".to_string(),
                (None, Some(sig)) => format!("the server was stopped by signal {sig}"),
                (None, None) => "the server stopped".to_string(),
            },
            None => "the server closed its output".to_string(),
        }
    };
    // the last few lines are usually the error itself; the lines before them are often a long stack
    let last: Vec<&str> = tail.iter().rev().take(3).map(|l| l.trim()).collect::<Vec<_>>().into_iter().rev().collect();
    if !last.is_empty() {
        why.push_str(": ");
        why.push_str(&last.join(" · "));
    }
    why
}

#[cfg(unix)]
fn signal(s: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    s.signal()
}

#[cfg(not(unix))]
fn signal(_: &std::process::ExitStatus) -> Option<i32> {
    None
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 160 { format!("{}…", &s[..s.char_indices().take_while(|(i, _)| *i < 160).last().map(|(i, _)| i).unwrap_or(0)]) } else { s }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A "server" that reads nothing, says it ran out of memory and exits after a moment.
    fn dying() -> Client {
        let script = "sleep 0.3; echo 'a long stack line' >&2; echo 'FATAL ERROR: heap limit reached - JavaScript heap out of memory' >&2; exit 134";
        Client::start("dying", Path::new("/bin/sh"), &["-c", script], Path::new("/"), Value::Null).expect("sh")
    }

    #[test]
    fn a_server_that_stops_ends_every_question_at_once() {
        let c = dying();
        let t0 = Instant::now();
        // asked before it stops: answered when it stops, not when the minute runs out
        let r = c.request_many("textDocument/definition", vec![Value::Null; 3], Duration::from_secs(60));
        assert!(t0.elapsed() < Duration::from_secs(10), "waited {:?}", t0.elapsed());
        assert!(r.iter().all(|a| a.as_ref().is_err_and(|e| e.contains("ran out of memory"))), "{r:?}");
        // asked after it stopped: answered at once
        let t1 = Instant::now();
        let e = c.request("textDocument/hover", Value::Null, Duration::from_secs(60)).unwrap_err();
        assert!(t1.elapsed() < Duration::from_millis(100));
        assert!(e.contains("JavaScript heap out of memory"), "{e}");
        assert_eq!(c.ended().as_deref(), Some(e.as_str()));
    }

    #[test]
    fn the_exit_status_is_said_when_the_output_says_nothing() {
        let c = Client::start("quiet", Path::new("/bin/sh"), &["-c", "exit 7"], Path::new("/"), Value::Null).expect("sh");
        let e = c.request("initialize", Value::Null, Duration::from_secs(60)).unwrap_err();
        assert_eq!(e, "the server exited with code 7");
    }
}
