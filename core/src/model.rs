// The story model: what the core hands to the view (and later to the English layer).
// Everything here comes from the code. Labels are code text, never English written by a model.
// The JSON shape is the one the diagram view reads.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct Ref {
    pub file: String,
    pub from: usize, // 1-based, inclusive
    pub to: usize,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Device,
    Browser,
    Server,
    Supabase,
    Outside,
    /// the project's code where nothing shows which place runs it (a script, a function nothing in the project
    /// starts)
    Code,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PartKind {
    Screen,
    Component,
    Endpoint,
    Function,
    Database,
    Auth,
    Service,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Participant {
    pub id: String,
    pub name: String,
    pub kind: PartKind,
    pub group: Group,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub at: Option<Ref>,
}

/// How an arrow was resolved. "exact" = proven by the analyzer or a framework rule; "inferred" = not proven.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Link {
    pub kind: LinkKind,
    pub how: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
    Exact,
    Inferred,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Reads,
    Writes,
    Deletes,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelfKind {
    Local,
    State,
    Render,
    Reads,
    Trigger,
    /// changes a value the function is given, or its own object
    Changes,
    /// `break` / `continue`: this round of the loop it is in ends here
    Jump,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FrameKind {
    Alt,
    Loop,
    Effect,
    /// what runs when the code before it throws; its label is the types it handles, when it names them
    Catch,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Call {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub link: Link,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<Effect>,
    #[serde(rename = "ref")]
    pub at: Ref,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Ref>,
    /// what happens inside the callee, when it crosses a boundary itself (the view folds it)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<Item>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opens: Option<String>,
    /// the story this step hands work to, told there: the id of a story that starts at a function handed over here
    /// to run later (a job given to a scheduler)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touches: Option<Vec<String>>,
    /// the caller does nothing with what the call gives back: it is a statement of its own, or thrown away (`void`)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unused: bool,
    /// the name the caller gives what the call gives back (`const { error } = …`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// how the call can end besides returning: what is thrown in it, or in what it calls, and not caught on the way
    /// (the labels of those throws, each once, in source order)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ends: Vec<String>,
    /// the callee's parameters whose values it changes (`items.append(x)`, `item.done = True`), as it names them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Item {
    Call(Call),
    Return {
        id: String,
        from: String,
        to: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(rename = "ref")]
        at: Ref,
        /// the function ends by throwing (`raise`, `throw`): the label says what, the detail is the statement
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        throws: bool,
        /// the function ends here before its end (code after it would run otherwise, or a loop stops): a rule
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        early: bool,
        /// a throw in a `catch` that throws what it caught again (`rethrow`, a bare `raise`, `throw e`): no new way
        /// to end. What it passes on, by the labels of what the `try` throws that this catch handles and the types it
        /// names; empty: nothing named.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passes: Option<Vec<String>>,
    },
    #[serde(rename = "self")]
    Own {
        id: String,
        at: String,
        kind: SelfKind,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(rename = "ref")]
        place: Ref,
    },
    Frame {
        id: String,
        kind: FrameKind,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(rename = "ref")]
        at: Ref,
        items: Vec<Item>,
        #[serde(rename = "else", skip_serializing_if = "Option::is_none")]
        els: Option<Vec<Item>>,
    },
}

impl Item {
    pub fn place(&self) -> &Ref {
        match self {
            Item::Call(c) => &c.at,
            Item::Return { at, .. } | Item::Frame { at, .. } => at,
            Item::Own { place, .. } => place,
        }
    }
    pub fn is_local(&self) -> bool {
        matches!(self, Item::Own { kind: SelfKind::Local, .. })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Finding {
    pub text: String,
    #[serde(rename = "ref")]
    pub at: Ref,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Coverage {
    #[serde(rename = "fn")]
    pub func: String,
    pub file: String,
    pub total: usize,
    pub covered: usize,
    pub missing: Vec<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Trigger {
    pub kind: String, // "screen-open" | "user-action" | "endpoint" | "schedule" | "lifecycle" | "function" | "file" | "test"
    pub route: String,
    #[serde(rename = "ref")]
    pub at: Ref,
    // user-action: what was used, the words on it, and the function it runs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    /// the place it starts in: as its adapter gives it, else where the code that hands the function over runs (see
    /// `discover`). None where the code does not show it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<Group>,
    /// schedule, lifecycle: when it runs, in the adapter's words around the code's values (`every 5 minutes`,
    /// `when the app starts and stops`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// lifecycle: a layout, drawn around the screens under this address (`/docs`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub around: Option<String>,
    // function: how something outside the project reaches it, as far as the code shows
    /// "decorated" | "passed" | "value" | "uncalled"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how: Option<String>,
    /// the code that shows it: the decorator, the call it is passed to, the place it is used as a value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(rename = "byRef", skip_serializing_if = "Option::is_none")]
    pub by_at: Option<Ref>,
    /// classes outside the project that its class extends, as written: code there may call it
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub extends: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Story {
    pub id: String,
    pub title: String,
    pub trigger: Trigger,
    /// other ways the same story starts: triggers on the same screen whose steps are the same (a button and the
    /// keyboard's submit that run one function), in source order
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<Trigger>,
    pub participants: Vec<Participant>,
    pub items: Vec<Item>,
    /// things the code shows for sure (no guessing), worth a look
    pub signals: Vec<Finding>,
    /// what the extractor could not resolve yet
    pub gaps: Vec<Finding>,
    pub coverage: Vec<Coverage>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Stats {
    pub ms: u128,
    #[serde(rename = "llmCalls")]
    pub llm_calls: u32,
}

/// How a story reaches a thing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum UseKind {
    /// the story starts in it
    Starts,
    Calls,
    Reads,
    Writes,
    Deletes,
    /// the code of `from` draws it on the screen (a component written as an element, a widget made while another
    /// builds), whether the story shows it or not
    Draws,
}

/// One place a story reaches a thing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub story: String,
    /// the step that draws it; when it is not drawn itself (inside a helper the story leaves out), the step it is
    /// inside. None: the start of the story, or code the story leaves out at its top level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// `step` draws this use itself (not only holds it)
    pub drawn: bool,
    pub how: UseKind,
    /// the code that does it; for `starts`, the function itself
    #[serde(rename = "ref")]
    pub at: Ref,
    /// the participant whose code does it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// A function, package, service, endpoint or table, with every place the stories reach it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Thing {
    /// a participant id, or `<participant id>/<name>` for a table or database function inside one
    pub id: String,
    pub name: String,
    pub kind: PartKind,
    pub group: Group,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// where it is defined, when it is the project's own
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub at: Option<Ref>,
    /// for a table or database function: the participant it is in
    #[serde(rename = "in", skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    pub uses: Vec<Use>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectModel {
    pub name: String,
    pub stories: Vec<Story>,
    /// everything the stories reach and where, made from the stories of this reading only (`story::reach`), so it
    /// is exactly as fresh as they are
    #[serde(default)]
    pub index: Vec<Thing>,
    /// source of every file the stories point into
    pub files: std::collections::BTreeMap<String, String>,
    pub stats: Stats,
    /// how far the analysis could go: servers that were not ready
    pub setup: Vec<String>,
    /// imported packages that are not installed where the language server looks
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<NotInstalled>,
}

/// An imported package that is not installed where the language server looks, so calls into it are shown by name
/// only. Read from the project's files (`deps.rs`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NotInstalled {
    pub package: String,
    /// the language server of the files that import it: "python" | "typescript" | "dart"
    pub language: String,
    /// the project's manifests that list it under this name
    pub listed: Vec<String>,
    /// where it was looked for: the project's Python environment, when it is inside the project
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// the commands the project's own tools would install what those manifests list with, run from the project's
    /// folder, where the files show the tool. The user runs them; plainfold never does.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install: Vec<String>,
}

// ---------- step identity ----------
// A step is known by what it is and where it sits, not by a counter, so it keeps its id when other code changes:
// lines added above it, a step added before it, or new arguments in the call itself.

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Item::Call(c) => &c.id,
            Item::Return { id, .. } | Item::Own { id, .. } | Item::Frame { id, .. } => id,
        }
    }
    pub(crate) fn id_mut(&mut self) -> &mut String {
        match self {
            Item::Call(c) => &mut c.id,
            Item::Return { id, .. } | Item::Own { id, .. } | Item::Frame { id, .. } => id,
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Item::Call(c) => &c.label,
            Item::Return { label, .. } | Item::Own { label, .. } | Item::Frame { label, .. } => label,
        }
    }
    pub fn detail(&self) -> Option<&str> {
        match self {
            Item::Call(c) => c.detail.as_deref(),
            Item::Return { detail, .. } | Item::Own { detail, .. } => detail.as_deref(),
            Item::Frame { .. } => None,
        }
    }

    /// What the step is, as a key: its kind, its ends, and its code with the values passed left out (string
    /// literals stay: they usually name a table, a route or a function). Two steps with the same signature among
    /// the same siblings are told apart by their order.
    pub fn signature(&self) -> String {
        match self {
            Item::Call(c) => {
                let mut s = format!("call {}>{} {}", c.from, c.to, shape(&c.label));
                if let Some(t) = &c.table {
                    s += &format!(" [{t}]");
                }
                if let Some(o) = &c.opens {
                    s += &format!(" >{o}");
                }
                if let Some(o) = &c.story {
                    s += &format!(" >>{o}");
                }
                s
            }
            Item::Return { from, to, throws: true, .. } => format!("throw {from}>{to}"),
            Item::Return { from, to, .. } => format!("return {from}>{to}"),
            Item::Own { at, kind: SelfKind::Trigger, .. } => format!("self {at} trigger"),
            Item::Own { at, kind, label, .. } => format!("self {at} {kind:?} {}", shape(label)),
            Item::Frame { kind, label, .. } => format!("frame {kind:?} {}", squeeze(label)),
        }
    }

    /// Looser than the signature: steps that are probably the same one edited (a call to the same place with other
    /// code, an `if` with another condition).
    pub fn kin(&self) -> String {
        match self {
            Item::Call(c) => format!("call {}>{}", c.from, c.to),
            Item::Return { from, to, throws: true, .. } => format!("throw {from}>{to}"),
            Item::Return { from, to, .. } => format!("return {from}>{to}"),
            Item::Own { at, kind, .. } => format!("self {at} {kind:?}"),
            Item::Frame { kind, .. } => format!("frame {kind:?}"),
        }
    }

    /// The lists of steps inside this one, each with a name that tells them apart ("" for the main one).
    pub fn lists(&self) -> Vec<(&'static str, &[Item])> {
        match self {
            Item::Call(c) => c.body.as_deref().map(|b| vec![("", b)]).unwrap_or_default(),
            Item::Frame { items, els, .. } => {
                let mut v = vec![("", items.as_slice())];
                if let Some(e) = els {
                    v.push(("else", e.as_slice()));
                }
                v
            }
            _ => vec![],
        }
    }
    pub(crate) fn lists_mut(&mut self) -> Vec<(&'static str, &mut Vec<Item>)> {
        match self {
            Item::Call(c) => c.body.as_mut().map(|b| vec![("", b)]).unwrap_or_default(),
            Item::Frame { items, els, .. } => {
                let mut v = vec![("", items)];
                if let Some(e) = els {
                    v.push(("else", e));
                }
                v
            }
            _ => vec![],
        }
    }
}

/// Gives every step its id: a hash of the path of signatures from the top of the story down to it.
pub fn name_steps(items: &mut [Item]) {
    fn walk(items: &mut [Item], parent: &str) {
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for it in items {
            let sig = it.signature();
            let n = seen.entry(sig.clone()).or_default();
            *n += 1;
            let path = format!("{parent}/{sig}#{n}");
            *it.id_mut() = step_id(&path);
            for (name, list) in it.lists_mut() {
                walk(list, &format!("{path}{name}"));
            }
        }
    }
    walk(items, "");
}

fn step_id(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(path.as_bytes());
    format!("s{}", h[..6].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// Whitespace runs squeezed to one space.
fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Code with what is inside brackets left out, except string literals passed directly to a call:
/// `from("books").insert({ title })` → `from("books").insert()`. A bracket a cut label leaves open drops the rest.
pub fn shape(code: &str) -> String {
    let mut out = String::new();
    let mut stack: Vec<char> = vec![];
    let mut quote: Option<char> = None;
    let mut keep = false; // inside a string literal that is kept
    for ch in code.chars() {
        if let Some(q) = quote {
            if keep {
                out.push(ch);
            }
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                keep = stack.is_empty() || stack == ['('];
                if keep {
                    out.push(ch);
                }
            }
            '(' | '[' | '{' => {
                if stack.is_empty() {
                    out.push(ch);
                }
                stack.push(ch);
            }
            ')' | ']' | '}' => {
                stack.pop();
                if stack.is_empty() {
                    out.push(ch);
                }
            }
            _ if stack.is_empty() => out.push(ch),
            _ => {}
        }
    }
    squeeze(&out)
}

#[cfg(test)]
mod tests {
    use super::shape;

    #[test]
    fn shape_keeps_names_and_drops_values() {
        assert_eq!(shape(r#"from("books").insert({ title, author })"#), r#"from("books").insert()"#);
        assert_eq!(shape("setError((await res.json()).error)"), "setError()");
        assert_eq!(shape(r#"rpc("complete_setup")"#), r#"rpc("complete_setup")"#);
        assert_eq!(shape("setTitleError(messages.errors.unkn…"), "setTitleError(");
        assert_eq!(shape("useEffect(…, [userId])"), "useEffect()");
    }
}
