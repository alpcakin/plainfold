// What a function is known by everywhere in the model: its file and the names it is written inside,
// `services/user.py#UserService.save`, `app/page.tsx#Page.onSave`. The index (`reach.rs`) collects uses under this
// id across stories, so two functions must never share one, and an edit elsewhere should not change it.
// A function with no name is known by the call it is written in, with the words given there, if any:
// `describe('totals').it('adds two')`, `Page.useEffect(…)`; elsewhere it is `anonymous`. The same name twice in one
// place (a getter and its setter, one definition per branch of an `if`, two effects in one component) is told apart
// by its order there: `…#Item.price~2`.
use crate::lang::{Expr, Lang};
use crate::source::one_line;
use std::collections::HashMap;
use tree_sitter::Node;

/// A node's place in its file: (start byte, end byte, kind).
pub type Key = (usize, usize, u16);

pub fn key(n: Node) -> Key {
    (n.start_byte(), n.end_byte(), n.kind_id())
}

/// The id of every function and class in a file.
pub fn of_file(rel: &str, l: &Lang, text: &str, root: Node) -> HashMap<Key, String> {
    let mut paths: HashMap<Key, String> = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    // source order, so the first of two with the same name keeps the plain one
    let mut stack = vec![(root, String::new())];
    while let Some((n, outer)) = stack.pop() {
        let mut inner = outer.clone();
        if let Some(own) = segment(l, n, text) {
            let path = if outer.is_empty() { own } else { format!("{outer}.{own}") };
            let k = seen.entry(path.clone()).or_default();
            *k += 1;
            let path = if *k == 1 { path } else { format!("{path}~{k}") };
            paths.insert(key(n), path.clone());
            inner = path;
        }
        let mut c = n.walk();
        let kids: Vec<Node> = n.named_children(&mut c).collect();
        stack.extend(kids.into_iter().rev().map(|k| (k, inner.clone())));
    }
    paths.into_iter().map(|(k, p)| (k, format!("{rel}#{p}"))).collect()
}

/// What a node adds to the names of what is written inside it: a function or class its name (without one, the call
/// it is passed to), an object literal with a name its name (`const api = { load() {} }` → `api.load`), anything
/// else nothing.
fn segment(l: &Lang, n: Node, text: &str) -> Option<String> {
    let kind = n.kind();
    let named = || (l.function_name)(n).map(|x| text[x.byte_range()].to_string());
    if l.is_function(kind) || l.reach.classes.contains(&kind) {
        return Some(named().or_else(|| passed_to(l, n, text)).unwrap_or_else(|| "anonymous".into()));
    }
    if l.follow.object.is_some_and(|o| o.0 == kind) {
        return named();
    }
    None
}

/// `it('adds two', () => …)` → `it('adds two')`: the call a function is passed to, with the first string written
/// before it; `useEffect(…)` when there is none.
fn passed_to(l: &Lang, n: Node, text: &str) -> Option<String> {
    let call = n.parent()?.parent()?;
    l.call_field(call.kind())?;
    let Expr::Call { callee, args } = (l.expr)(call, text) else { return None };
    let at = args.iter().position(|a| l.unwrap(*a) == n)?;
    let words = args[..at].iter().find(|a| l.strings.nodes.contains(&a.kind())).map(|a| one_line(&text[a.byte_range()], 60));
    let callee = one_line(&text[callee.byte_range()], 40);
    Some(match words {
        Some(w) => format!("{callee}({w})"),
        None => format!("{callee}(…)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::LANGS;
    use tree_sitter::Parser;

    fn ids(lang: &str, code: &str) -> Vec<String> {
        let l = LANGS.iter().find(|l| l.id == lang).unwrap();
        let mut p = Parser::new();
        p.set_language(&(l.grammar)()).unwrap();
        let tree = p.parse(code, None).unwrap();
        let mut v: Vec<(Key, String)> = of_file("f", l, code, tree.root_node()).into_iter().collect();
        v.sort();
        v.into_iter().map(|(_, id)| id).collect()
    }

    #[test]
    fn methods_of_two_classes_are_two_things() {
        let code = "class A:\n    def save(self):\n        pass\n\nclass B:\n    def save(self):\n        pass\n\ndef save():\n    pass\n";
        assert_eq!(ids("python", code), ["f#A", "f#A.save", "f#B", "f#B.save", "f#save"]);
    }

    #[test]
    fn the_same_name_twice_in_one_place_is_counted() {
        let code = "class A:\n    @property\n    def x(self):\n        return 1\n\n    @x.setter\n    def x(self, v):\n        pass\n";
        assert_eq!(ids("python", code), ["f#A", "f#A.x", "f#A.x~2"]);
    }

    #[test]
    fn nested_and_unnamed_functions_are_known_by_where_they_are_written() {
        let code = "export function Page() {\n  const onSave = () => {};\n  run(() => {});\n  const api = { load() {}, keep: () => {} };\n}\nexport default () => {};\n";
        assert_eq!(ids("typescript", code), ["f#Page", "f#Page.onSave", "f#Page.run(…)", "f#Page.api", "f#Page.api.load", "f#Page.api.keep", "f#anonymous"]);
        let tests = "describe('totals', () => {\n  it('adds two', () => {});\n  it('adds none', () => {});\n});\n";
        assert_eq!(ids("typescript", tests), ["f#describe('totals')", "f#describe('totals').it('adds two')", "f#describe('totals').it('adds none')"]);
    }
}
