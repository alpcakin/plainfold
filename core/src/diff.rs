// What changed between two readings of a project, story by story and step by step, the way `git diff` shows lines.
// Stories are matched by id. Among the steps of one list, steps are matched by signature (longest common
// subsequence); where steps were removed and added in the same place, a removed and an added step of the same kin
// count as one changed step. Matched steps whose code differs are changed too. Line numbers are not compared: lines
// added above a step do not change it.
// A function renamed or moved (`renames.rs`) is the same function: the old reading is first given the new ids, so its
// story and the calls to it are changed, not removed and added.
use crate::model::{Item, Participant, ProjectModel, Story, UseKind};
use crate::renames::{Renamed, renames};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    Added,
    Removed,
    Changed,
}

#[derive(Serialize, Clone, Debug)]
pub struct StoryDiff {
    pub id: String,
    pub title: String,
    pub mark: Mark,
    /// for a changed story: its steps that are new, gone or different. A step inside an added or removed step is
    /// not listed again.
    pub steps: Vec<StepDiff>,
    /// for a removed story: the story as it was, so it can still be shown
    #[serde(skip_serializing_if = "Option::is_none")]
    pub story: Option<Story>,
    /// for a changed story: participants it had and has no more, which its removed steps still reach
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gone: Vec<Participant>,
    /// the story starts in a function renamed since: the name it had
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed: Option<String>,
    /// the story starts in a function moved to another file since: the file it was in
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct StepDiff {
    /// the step's id in the new story; for a removed step, "was:" and its id in the old one
    pub id: String,
    pub mark: Mark,
    /// changed: what the step said before
    #[serde(skip_serializing_if = "Option::is_none")]
    pub was: Option<Was>,
    /// removed: the step as it was, with what it held (ids prefixed "was:")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Item>,
    /// removed: where it goes in the new story. `parent` is the new step it was inside (none: the top of the story),
    /// `list` names which of that step's lists ("else"), `after` is the new step it follows (none: first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Was {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Every story that is new, gone or different in `new` compared with `old`: new and changed ones in the new order,
/// then removed ones in the old order.
pub fn diff(old: &ProjectModel, new: &ProjectModel) -> Vec<StoryDiff> {
    let renamed = renames(old, new);
    let (old, story_was) = with_new_ids(old, new, &renamed);
    let ren: HashSet<String> = renamed.iter().map(|r| r.id.clone()).collect();
    let mut out = vec![];
    for s in &new.stories {
        match old.stories.iter().find(|o| o.id == s.id) {
            None => out.push(StoryDiff { id: s.id.clone(), title: s.title.clone(), mark: Mark::Added, steps: vec![], story: None, gone: vec![], renamed: None, moved: None }),
            Some(o) => {
                let mut steps = vec![];
                diff_list(&o.items, &s.items, None, "", &ren, &mut steps);
                let was = story_was.get(&s.id);
                if !steps.is_empty() || was.is_some() {
                    let gone = o.participants.iter().filter(|p| !s.participants.iter().any(|q| q.id == p.id)).cloned().collect();
                    let renamed = was.filter(|r| r.was_name != r.name).map(|r| r.was_name.clone());
                    let moved = was.filter(|r| r.was_file != r.file).map(|r| r.was_file.clone());
                    out.push(StoryDiff { id: s.id.clone(), title: s.title.clone(), mark: Mark::Changed, steps, story: None, gone, renamed, moved });
                }
            }
        }
    }
    for o in old.stories.iter().filter(|o| !new.stories.iter().any(|s| s.id == o.id)) {
        out.push(StoryDiff { id: o.id.clone(), title: o.title.clone(), mark: Mark::Removed, steps: vec![], story: Some(o.clone()), gone: vec![], renamed: None, moved: None });
    }
    out
}

pub fn diff_story(old: &Story, new: &Story) -> Vec<StepDiff> {
    let mut out = vec![];
    diff_list(&old.items, &new.items, None, "", &HashSet::new(), &mut out);
    out
}

/// The old reading with the ids renamed functions have now, and which of its stories are the new stories that start
/// in a renamed function (new story id → that rename).
fn with_new_ids<'r>(old: &ProjectModel, new: &ProjectModel, renamed: &'r [Renamed]) -> (ProjectModel, HashMap<String, &'r Renamed>) {
    let mut old = old.clone();
    if renamed.is_empty() {
        return (old, HashMap::new());
    }
    let ids: HashMap<&str, &str> = renamed.iter().map(|r| (r.was.as_str(), r.id.as_str())).collect();
    // the one story a function starts, in a reading
    let started = |m: &ProjectModel, id: &str| -> Option<String> {
        let t = m.index.iter().find(|t| t.id == id)?;
        let starts: HashSet<&str> = t.uses.iter().filter(|u| u.how == UseKind::Starts).map(|u| u.story.as_str()).collect();
        (starts.len() == 1).then(|| starts.into_iter().next().unwrap().to_string())
    };
    let mut stories: HashMap<String, (String, &Renamed)> = HashMap::new();
    for r in renamed {
        if let (Some(a), Some(b)) = (started(&old, &r.was), started(new, &r.id)) {
            stories.insert(a, (b, r));
        }
    }
    let swap = |id: &mut String| {
        if let Some(n) = ids.get(id.as_str()) {
            *id = n.to_string();
        }
    };
    fn items(list: &mut [Item], swap: &dyn Fn(&mut String)) {
        for i in list {
            match i {
                Item::Call(c) => {
                    swap(&mut c.from);
                    swap(&mut c.to);
                }
                Item::Return { from, to, .. } => {
                    swap(from);
                    swap(to);
                }
                Item::Own { at, .. } => swap(at),
                Item::Frame { .. } => {}
            }
            for (_, l) in i.lists_mut() {
                items(l, swap);
            }
        }
    }
    let mut was = HashMap::new();
    for s in &mut old.stories {
        if let Some((id, r)) = stories.get(&s.id) {
            s.id = id.clone();
            was.insert(id.clone(), *r);
        }
        s.participants.iter_mut().for_each(|p| swap(&mut p.id));
        items(&mut s.items, &swap);
    }
    (old, was)
}

enum Op {
    Same(usize, usize),
    Del(usize),
    Add(usize),
}

/// Aligns two lists of sibling steps by signature: removals come before additions in the same place, as in git.
fn align(a: &[Item], b: &[Item]) -> Vec<Op> {
    let (sa, sb): (Vec<String>, Vec<String>) = (a.iter().map(Item::signature).collect(), b.iter().map(Item::signature).collect());
    let (n, m) = (a.len(), b.len());
    // l[i][j]: the longest common subsequence of a[i..] and b[j..]
    let mut l = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            l[i][j] = if sa[i] == sb[j] { l[i + 1][j + 1] + 1 } else { l[i + 1][j].max(l[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut ops) = (0, 0, vec![]);
    while i < n || j < m {
        if i < n && j < m && sa[i] == sb[j] {
            ops.push(Op::Same(i, j));
            i += 1;
            j += 1;
        } else if j == m || (i < n && l[i + 1][j] >= l[i][j + 1]) {
            ops.push(Op::Del(i));
            i += 1;
        } else {
            ops.push(Op::Add(j));
            j += 1;
        }
    }
    ops
}

/// `ren`: the ids of functions renamed since; a call to one is the same call even though its words changed.
fn diff_list(old: &[Item], new: &[Item], parent: Option<&str>, list: &str, ren: &HashSet<String>, out: &mut Vec<StepDiff>) {
    let ops = align(old, new);
    let mut last_new: Option<String> = None;
    let mut k = 0;
    while k < ops.len() {
        if let Op::Same(i, j) = ops[k] {
            diff_pair(&old[i], &new[j], ren, out);
            last_new = Some(new[j].id().to_string());
            k += 1;
            continue;
        }
        // a run of removals and additions between two matched steps
        let (mut dels, mut adds) = (vec![], vec![]);
        while k < ops.len() {
            match ops[k] {
                Op::Del(i) => dels.push(i),
                Op::Add(j) => adds.push(j),
                Op::Same(..) => break,
            }
            k += 1;
        }
        // a removed and an added step are one edited step when they are of the same kin and their code shares names;
        // pairs keep their order and share as many names as they can
        let pair_of = pairs(&dels.iter().map(|&i| &old[i]).collect::<Vec<_>>(), &adds.iter().map(|&j| &new[j]).collect::<Vec<_>>(), ren);
        let paired: Vec<bool> = (0..dels.len()).map(|d| pair_of.contains(&Some(d))).collect();
        let pair_of: Vec<Option<usize>> = pair_of.into_iter().map(|d| d.map(|d| dels[d])).collect();
        let anchor = last_new.clone();
        for (d, &i) in dels.iter().enumerate() {
            if !paired[d] {
                out.push(StepDiff {
                    id: format!("was:{}", old[i].id()),
                    mark: Mark::Removed,
                    was: None,
                    item: Some(was_item(&old[i])),
                    parent: parent.map(str::to_string),
                    list: (!list.is_empty()).then(|| list.to_string()),
                    after: anchor.clone(),
                });
            }
        }
        for (x, &j) in adds.iter().enumerate() {
            match pair_of[x] {
                Some(i) => diff_pair(&old[i], &new[j], ren, out),
                None => out.push(StepDiff { id: new[j].id().to_string(), mark: Mark::Added, was: None, item: None, parent: None, list: None, after: None }),
            }
            last_new = Some(new[j].id().to_string());
        }
    }
}

/// The names in a step's words (identifiers and other words), to tell whether two steps are about the same thing.
fn names(it: &Item) -> std::collections::HashSet<&str> {
    it.label().split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')).filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit())).collect()
}

/// Which removed step each added step pairs with (by position in `dels`): only steps of one kin that share names (or
/// call a function renamed since), in order, sharing as many names in all as possible.
fn pairs(dels: &[&Item], adds: &[&Item], ren: &HashSet<String>) -> Vec<Option<usize>> {
    let (p, q) = (dels.len(), adds.len());
    let to_renamed = |it: &Item| matches!(it, Item::Call(c) if ren.contains(&c.to));
    let score = |i: usize, j: usize| {
        if dels[i].kin() != adds[j].kin() {
            return 0;
        }
        let shared = names(dels[i]).intersection(&names(adds[j])).count();
        if shared == 0 && to_renamed(adds[j]) { 1 } else { shared }
    };
    // best[i][j]: the most names shared by pairs among dels[i..] and adds[j..]
    let mut best = vec![vec![0usize; q + 1]; p + 1];
    for i in (0..p).rev() {
        for j in (0..q).rev() {
            let s = score(i, j);
            best[i][j] = best[i + 1][j].max(best[i][j + 1]).max(if s > 0 { s + best[i + 1][j + 1] } else { 0 });
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![None; q]);
    while i < p && j < q {
        let s = score(i, j);
        if s > 0 && best[i][j] == s + best[i + 1][j + 1] {
            out[j] = Some(i);
            i += 1;
            j += 1;
        } else if best[i][j] == best[i + 1][j] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Two steps taken to be the same one: changed if their code differs, and then what they hold, list by list.
fn diff_pair(old: &Item, new: &Item, ren: &HashSet<String>, out: &mut Vec<StepDiff>) {
    if differs(old, new) {
        let was = Was { label: old.label().to_string(), detail: old.detail().map(str::to_string) };
        out.push(StepDiff { id: new.id().to_string(), mark: Mark::Changed, was: Some(was), item: None, parent: None, list: None, after: None });
    }
    let (ol, nl) = (old.lists(), new.lists());
    for (name, items) in &nl {
        let before = ol.iter().find(|(n, _)| n == name).map(|(_, l)| *l).unwrap_or(&[]);
        diff_list(before, items, Some(new.id()), name, ren, out);
    }
    for (name, items) in ol.iter().filter(|(n, _)| !nl.iter().any(|(m, _)| m == n)) {
        diff_list(items, &[], Some(new.id()), name, ren, out);
    }
}

/// Whether a step's code differs. For a call or a return the code behind it is one expression, so all of it counts.
/// Other steps can stand for a whole block (a function declared in place, a `try`, several statements folded into
/// one row) whose inside other steps already show, so only what the row says counts.
fn differs(a: &Item, b: &Item) -> bool {
    let squeeze = |s: Option<&str>| s.map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "));
    if squeeze(Some(a.label())) != squeeze(Some(b.label())) {
        return true;
    }
    match (a, b) {
        (Item::Call(x), Item::Call(y)) => {
            squeeze(a.detail()) != squeeze(b.detail())
                || x.effect != y.effect
                || x.table != y.table
                || x.opens != y.opens
                || x.story != y.story
                || x.link.kind != y.link.kind
                || x.target.as_ref().map(|t| &t.file) != y.target.as_ref().map(|t| &t.file)
        }
        (Item::Return { .. }, Item::Return { .. }) => squeeze(a.detail()) != squeeze(b.detail()),
        _ => false,
    }
}

/// A removed step as it was, its ids and those of what it held marked as old so they never meet a new step's id.
fn was_item(it: &Item) -> Item {
    let mut it = it.clone();
    fn mark(it: &mut Item) {
        *it.id_mut() = format!("was:{}", it.id());
        for (_, l) in it.lists_mut() {
            l.iter_mut().for_each(mark);
        }
    }
    mark(&mut it);
    it
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Call, Link, LinkKind, Ref, SelfKind, name_steps};

    fn at() -> Ref {
        Ref { file: "a.ts".into(), from: 1, to: 1 }
    }
    fn call(to: &str, label: &str, body: Option<Vec<Item>>) -> Item {
        Item::Call(Call {
            id: String::new(),
            from: "screen".into(),
            to: to.into(),
            label: label.into(),
            detail: None,
            link: Link { kind: LinkKind::Exact, how: String::new() },
            effect: None,
            at: at(),
            target: None,
            body,
            note: None,
            opens: None,
            story: None,
            table: None,
            touches: None,
            unused: false,
            into: None,
            ends: vec![],
            changes: vec![],
        })
    }
    fn own(label: &str) -> Item {
        Item::Own { id: String::new(), at: "screen".into(), kind: SelfKind::State, label: label.into(), detail: None, place: at() }
    }
    fn named(mut items: Vec<Item>) -> Vec<Item> {
        name_steps(&mut items);
        items
    }
    fn marks(old: Vec<Item>, new: Vec<Item>) -> Vec<(Mark, String)> {
        let (old, new) = (named(old), named(new));
        let mut out = vec![];
        diff_list(&old, &new, None, "", &HashSet::new(), &mut out);
        out.into_iter().map(|d| (d.mark, d.item.as_ref().map(|i| i.label().to_string()).or(d.was.map(|w| w.label)).unwrap_or_else(|| find(&new, &d.id)))).collect()
    }
    fn find(items: &[Item], id: &str) -> String {
        for i in items {
            if i.id() == id {
                return i.label().to_string();
            }
            for (_, l) in i.lists() {
                let f = find(l, id);
                if !f.is_empty() {
                    return f;
                }
            }
        }
        String::new()
    }

    #[test]
    fn a_step_added_before_others_leaves_their_ids() {
        let old = named(vec![call("api", "load()", None), own("setBusy(false)")]);
        let new = named(vec![own("setBusy(true)"), call("api", "load()", None), own("setBusy(false)")]);
        assert_eq!(old[0].id(), new[1].id());
        assert_ne!(old[1].id(), new[2].id(), "two steps of one signature are told apart by their order");
        assert_eq!(marks(vec![call("api", "load()", None)], vec![own("setBusy(true)"), call("api", "load()", None)]), vec![(Mark::Added, "setBusy(true)".into())]);
    }

    #[test]
    fn new_arguments_change_a_step_without_renaming_it() {
        let old = named(vec![call("db", r#"from("books").insert({ title })"#, None)]);
        let new = named(vec![call("db", r#"from("books").insert({ title, isbn })"#, None)]);
        assert_eq!(old[0].id(), new[0].id());
        assert_eq!(marks(old, new), vec![(Mark::Changed, r#"from("books").insert({ title })"#.into())]);
    }

    #[test]
    fn removed_steps_keep_their_place_and_what_they_held() {
        let old = vec![call("api", "POST /books", Some(vec![call("auth", "getUser()", None), call("db", r#"from("books").insert()"#, None)]))];
        let new = vec![call("api", "POST /books", Some(vec![call("db", r#"from("books").insert()"#, None), call("db", r#"from("audit").insert()"#, None)]))];
        let (o, n) = (named(old), named(new));
        let mut out = vec![];
        diff_list(&o, &n, None, "", &HashSet::new(), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].mark, out[0].parent.as_deref(), out[0].after.as_deref()), (Mark::Removed, Some(n[0].id()), None));
        assert!(out[0].id.starts_with("was:"));
        assert_eq!((out[1].mark, find(&n, &out[1].id)), (Mark::Added, r#"from("audit").insert()"#.to_string()));
    }

    #[test]
    fn a_call_to_another_place_is_removed_and_added() {
        assert_eq!(marks(vec![call("auth", "getUser()", None)], vec![call("api", "load()", None)]), vec![(Mark::Removed, "getUser()".into()), (Mark::Added, "load()".into())]);
    }

    #[test]
    fn edits_in_one_place_pair_by_shared_names() {
        let frame = |c: &str| Item::Frame { id: String::new(), kind: crate::model::FrameKind::Alt, label: c.into(), note: None, at: at(), items: vec![], els: None };
        let old = vec![call("auth", "getUser()", None), frame("!user"), own("const body"), frame("!body.title || !body.author")];
        let new = vec![own("const user, body"), frame("!body.title")];
        assert_eq!(
            marks(old, new),
            vec![(Mark::Removed, "getUser()".into()), (Mark::Removed, "!user".into()), (Mark::Changed, "const body".into()), (Mark::Changed, "!body.title || !body.author".into())]
        );
    }

    #[test]
    fn a_renamed_function_keeps_its_story_and_the_calls_to_it() {
        use crate::model::{Group, PartKind, Stats, Thing, Trigger, Use};
        // `run` calls `load`; both are renamed in one edit, to `main` and `fetch`
        let model = |root: &str, callee: &str| {
            let (r, c) = (format!("a.py#{root}"), format!("a.py#{callee}"));
            let story_id = format!("function:{r}");
            let part = |id: &str, name: &str| Participant { id: id.into(), name: name.into(), kind: PartKind::Function, group: Group::Server, sub: None, at: None };
            let mut items = vec![call(&c, &format!("{callee}()"), None)];
            if let Item::Call(x) = &mut items[0] {
                x.from = r.clone();
            }
            let story = Story { id: story_id.clone(), title: format!("{root} runs"), trigger: Trigger::default(), also: vec![], participants: vec![part(&r, root), part(&c, callee)], items: named(items), signals: vec![], gaps: vec![], coverage: vec![] };
            let thing = |id: &str, name: &str, from: usize, how: UseKind| Thing {
                id: id.into(),
                name: name.into(),
                kind: PartKind::Function,
                group: Group::Server,
                sub: None,
                at: Some(Ref { file: "a.py".into(), from, to: from + 1 }),
                within: None,
                uses: vec![Use { story: story_id.clone(), step: None, drawn: false, how, at: at(), from: None }],
            };
            let file = format!("def {root}():\n    {callee}()\n\ndef {callee}():\n    return 1\n");
            ProjectModel {
                name: "p".into(),
                stories: vec![story],
                index: vec![thing(&r, root, 1, UseKind::Starts), thing(&c, callee, 4, UseKind::Calls)],
                files: [("a.py".to_string(), file)].into_iter().collect(),
                stats: Stats { ms: 0, llm_calls: 0 },
                setup: vec![],
                missing: vec![],
            }
        };
        let d = diff(&model("run", "load"), &model("main", "fetch"));
        assert_eq!(d.len(), 1, "one story, changed: not one removed and one added");
        assert_eq!((d[0].mark, d[0].id.as_str(), d[0].renamed.as_deref()), (Mark::Changed, "function:a.py#main", Some("run")));
        assert_eq!(d[0].steps.iter().map(|s| (s.mark, s.was.as_ref().map(|w| w.label.as_str()))).collect::<Vec<_>>(), [(Mark::Changed, Some("load()"))]);
    }

    #[test]
    fn the_same_steps_give_no_difference() {
        let l = || vec![own("setBusy(true)"), call("api", "POST /books", Some(vec![call("db", "from(\"books\").insert()", None)]))];
        assert!(marks(l(), l()).is_empty());
    }
}
