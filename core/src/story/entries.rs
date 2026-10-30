// Entry points no adapter names: the functions something outside the project starts (a framework, a command, a
// test runner, another program). Facts from the code only. A function is one when nothing in the project calls it,
// and how it is reached is kept with it where the code shows it: a decorator on it, the call it is passed to, the
// place it is used as a value. Where an adapter starts a story at a function, that function is left to the adapter,
// which names it better.
//
// A function passed to a call outside the project whose result is not used (`scheduler.add_job(job, …)`,
// `addEventListener("load", f)`) is registered there: it is an entry point even where the project calls it too
// (a test that runs the job directly). A call whose result is used may only wrap the function (`memo(f)`), so
// passing it there counts only when nothing calls it.
//
// Only functions written at the top of a file or of a class are candidates. A function written inside another one
// belongs to that one's story (a callback, a handler that runs later), except an unnamed function passed to a call
// at the top of a file (`app.get("/items", (req, res) => …)`): nothing else would ever show it.
use super::World;
use crate::lang::{Expr, Lang, Stmt};
use crate::resolve::{Target, callee_name};
use crate::source::{At, NodeKey, one_line};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node;

pub enum How<'w> {
    /// decorators are written on it (`@app.get("/items")`): the first one
    Decorated(At<'w>),
    /// it is passed to a call, by name or written in the call: the call
    Passed(At<'w>),
    /// it is used as a value (stored, returned, exported), never called: the first use
    Value(At<'w>),
    /// nothing in the project names it again
    Uncalled,
}

impl How<'_> {
    pub fn kind(&self) -> &'static str {
        match self {
            How::Decorated(_) => "decorated",
            How::Passed(_) => "passed",
            How::Value(_) => "value",
            How::Uncalled => "uncalled",
        }
    }
    pub fn at(&self) -> Option<At<'_>> {
        match self {
            How::Decorated(a) | How::Passed(a) | How::Value(a) => Some(*a),
            How::Uncalled => None,
        }
    }
}

pub struct Entry<'w> {
    pub decl: At<'w>,
    /// its name, or for an unnamed function the call it is written in (`app.get("/items", …)`)
    pub name: String,
    pub named: bool,
    pub how: How<'w>,
    /// the classes outside the project its class extends, as written: code there may call it
    pub extends: Vec<String>,
}

/// What `find` returns: the functions, and the files whose top-level code does work of the project's own.
pub struct Found<'w> {
    pub entries: Vec<Entry<'w>>,
    /// a file and its top-level statements (definitions and imports left out), where that code calls one of the
    /// project's functions or hands one to something: it runs when the file is run or first loaded
    pub files: Vec<(usize, Vec<Node<'w>>)>,
}

struct Candidate<'w> {
    decl: At<'w>,
    name: Option<Node<'w>>,
    class: Option<Node<'w>>,
    /// for an unnamed function: the call it is passed to
    passed_to: Option<Node<'w>>,
}

/// The entry points of the project's own code, in file and source order. `covered`: functions an adapter already
/// starts a story at.
pub fn find<'w>(w: &World<'w>, covered: &HashSet<NodeKey>) -> Found<'w> {
    let p = w.p;
    let res = w.res;
    let ready: Vec<usize> = (0..p.files.len()).filter(|&i| res.ready(i)).collect();

    // 1. the candidates
    let mut cands: Vec<Candidate> = vec![];
    // what a tool wrote is no entry point of its own: code of it that nothing calls is no story
    for &i in ready.iter().filter(|&&i| !p.files[i].generated()) {
        let f = &p.files[i];
        let l = f.lang;
        let mut stack = vec![(f.tree.root_node(), None::<Node>)];
        while let Some((n, class)) = stack.pop() {
            if l.is_function(n.kind()) {
                let name = (l.function_name)(n);
                let decl = At { file: i, node: n };
                // an unnamed function counts only where the call it is written in registers it
                let passed_to = if name.is_none() { passed_in(l, n, &f.text).filter(|c| registers(w, i, *c)) } else { None };
                let implicit = name.is_some_and(|x| (l.reach.implicit)(f.text_of(x)));
                if (name.is_some() || passed_to.is_some()) && !implicit && !covered.contains(&decl.key()) {
                    cands.push(Candidate { decl, name, class, passed_to });
                }
                continue; // what is written inside belongs to its story
            }
            let class = if l.reach.classes.contains(&n.kind()) { Some(n) } else { class };
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c).map(|k| (k, class)));
        }
    }
    let by_key: HashMap<NodeKey, usize> = cands.iter().enumerate().map(|(k, c)| (c.decl.key(), k)).collect();

    // 2. what the project calls: every call, element and bare decorator, wherever it is written
    let mut callees: Vec<(usize, Node)> = vec![];
    for &i in &ready {
        let f = &p.files[i];
        let l = f.lang;
        let mut stack = vec![f.tree.root_node()];
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
            let field = l.call_field(n.kind()).or_else(|| l.element_field(n.kind()));
            if let Some(callee) = field.and_then(|fd| n.child_by_field_name(fd)) {
                callees.push((i, callee));
            } else if l.reach.decorators.contains(&n.kind()) {
                // `@name` applies `name` to what it stands on: a call (`@name(…)` is a call already)
                if let Some(e) = n.named_child(0).filter(|e| l.call_field(e.kind()).is_none()) {
                    callees.push((i, e));
                }
            }
        }
    }
    res.prefetch(&callees.iter().filter_map(|&(i, c)| Some((i, callee_name(c, p.files[i].lang)?))).collect::<Vec<_>>());
    let callee_names: HashSet<(usize, usize)> = callees.iter().filter_map(|&(i, c)| Some((i, callee_name(c, p.files[i].lang)?.start_byte()))).collect();
    let mut called: HashSet<NodeKey> = HashSet::new();
    // where each called name is declared, as the server says: an interface member has no body to resolve to
    let mut called_at: HashSet<(usize, usize, u32)> = HashSet::new();
    // files whose top-level code calls the project's functions or hands them on
    let mut working: HashSet<usize> = HashSet::new();
    // members called on a value the server cannot follow to a function, here or in a package (`model.load()`, typed by
    // an interface of the project): a function stored under that key in an object is called there
    let mut called_members: HashSet<&str> = HashSet::new();
    for &(i, callee) in &callees {
        let target = res.target(i, callee, w.adapters);
        // a member declared with no body (an interface's) is called through a value of that type
        let declared_only = matches!(&target, Target::Own { func, .. } if !p.files[func.file].lang.is_function(func.node.kind()));
        if let Target::Own { func, .. } = target {
            if !inside(func, i, callee) {
                called.insert(func.key());
            }
            // a call that reaches only a declaration (an interface's member) runs no code of the project's
            if !declared_only && top_level(p.files[i].lang, callee) && !p.files[i].generated() {
                working.insert(i);
            }
        }
        if declared_only || matches!(target, Target::Unresolved(_) | Target::Declared(_)) {
            let f = &p.files[i];
            let c = f.lang.unwrap(callee);
            if let Some(prop) = f.lang.member_fields(c.kind()).and_then(|(_, prop)| c.child_by_field_name(prop)) {
                called_members.insert(f.text_of(prop));
            }
        }
        if let Some(loc) = callee_name(callee, p.files[i].lang).and_then(|n| res.definition(i, n).into_iter().next()) {
            if let Some(fi) = p.file_index(&loc.file) {
                called_at.insert((fi, loc.line as usize, loc.col));
            }
        }
    }

    // 3. where a candidate is named without being called: passed to a call, stored, returned, exported
    let names: HashSet<&str> = cands.iter().filter_map(|c| Some(p.files[c.decl.file].text_of(c.name?))).collect();
    let own_names: HashSet<(usize, usize)> = cands.iter().filter_map(|c| Some((c.decl.file, c.name?.start_byte()))).collect();
    let mut refs: Vec<(usize, Node)> = vec![];
    for &i in &ready {
        let f = &p.files[i];
        let l = f.lang;
        let mut stack = vec![f.tree.root_node()];
        while let Some(n) = stack.pop() {
            if l.reach.not_uses.contains(&n.kind()) {
                continue;
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
            let key = (i, n.start_byte());
            // a property of it given a value (`Save.displayName = …`) hands nothing on
            let assigned = |m: Node| m.parent().is_some_and(|a| l.assigns.iter().any(|(k, field)| a.kind() == *k && a.child_by_field_name(field) == Some(m)));
            let property = n.parent().is_some_and(|x| l.member_fields(x.kind()).is_some_and(|(obj, _)| x.child_by_field_name(obj) == Some(n)) && assigned(x));
            if l.is_name(n.kind()) && names.contains(f.text_of(n)) && !callee_names.contains(&key) && !own_names.contains(&key) && !property {
                refs.push((i, n));
            }
        }
    }
    res.prefetch(&refs);
    let mut uses: HashMap<usize, Vec<(At, Option<Node>)>> = HashMap::new();
    let mut registered: HashMap<usize, At> = HashMap::new();
    // where each name used is declared, as the server says (a provider's name, though its value is made by a call)
    let mut named_at: HashSet<(usize, usize, u32)> = called_at.clone();
    for &(i, n) in &refs {
        if let Some(fi) = res.definition(i, n).into_iter().next().and_then(|loc| Some((p.file_index(&loc.file)?, loc.line as usize, loc.col))) {
            named_at.insert(fi);
        }
        let Target::Own { func, .. } = res.target(i, n, w.adapters) else { continue };
        let Some(&k) = by_key.get(&func.key()) else { continue };
        if inside(func, i, n) {
            continue;
        }
        // `export default save` makes it visible; nothing runs it there
        let exposed = n.parent().is_some_and(|x| p.files[i].lang.reach.exposes.contains(&x.kind()));
        if top_level(p.files[i].lang, n) && !exposed {
            working.insert(i);
        }
        let f = &p.files[func.file];
        if (f.lang.reach.accessor)(func.node, &f.text) {
            called.insert(func.key()); // reading it runs it
            continue;
        }
        let call = passed_in(p.files[i].lang, n, &p.files[i].text);
        if let Some(call) = call.filter(|c| registers(w, i, *c)) {
            registered.entry(k).or_insert(At { file: i, node: call });
        }
        uses.entry(k).or_default().push((At { file: i, node: n }, call));
    }

    // modules the project's own code imports, and where the names it imports from them are declared
    let (imported_files, imported_names) = imports(w, &ready);

    // 4. methods reached through a class they extend: a call written against the base reaches the override
    let mut out = vec![];
    for (k, c) in cands.iter().enumerate() {
        if called.contains(&c.decl.key()) && !registered.contains_key(&k) {
            continue;
        }
        let f = &p.files[c.decl.file];
        let l = f.lang;
        let unnamed = !uses.contains_key(&k) && !registered.contains_key(&k) && c.passed_to.is_none() && decorators(l, c.decl.node).is_empty();
        // a function stored under a key of an object (`{ load: () => … }`) where the project calls that key on a value
        // it cannot follow (`model.load()`): it runs there, through the value
        let in_object = l.follow.object.is_some_and(|(obj, pair, ..)| c.decl.node.parent().is_some_and(|x| x.kind() == pair || x.kind() == obj));
        if unnamed && in_object && c.name.is_some_and(|n| called_members.contains(f.text_of(n))) {
            continue;
        }
        // a function declared with a name of its own that nothing names (no use and no import reaches it), in a module
        // the program imports for something else: code nothing runs. A module nothing imports may be run by something
        // outside (a tool, a command), and a name another module imports may be used from outside the project (a
        // library's own). A method is reached through its object, and tests are run by their runner.
        let at = c.name.map(|n| (c.decl.file, n.start_position().row, f.utf16_col(n.start_position().row, n.start_position().column)));
        let named_elsewhere = at.is_some_and(|a| imported_names.contains(&a) || named_at.contains(&a));
        let test = w.adapters.list.iter().any(|a| a.test(f));
        if unnamed && !in_object && c.class.is_none() && imported_files.contains(&c.decl.file) && !named_elsewhere && !test {
            continue;
        }
        // code a test runner never starts by itself (a fixture, a helper written beside the tests)
        if test && w.adapters.list.iter().any(|a| a.reads(l) && a.support(f, c.decl.node)) {
            continue;
        }
        // one the framework runs only where the code names it (a component) never runs when nothing names it
        if unnamed && !in_object && !named_elsewhere && w.adapters.list.iter().any(|a| a.reads(l) && a.only_named(f, c.decl.node)) {
            continue;
        }
        let mut extends = vec![];
        if let (Some(class), Some(name)) = (c.class, c.name) {
            let mut seen = HashSet::new();
            if through_base(w, c.decl.file, class, f.text_of(name), &called, &called_at, &mut extends, &mut seen, 0) {
                continue;
            }
        }
        // 5. how it is reached, as far as the code shows
        let how = if let Some(d) = decorators(f.lang, c.decl.node).into_iter().next() {
            How::Decorated(At { file: c.decl.file, node: d })
        } else if let Some(call) = registered.get(&k) {
            How::Passed(*call)
        } else if let Some(call) = c.passed_to {
            How::Passed(At { file: c.decl.file, node: call })
        } else {
            let list = uses.get(&k).map(|v| v.as_slice()).unwrap_or(&[]);
            match list.iter().find(|(_, call)| call.is_some()).or(list.first()) {
                Some((at, Some(call))) => How::Passed(At { file: at.file, node: *call }),
                Some((at, None)) => How::Value(At { file: at.file, node: use_site(p.files[at.file].lang, at.node) }),
                None => How::Uncalled,
            }
        };
        let (name, named) = match (c.name, c.passed_to) {
            (Some(n), _) => (f.text_of(n).to_string(), true),
            (None, Some(call)) => (call_label(f.lang, call, c.decl.node, &f.text), false),
            (None, None) => ("anonymous".to_string(), false),
        };
        out.push(Entry { decl: c.decl, name, named, how, extends });
    }
    out.sort_by_key(|e| (e.decl.file, e.decl.node.start_byte()));
    let mut files: Vec<(usize, Vec<Node>)> = working
        .into_iter()
        .map(|i| {
            let f = &p.files[i];
            let l = f.lang;
            let mut c = f.tree.root_node().walk();
            let list = f
                .tree
                .root_node()
                .named_children(&mut c)
                .filter(|st| !l.reach.not_uses.contains(&st.kind()) && !matches!((l.stmt)(*st, &f.text), Stmt::Definition | Stmt::Other))
                .collect();
            (i, list)
        })
        .filter(|(_, list): &(usize, Vec<Node>)| !list.is_empty())
        .collect();
    files.sort_by_key(|(i, _)| *i);
    Found { entries: out, files }
}

/// The project's files another of its files imports, and where the names imported are declared (file, row, column
/// as the language server counts it), as the language server finds them.
fn imports(w: &World, ready: &[usize]) -> (HashSet<usize>, HashSet<(usize, usize, u32)>) {
    let p = w.p;
    let mut asks: Vec<(usize, Node, bool)> = vec![];
    for &i in ready {
        let f = &p.files[i];
        let root = f.tree.root_node();
        for imp in &f.imports {
            for (row, col, name) in [(imp.line, imp.col, true), (imp.module_at.0, imp.module_at.1, false)] {
                let at = tree_sitter::Point { row, column: col };
                if let Some(n) = root.named_descendant_for_point_range(at, at) {
                    asks.push((i, n, name));
                }
            }
        }
    }
    w.res.prefetch(&asks.iter().map(|&(i, n, _)| (i, n)).collect::<Vec<_>>());
    let (mut files, mut names) = (HashSet::new(), HashSet::new());
    for (i, n, name) in asks {
        for loc in w.res.definition(i, n) {
            let Some(fi) = p.file_index(&loc.file).filter(|&fi| fi != i) else { continue };
            files.insert(fi);
            if name {
                names.insert((fi, loc.line as usize, loc.col));
            }
        }
    }
    (files, names)
}

/// Written at the top of a file: inside no function and no class.
fn top_level(l: &Lang, n: Node) -> bool {
    let mut p = n.parent();
    while let Some(x) = p {
        if l.is_function(x.kind()) || l.reach.classes.contains(&x.kind()) {
            return false;
        }
        p = x.parent();
    }
    true
}

/// `inner` is written inside `outer` (a function calling itself is not called by the project).
fn inside(outer: At, file: usize, inner: Node) -> bool {
    outer.file == file && outer.node.start_byte() <= inner.start_byte() && inner.end_byte() <= outer.node.end_byte()
}

/// The call a value is passed to: it is one of the call's arguments, directly or as a keyword or a key in an object
/// written there.
fn passed_in<'t>(l: &Lang, n: Node<'t>, src: &str) -> Option<Node<'t>> {
    let mut x = n;
    for _ in 0..4 {
        let p = x.parent()?;
        if l.call_field(p.kind()).is_some() {
            return match (l.expr)(p, src) {
                Expr::Call { args, .. } if args.iter().any(|a| a.start_byte() <= n.start_byte() && n.end_byte() <= a.end_byte()) => Some(p),
                _ => None,
            };
        }
        if l.is_function(p.kind()) || p.kind().ends_with("statement") || l.blocks.contains(&p.kind()) {
            return None;
        }
        x = p;
    }
    None
}

/// A call that registers what is passed to it: it reaches code outside the project and its result is not used.
fn registers(w: &World, file: usize, call: Node) -> bool {
    let f = &w.p.files[file];
    let l = f.lang;
    let Expr::Call { callee, .. } = (l.expr)(call, &f.text) else { return false };
    if matches!(w.res.target(file, callee, w.adapters), Target::Own { .. }) {
        return false;
    }
    // the call is the whole statement (awaited or not)
    let mut top = call;
    while let Some(p) = top.parent().filter(|p| l.unwrap.contains(&p.kind())) {
        top = p;
    }
    top.parent().is_some_and(|s| matches!((l.stmt)(s, &f.text), Stmt::Expr(e) if e == top))
}

/// The decorators written on a function, in source order: its own, those of a wrapper that holds only it, or the
/// ones written just before it among its siblings.
fn decorators<'t>(l: &Lang, f: Node<'t>) -> Vec<Node<'t>> {
    let is_dec = |n: &Node| l.reach.decorators.contains(&n.kind());
    let mut c = f.walk();
    let own: Vec<Node> = f.named_children(&mut c).filter(is_dec).collect();
    if !own.is_empty() {
        return own;
    }
    if let Some(p) = f.parent() {
        let mut c = p.walk();
        let kids: Vec<Node> = p.named_children(&mut c).collect();
        if kids.iter().filter(|k| !is_dec(k) && k.kind() != "comment").count() == 1 && kids.iter().any(is_dec) {
            return kids.into_iter().filter(is_dec).collect();
        }
    }
    let mut before = vec![];
    let mut s = f.prev_named_sibling();
    while let Some(x) = s.filter(is_dec) {
        before.insert(0, x);
        s = x.prev_named_sibling();
    }
    before
}

/// What to show for a value use: the member expression a name is the end of (`self.handle`), else the name.
fn use_site<'t>(l: &Lang, n: Node<'t>) -> Node<'t> {
    n.parent().filter(|p| l.member_fields(p.kind()).is_some_and(|(_, prop)| p.child_by_field_name(prop) == Some(n))).unwrap_or(n)
}

/// An unnamed function's name: the call it is written in, with the arguments before it (`app.get("/items", …)`).
fn call_label(l: &Lang, call: Node, func: Node, src: &str) -> String {
    let Expr::Call { callee, args } = (l.expr)(call, src) else { return "anonymous".into() };
    let short = |n: Node| one_line(&src[n.byte_range()], 40);
    let mut parts: Vec<String> = args.iter().take_while(|a| a.end_byte() <= func.start_byte()).map(|a| short(*a)).collect();
    parts.push("…".into());
    format!("{}({})", short(callee), parts.join(", "))
}

/// Whether a method named `name` in `class` is reached through a class it extends: one in the project that declares
/// the same member, where that member is called. Classes outside the project are collected in `outside`.
#[allow(clippy::too_many_arguments)]
fn through_base(
    w: &World,
    file: usize,
    class: Node,
    name: &str,
    called: &HashSet<NodeKey>,
    called_at: &HashSet<(usize, usize, u32)>,
    outside: &mut Vec<String>,
    seen: &mut HashSet<NodeKey>,
    depth: usize,
) -> bool {
    if depth > 5 || !seen.insert(At { file, node: class }.key()) {
        return false;
    }
    let p = w.p;
    let f = &p.files[file];
    let l = f.lang;
    for base in (l.reach.bases)(class) {
        let Some(nn) = callee_name(base, l) else { continue };
        let Some(loc) = w.res.definition(file, nn).into_iter().next() else { continue };
        let Some(bi) = p.file_index(&loc.file).filter(|_| p.is_own(&loc.file)) else {
            outside.push(f.text_of(base).to_string());
            continue;
        };
        let bf = &p.files[bi];
        // the class the definition names
        let mut at = bf.node_at(loc.line as usize, loc.col);
        while let Some(x) = at.filter(|x| !bf.lang.reach.classes.contains(&x.kind())) {
            at = x.parent();
        }
        let Some(bc) = at else { continue };
        for m in (bf.lang.reach.members)(bc).into_iter().filter(|m| bf.text_of(*m) == name) {
            let pos = (bi, m.start_position().row, bf.utf16_col(m.start_position().row, m.start_position().column));
            let body = bf.lang.function_of(m).map(|d| At { file: bi, node: d }.key());
            if called_at.contains(&pos) || body.is_some_and(|b| called.contains(&b)) {
                return true;
            }
        }
        if through_base(w, bi, bc, name, called, called_at, outside, seen, depth + 1) {
            return true;
        }
    }
    false
}
