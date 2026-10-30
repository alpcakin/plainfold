// Language servers kept running between reads of one project, so that reading it again does not wait for them to
// start and load the project (most of a read). They are used again only for the same project (`resolve.rs` decides
// per server), and they are stopped when this is dropped: whoever holds it decides how long they live.
use crate::resolve::Server;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Kept {
    /// the project they were started for
    pub root: PathBuf,
    pub(crate) servers: HashMap<&'static str, Server>,
}

/// One kept server, as the settings show it.
#[derive(Serialize, Clone, Debug)]
pub struct Held {
    pub kind: String,
    pub name: String,
    /// memory held by the server and the processes it started; None where it cannot be measured
    pub bytes: Option<u64>,
    /// when it started, in seconds since 1970
    pub since: u64,
    /// how many reads it has answered
    pub reads: usize,
}

impl Kept {
    pub fn new(root: PathBuf, servers: HashMap<&'static str, Server>) -> Kept {
        Kept { root, servers }
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// The servers, for a read of this project; they are no longer stopped with this.
    pub(crate) fn take(mut self) -> HashMap<&'static str, Server> {
        std::mem::take(&mut self.servers)
    }

    /// What each server holds now.
    pub fn held(&self) -> Vec<Held> {
        let tree = processes();
        let mut out: Vec<Held> = self
            .servers
            .iter()
            .map(|(kind, s)| Held {
                kind: kind.to_string(),
                name: s.client.name.clone(),
                bytes: tree.as_ref().map(|t| memory_of(t, s.client.pid())),
                since: s.started.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                reads: s.reads,
            })
            .collect();
        out.sort_by(|a, b| a.kind.cmp(&b.kind));
        out
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        for s in self.servers.values() {
            s.client.stop();
        }
    }
}

/// Every process: (id, parent id, resident memory in bytes). None where `ps` is not there to ask (Windows).
fn processes() -> Option<Vec<(u32, u32, u64)>> {
    if cfg!(windows) {
        return None;
    }
    let out = std::process::Command::new("ps").args(["-axo", "pid=,ppid=,rss="]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace().map(|x| x.parse::<u64>().ok());
                Some((w.next()?? as u32, w.next()?? as u32, w.next()?? * 1024))
            })
            .collect(),
    )
}

/// The memory of a process and of every process it started, and they started (a server often runs its engine as a
/// child process).
fn memory_of(all: &[(u32, u32, u64)], pid: u32) -> u64 {
    let mut total = 0;
    let mut stack = vec![pid];
    while let Some(p) = stack.pop() {
        for &(id, parent, rss) in all {
            if id == p {
                total += rss;
            }
            if parent == p {
                stack.push(id);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::memory_of;

    #[test]
    fn a_server_holds_what_its_child_processes_hold() {
        let all = [(10, 1, 100), (11, 10, 50), (12, 11, 5), (20, 1, 999)];
        assert_eq!(memory_of(&all, 10), 155);
    }
}
