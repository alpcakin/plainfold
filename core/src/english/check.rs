// What a model's text must pass before it is shown (DECISIONS §7, §8, §9). Every rule reads only the text and the
// step's code; none knows a project. A text that fails is not shown: the step keeps its code.
use super::code::Symbol;
use crate::model::Effect;
use serde::{Deserialize, Serialize};

/// Text that passed, with the names copied from the code marked.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Said {
    pub text: String,
    /// where the names and values copied from the code are in `text`: start and end in UTF-16 units, as the view
    /// counts
    pub names: Vec<[usize; 2]>,
    /// names in the code that read like instructions, which the model was not shown
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub masked: Vec<String>,
}

/// What the code shows a step does to data, which the text must not contradict, and its code text.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// the step as the code writes it (its label)
    pub code: String,
    /// what the step itself does to a table
    pub effect: Option<Effect>,
    /// what is done to tables anywhere inside the call
    pub inside: Vec<Effect>,
}

/// The most words a text may have, placeholders counted as one.
const MAX_WORDS: usize = 12;

const DELETE: &[&str] = &["delet", "remov", "eras", "destroy", "drop", "purg", "wip"];
const WRITE: &[&str] = &["sav", "writ", "wrote", "insert", "updat", "creat", "add", "stor", "upsert", "record", "put", "edit", "modif", "post", "submit", "register", "set", "chang"];
const READ: &[&str] = &["read", "load", "fetch", "get", "got", "find", "found", "list", "look", "query", "queri", "select", "count", "retriev", "search", "check", "show"];
/// The endings that make a form of a stem (`delet` → delete, deletes, deleted, deleting, deletion).
const ENDINGS: &[&str] = &["", "s", "e", "es", "ed", "d", "ing", "ion", "ions", "al", "y", "ies", "ied", "n", "ten"];

/// A word is a form of one of the stems: `delete`, `deleting`, `dropped`; not `address` for `add`.
fn form_of(word: &str, stems: &[&str]) -> bool {
    stems.iter().any(|s| {
        let Some(rest) = word.strip_prefix(s) else { return false };
        // a doubled last letter before the ending: dropped, setting, adding
        let rest = match (rest.chars().next(), s.chars().last()) {
            (Some(a), Some(b)) if a == b && (rest.len() > 1) => &rest[1..],
            _ => rest,
        };
        ENDINGS.contains(&rest)
    })
}

pub fn check(raw: &str, symbols: &[Symbol], facts: &Facts) -> Result<Said, &'static str> {
    let t = raw.trim();
    let t = t.strip_suffix('.').unwrap_or(t).trim();
    if t.is_empty() {
        return Err("left empty");
    }
    // plain words and placeholders only: no code, digits, quotes, markup, links or characters outside ASCII
    let mut parts: Vec<Result<String, usize>> = vec![]; // a run of plain text, or a placeholder (index into symbols)
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            let close = chars[i..].iter().position(|&x| x == '}').map(|p| i + p).ok_or("not plain text")?;
            let n: usize = chars[i + 1..close].iter().collect::<String>().parse().map_err(|_| "not plain text")?;
            if n == 0 || n > symbols.len() {
                return Err("a placeholder that is not in the code");
            }
            parts.push(Err(n - 1));
            i = close + 1;
            continue;
        }
        let ok = c.is_ascii_alphabetic()
            || c == ' '
            || c == ','
            || c == '-'
            || c == '/'
            || (c == '\'' && i > 0 && chars[i - 1].is_ascii_alphabetic() && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic()));
        if !ok {
            return Err(if "_.()[]=<>!&|+*%".contains(c) {
                "code instead of words"
            } else if c.is_ascii_digit() {
                "a number the model wrote"
            } else if "\"`“”‘’".contains(c) || c == '\'' {
                "a quotation the model wrote"
            } else {
                "not plain text"
            });
        }
        match parts.last_mut() {
            Some(Ok(s)) => s.push(c),
            _ => parts.push(Ok(c.to_string())),
        }
        i += 1;
    }
    let words: Vec<String> = parts
        .iter()
        .filter_map(|p| p.as_ref().ok())
        .flat_map(|s| s.split([' ', ',', '/']).filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>())
        .collect();
    let placeholders = parts.iter().filter(|p| p.is_err()).count();
    if words.len() + placeholders > MAX_WORDS {
        return Err("too long");
    }
    // a name from the code written out instead of by its placeholder: plainfold inserts names, the model never does
    if words.iter().any(|w| w.chars().zip(w.chars().skip(1)).any(|(a, b)| a.is_ascii_lowercase() && b.is_ascii_uppercase())) {
        return Err("a name written without its placeholder");
    }
    let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
    // the arrow already says that something is called; the text has to say what for
    if !matches!(parts.first(), Some(Err(_))) && lower.first().is_some_and(|w| ["call", "calls", "invoke", "invokes", "execute", "executes"].contains(&w.as_str())) {
        return Err("says only that something is called");
    }
    // the code's own words read back ("not user", "catch") say nothing the code does not
    let letters = |s: &str| s.to_ascii_lowercase().split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    if placeholders == 0 && !facts.code.is_empty() && letters(t) == letters(&facts.code) {
        return Err("repeats the code");
    }
    let says = |stems: &[&str]| lower.iter().any(|w| form_of(w, stems));
    // what the text says is done to data must be what the code does (DECISIONS §9)
    match facts.effect {
        Some(Effect::Deletes) if !says(DELETE) => return Err("does not say it deletes, and the code does"),
        Some(Effect::Writes) if says(DELETE) => return Err("says it deletes, and the code writes"),
        Some(Effect::Writes) if says(READ) && !says(WRITE) => return Err("reads as a read, and the code writes"),
        Some(Effect::Reads) if says(DELETE) || says(WRITE) => return Err("says it changes data, and the code only reads"),
        _ => {}
    }
    if facts.effect.is_none() && !facts.inside.is_empty() && says(DELETE) && !facts.inside.contains(&Effect::Deletes) {
        return Err("says it deletes, and nothing inside it does");
    }
    // the names go in, between the plain words with their spaces squeezed
    let mut text = String::new();
    let mut names = vec![];
    let mut masked = vec![];
    let units = |s: &str| s.encode_utf16().count();
    let last = parts.len().saturating_sub(1);
    for (k, p) in parts.into_iter().enumerate() {
        match p {
            Ok(s) => {
                let mut s = squeeze(&s);
                if k == 0 {
                    s = s.trim_start().to_string();
                }
                if k == last {
                    s = s.trim_end().to_string();
                }
                text.push_str(&s);
            }
            Err(i) => {
                let s = &symbols[i];
                let at = units(&text);
                text.push_str(&s.text);
                names.push([at, units(&text)]);
                if s.masked && !masked.contains(&s.text) {
                    masked.push(s.text.clone());
                }
            }
        }
    }
    Ok(Said { text, names, masked })
}

/// Runs of spaces as one.
fn squeeze(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if !(c == ' ' && out.ends_with(' ')) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syms(xs: &[&str]) -> Vec<Symbol> {
        xs.iter().map(|x| Symbol { text: x.to_string(), masked: false }).collect()
    }

    #[test]
    fn names_go_in_by_placeholder() {
        let s = check("save the new book in {2}.", &syms(&["insert", "books"]), &Facts { effect: Some(Effect::Writes), inside: vec![], ..Default::default() }).unwrap();
        assert_eq!(s.text, "save the new book in books");
        assert_eq!(s.names, [[21, 26]]);
    }

    #[test]
    fn code_and_facts_are_refused() {
        let f = Facts::default();
        let s = syms(&["getUser"]);
        assert_eq!(check("call getUser", &s, &f), Err("a name written without its placeholder"));
        assert_eq!(check("wait 20 seconds", &s, &f), Err("a number the model wrote"));
        assert_eq!(check("say \"hi\"", &s, &f), Err("a quotation the model wrote"));
        assert_eq!(check("see https://x.io", &s, &f), Err("not plain text"));
        assert_eq!(check("when len(items) == 0", &s, &f), Err("code instead of words"));
        assert_eq!(check("check the user_id", &s, &f), Err("code instead of words"));
        assert_eq!(check("call the database function", &s, &f), Err("says only that something is called"));
        let not_user = Facts { code: "not user".into(), ..Default::default() };
        assert_eq!(check("Not user", &s, &not_user), Err("repeats the code"));
        assert!(check("no one is signed in", &s, &not_user).is_ok());
        assert_eq!(check("use {3}", &s, &f), Err("a placeholder that is not in the code"));
        assert_eq!(check("", &s, &f), Err("left empty"));
        assert_eq!(check("get the user's {1}", &s, &f).unwrap().text, "get the user's getUser");
    }

    #[test]
    fn text_must_not_contradict_what_the_code_does_to_data() {
        let s = syms(&["books"]);
        let reads = Facts { effect: Some(Effect::Reads), inside: vec![], ..Default::default() };
        let deletes = Facts { effect: Some(Effect::Deletes), inside: vec![], ..Default::default() };
        let writes = Facts { effect: Some(Effect::Writes), inside: vec![], ..Default::default() };
        assert!(check("load the books", &s, &reads).is_ok());
        assert!(check("delete old books", &s, &reads).is_err());
        assert!(check("remove the book from {1}", &s, &deletes).is_ok());
        assert!(check("look up the book", &s, &deletes).is_err());
        assert!(check("save the book", &s, &writes).is_ok());
        assert!(check("load the book", &s, &writes).is_err());
        let inside = Facts { effect: None, inside: vec![Effect::Reads, Effect::Writes], ..Default::default() };
        assert!(check("delete the account", &s, &inside).is_err());
        assert!(check("save the account", &s, &inside).is_ok());
    }
}
