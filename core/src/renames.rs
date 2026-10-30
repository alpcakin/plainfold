// Functions renamed or moved between two readings. A function's id is its file and its names (`story/ids.rs`), so a
// rename or a move gives it a new id. A function whose id is gone and one whose id is new are the same function when
// their code is the same once their own names are left out (as `git` finds a renamed file by its content). Only a
// pair that matches one to one counts; a function renamed and edited at once, or two with the same code, stay
// removed and added. A function that calls one renamed in the same edit is compared with that call renamed too.
use crate::model::{ProjectModel, Thing};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Renamed {
    /// the id, name and file it had
    pub was: String,
    #[serde(rename = "wasName")]
    pub was_name: String,
    #[serde(rename = "wasFile")]
    pub was_file: String,
    pub id: String,
    pub name: String,
    pub file: String,
}

/// Every function of `old` renamed or moved in `new`.
pub fn renames(old: &ProjectModel, new: &ProjectModel) -> Vec<Renamed> {
    let in_old: HashSet<&str> = old.index.iter().map(|t| t.id.as_str()).collect();
    let in_new: HashSet<&str> = new.index.iter().map(|t| t.id.as_str()).collect();
    let mut out: Vec<Renamed> = vec![];
    // again while renames are found: the next round reads the old code with the names found so far renamed
    loop {
        let known: HashSet<&str> = out.iter().flat_map(|r| [r.was.as_str(), r.id.as_str()]).collect();
        let words: Vec<(&str, &str)> = out.iter().filter(|r| r.was_name != r.name).map(|r| (r.was_name.as_str(), r.name.as_str())).collect();
        let mut by: HashMap<String, (Vec<&Thing>, Vec<&Thing>)> = HashMap::new();
        for t in old.index.iter().filter(|t| !in_new.contains(t.id.as_str()) && !known.contains(t.id.as_str())) {
            if let Some(f) = fingerprint(old, t, &words) {
                by.entry(f).or_default().0.push(t);
            }
        }
        for t in new.index.iter().filter(|t| !in_old.contains(t.id.as_str()) && !known.contains(t.id.as_str())) {
            if let Some(f) = fingerprint(new, t, &[]) {
                by.entry(f).or_default().1.push(t);
            }
        }
        let found: Vec<Renamed> = by
            .into_values()
            .filter_map(|(gone, came)| match (gone.as_slice(), came.as_slice()) {
                ([a], [b]) => Some(Renamed {
                    was: a.id.clone(),
                    was_name: a.name.clone(),
                    was_file: a.at.as_ref().map(|r| r.file.clone()).unwrap_or_default(),
                    id: b.id.clone(),
                    name: b.name.clone(),
                    file: b.at.as_ref().map(|r| r.file.clone()).unwrap_or_default(),
                }),
                _ => None,
            })
            .collect();
        if found.is_empty() {
            break;
        }
        out.extend(found);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// A function's code with its own name left out and whitespace squeezed: what stays when it is only renamed, moved,
/// or moved into a class. `words`: names renamed since, written as they are now. None for what has no code of the
/// project's own.
fn fingerprint(m: &ProjectModel, t: &Thing, words: &[(&str, &str)]) -> Option<String> {
    let at = t.at.as_ref().filter(|_| t.within.is_none())?;
    let text = m.files.get(&at.file)?;
    let lines: Vec<&str> = text.lines().skip(at.from.saturating_sub(1)).take(at.to + 1 - at.from).collect();
    // its own name, but not where it is another thing's member (a route `create` calling `store.create`)
    let mut code = without_word(&lines.join("\n"), &t.name, "·", false);
    for (was, now) in words {
        code = without_word(&code, was, now, true);
    }
    let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
    (!code.is_empty()).then_some(code)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// `code` with every whole-word `name` written as `with`; after a `.` too when `members`. A name that is not one word
/// (an unnamed function known by the call it is passed to) is not in its own code.
fn without_word(code: &str, name: &str, with: &str, members: bool) -> String {
    if name.is_empty() || !name.chars().all(is_word) {
        return code.to_string();
    }
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + name.len()..].chars().next();
        out.push_str(&rest[..i]);
        if before.is_some_and(is_word) || after.is_some_and(is_word) || (!members && before == Some('.')) {
            out.push_str(name);
        } else {
            out.push_str(with);
        }
        rest = &rest[i + name.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Group, PartKind, Ref, Stats};

    fn thing(id: &str, name: &str, file: &str, from: usize, to: usize) -> Thing {
        Thing { id: id.into(), name: name.into(), kind: PartKind::Function, group: Group::Server, sub: None, at: Some(Ref { file: file.into(), from, to }), within: None, uses: vec![] }
    }
    fn model(files: &[(&str, &str)], index: Vec<Thing>) -> ProjectModel {
        ProjectModel {
            name: "p".into(),
            stories: vec![],
            index,
            files: files.iter().map(|(f, t)| (f.to_string(), t.to_string())).collect(),
            stats: Stats { ms: 0, llm_calls: 0 },
            setup: vec![],
            missing: vec![],
        }
    }

    #[test]
    fn the_same_code_under_a_new_name_or_in_another_file_is_the_same_function() {
        let old = model(&[("a.py", "def load(x):\n    return load(x - 1)\n\ndef keep():\n    pass\n")], vec![thing("a.py#load", "load", "a.py", 1, 2), thing("a.py#keep", "keep", "a.py", 4, 5)]);
        let new = model(
            &[("a.py", "def keep():\n    pass\n"), ("b.py", "class Store:\n    def fetch(x):\n        return fetch(x - 1)\n")],
            vec![thing("a.py#keep", "keep", "a.py", 1, 2), thing("b.py#Store.fetch", "fetch", "b.py", 2, 3)],
        );
        let r = renames(&old, &new);
        assert_eq!(r.iter().map(|r| (r.was.as_str(), r.id.as_str(), r.was_file.as_str(), r.file.as_str())).collect::<Vec<_>>(), [("a.py#load", "b.py#Store.fetch", "a.py", "b.py")]);
    }

    #[test]
    fn renamed_and_edited_or_not_one_to_one_is_not_a_rename() {
        let old = model(&[("a.py", "def a():\n    pass\n\ndef b():\n    pass\n\ndef c(x):\n    return x\n")], vec![thing("a.py#a", "a", "a.py", 1, 2), thing("a.py#b", "b", "a.py", 4, 5), thing("a.py#c", "c", "a.py", 7, 8)]);
        let new = model(&[("a.py", "def d():\n    pass\n\ndef e():\n    pass\n\ndef f(x):\n    return x + 1\n")], vec![thing("a.py#d", "d", "a.py", 1, 2), thing("a.py#e", "e", "a.py", 4, 5), thing("a.py#f", "f", "a.py", 7, 8)]);
        assert!(renames(&old, &new).is_empty());
    }

    #[test]
    fn only_whole_words_are_the_name() {
        assert_eq!(without_word("load(x); loader(); reload(); obj.load", "load", "·", false), "·(x); loader(); reload(); obj.load");
        assert_eq!(without_word("obj.load()", "load", "fetch", true), "obj.fetch()");
        assert_eq!(without_word("it('adds')", "describe('x')", "·", false), "it('adds')");
    }
}
