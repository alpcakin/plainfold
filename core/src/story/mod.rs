// Builds one story (a trigger and everything after it) by walking the code from the entry point.
// Structure only: arrows, order, branches and loops come from the syntax tree, the language server and
// framework adapters. Nothing here names a language or a framework: syntax comes from the language tables,
// meaning from adapters (`crate::adapters`).
pub mod entries;
pub mod ids;
pub mod reach;
pub mod routes;

use crate::adapters::{Adapters, Start, TriggerSite};
use crate::lang::{Expr, Lang, Own, Stmt};
use crate::model::*;
use crate::resolve::{Resolver, Target};
use crate::source::{At, NodeKey, Project, SourceFile, one_line};
use routes::{Route, RouteKind};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use tree_sitter::Node;

/// Values known as code text while walking (a state's current value, for example); "?" = unknown.
pub type Known = Rc<RefCell<Vec<(String, String)>>>;

#[derive(Clone)]
pub struct Ctx {
    /// participant doing the work
    pub this: String,
    pub group: Group,
    /// who receives `return` (None inside callbacks: their return goes nowhere visible)
    pub return_to: Option<String>,
    /// returns are responses to a request
    pub endpoint: bool,
    pub known: Known,
    /// functions being expanded, against cycles
    pub stack: Vec<NodeKey>,
    /// runs inside the caller (same lifeline)
    pub lane: bool,
    /// inside a repeated row
    pub in_loop: bool,
    /// plain helper calls (formatting while drawing) are not part of the story
    pub quiet: bool,
    /// the `catch` clause being walked, in this function
    pub catching: Option<Rc<Caught>>,
}

/// A `catch` clause: the name it gives what it caught, and what of the `try` it handles (a rethrow passes it on).
pub struct Caught {
    name: Option<String>,
    ends: Vec<End>,
}

impl Ctx {
    pub fn new(this: String, group: Group, stack: Vec<NodeKey>) -> Ctx {
        Ctx { this, group, return_to: None, endpoint: false, known: Rc::default(), stack, lane: false, in_loop: false, quiet: false, catching: None }
    }
    pub fn known(&self, name: &str) -> Option<String> {
        self.known.borrow().iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
    pub fn set_known(&self, name: &str, value: String) {
        let mut k = self.known.borrow_mut();
        match k.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = value,
            None => k.push((name.to_string(), value)),
        }
    }
}

/// `a.b(x).c(y)` → root `a`, segments [b(x), c(y)].
pub struct Seg<'w> {
    pub name: String,
    pub args: Option<Vec<Node<'w>>>,
    pub node: Node<'w>,
}
pub struct Chain<'w> {
    pub root: Node<'w>,
    pub segs: Vec<Seg<'w>>,
}

/// Everything shared by the stories of one project.
pub struct World<'w> {
    pub p: &'w Project,
    pub res: &'w Resolver<'w>,
    pub adapters: &'w Adapters,
    pub routes: Vec<Route<'w>>,
    /// the id of every function and class, per file, made when a file is first asked about
    ids: RefCell<HashMap<usize, Rc<HashMap<ids::Key, String>>>>,
    /// the project's classes that name each of its classes as a base, made when first asked about
    subs: OnceCell<HashMap<NodeKey, Vec<NodeKey>>>,
    /// whether a screen's opening is a story of its own, worked out when first asked about
    told: RefCell<HashMap<NodeKey, bool>>,
}

impl<'w> World<'w> {
    pub fn new(p: &'w Project, res: &'w Resolver<'w>, adapters: &'w Adapters) -> World<'w> {
        let routes = adapters.list.iter().flat_map(|a| a.routes(p, res)).collect();
        World { p, res, adapters, routes, ids: RefCell::default(), subs: OnceCell::new(), told: RefCell::default() }
    }

    /// The project's classes that extend `class`, directly or through others.
    pub fn subclasses(&self, class: At<'w>) -> Vec<At<'w>> {
        let subs = self.subs.get_or_init(|| {
            let p = self.p;
            let mut bases: Vec<(At<'w>, Node<'w>)> = vec![];
            for (i, f) in p.files.iter().enumerate() {
                let l = f.lang;
                let mut stack = vec![f.tree.root_node()];
                while let Some(n) = stack.pop() {
                    let mut c = n.walk();
                    stack.extend(n.named_children(&mut c));
                    if l.reach.classes.contains(&n.kind()) {
                        for b in (l.reach.bases)(n) {
                            if let Some(name) = crate::resolve::callee_name(b, l) {
                                bases.push((At { file: i, node: n }, name));
                            }
                        }
                    }
                }
            }
            self.res.prefetch(&bases.iter().map(|(c, n)| (c.file, *n)).collect::<Vec<_>>());
            let mut out: HashMap<NodeKey, Vec<NodeKey>> = HashMap::new();
            for (class, name) in bases {
                let Some(loc) = self.res.definition(class.file, name).into_iter().next().filter(|l| p.is_own(&l.file)) else { continue };
                let Some(bi) = p.file_index(&loc.file) else { continue };
                let bf = &p.files[bi];
                let mut at = bf.node_at(loc.line as usize, loc.col);
                while let Some(x) = at.filter(|x| !bf.lang.reach.classes.contains(&x.kind())) {
                    at = x.parent();
                }
                if let Some(base) = at {
                    out.entry(At { file: bi, node: base }.key()).or_default().push(class.key());
                }
            }
            out
        });
        let mut all: Vec<NodeKey> = vec![];
        let mut todo = vec![class.key()];
        while let Some(c) = todo.pop() {
            for s in subs.get(&c).into_iter().flatten() {
                if *s != class.key() && !all.contains(s) {
                    all.push(*s);
                    todo.push(*s);
                }
            }
        }
        all.sort_by_key(|k| (k.file, k.start));
        all.into_iter().filter_map(|k| self.node_of(k)).collect()
    }

    /// The node a key was made from.
    fn node_of(&self, k: NodeKey) -> Option<At<'w>> {
        let mut n = self.p.files[k.file].tree.root_node().descendant_for_byte_range(k.start, k.end)?;
        while n.kind_id() != k.kind {
            n = n.parent().filter(|p| p.start_byte() == k.start && p.end_byte() == k.end)?;
        }
        Some(At { file: k.file, node: n })
    }

    /// What a function or class is known by in the model (`ids.rs`).
    pub fn id_of(&self, decl: At<'w>) -> String {
        let f = &self.p.files[decl.file];
        let table = self.ids.borrow_mut().entry(decl.file).or_insert_with(|| Rc::new(ids::of_file(&f.rel, f.lang, &f.text, f.tree.root_node()))).clone();
        match table.get(&ids::key(decl.node)) {
            Some(id) => id.clone(),
            // not a function or class (a value the resolver ended at): its name, as before
            None => format!("{}#{}", f.rel, (f.lang.function_name)(decl.node).map(|n| f.text_of(n)).unwrap_or("anonymous")),
        }
    }
}

/// The text of a string literal, with parts only known at runtime shown as "{…}"; None for anything else.
pub fn string_value(l: &Lang, text: &str, n: Node) -> Option<String> {
    let n = l.unwrap(n);
    let s = &l.strings;
    if !s.nodes.contains(&n.kind()) {
        return None;
    }
    let mut out = String::new();
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        if s.fragments.contains(&ch.kind()) {
            out.push_str(&text[ch.byte_range()]);
        } else if s.interpolations.contains(&ch.kind()) {
            out.push_str("{…}");
        } else if s.nodes.contains(&ch.kind()) {
            // strings written next to each other are one (`"a" "b"`), and a string can be written inside the node
            // that says it is one (Dart's quotes)
            out.push_str(&string_value(l, text, ch)?);
        }
    }
    Some(out)
}

pub fn boring(items: &[Item]) -> bool {
    items.iter().all(Item::is_local)
}

/// The steps change the state a screen shows (a setter, `setState`, a shared value reset), here or in a call they make,
/// not later. A state step that says nothing it changes (the values a component starts with) does not count.
fn changes_state(items: &[Item]) -> bool {
    items.iter().any(|i| match i {
        Item::Own { kind: SelfKind::State, detail, .. } => detail.is_some(),
        Item::Frame { kind: FrameKind::Effect, .. } => false,
        Item::Frame { items, els, .. } => changes_state(items) || els.as_deref().is_some_and(changes_state),
        Item::Call(c) => c.body.as_deref().is_some_and(changes_state),
        _ => false,
    })
}

/// Plain statements, and jumps out of a round of the loop under conditions and catches: nothing else happens.
fn only_jumps(items: &[Item]) -> bool {
    items.iter().all(|i| match i {
        Item::Own { kind: SelfKind::Local | SelfKind::Jump, .. } => true,
        Item::Frame { kind: FrameKind::Alt | FrameKind::Catch, items, els, .. } => only_jumps(items) && els.as_deref().is_none_or(only_jumps),
        _ => false,
    })
}

pub struct Builder<'w> {
    pub w: &'w World<'w>,
    pub parts: Vec<Participant>,
    pub signals: Vec<Finding>,
    pub gaps: Vec<Finding>,
    pub walked: Vec<(String, At<'w>)>,
    n: usize,
    pub files: BTreeSet<String>,
    pub meaningful: bool,
    /// what the walk reached, and the steps being walked inside (`reach.rs`)
    reached: Vec<reach::Reached>,
    anchors: Vec<String>,
    parents: HashMap<String, Option<String>>,
    /// the finished story's uses, for the index
    pub uses: Vec<reach::Reach>,
    /// how each throw, and each call with a body, can end the function it is in (by item id)
    ends: HashMap<String, Vec<End>>,
    /// what each function changes of what reaches it from outside
    changes: HashMap<NodeKey, Vec<Change>>,
    /// the call of a project function that returned each function being walked (`save = guard(check, f)`)
    made: Vec<(NodeKey, At<'w>)>,
    /// things already shown once in this story (a provider's code, at its first read)
    once: HashSet<NodeKey>,
}

/// Something a function changes of what reaches it from outside: a value it is given (by the parameter's name) or its
/// own object, and the part changed, as written after it (`.items`, `[key]`), or, for a field written without its
/// object, from the field's name on (`items`).
#[derive(Clone, PartialEq)]
pub struct Change {
    pub root: Root,
    pub path: String,
}

#[derive(Clone, PartialEq)]
pub enum Root {
    Param(String),
    Own,
}

/// A part written after a value: `cart` and `.items` → `cart.items`; a field's name gets its dot.
fn join_path(base: &str, path: &str) -> String {
    if base.is_empty() || path.is_empty() {
        format!("{base}{path}")
    } else if path.starts_with('.') || path.starts_with('[') {
        format!("{base}{path}")
    } else {
        format!("{base}.{path}")
    }
}

/// A way a function ends besides returning: what it throws, as labelled, and the type thrown when the code names it.
#[derive(Clone)]
struct End {
    label: String,
    ty: Option<String>,
}

impl<'w> Builder<'w> {
    pub fn new(w: &'w World<'w>) -> Builder<'w> {
        Builder {
            w,
            parts: vec![],
            signals: vec![],
            gaps: vec![],
            walked: vec![],
            n: 0,
            files: BTreeSet::new(),
            meaningful: false,
            reached: vec![],
            anchors: vec![],
            parents: HashMap::new(),
            uses: vec![],
            ends: HashMap::new(),
            changes: HashMap::new(),
            made: vec![],
            once: HashSet::new(),
        }
    }

    /// Runs `f` with the parameters of the function that made the one being walked (`via`: the call that returned it)
    /// standing for what that call was given.
    pub fn with_via<R>(&mut self, func: At<'w>, via: Option<At<'w>>, f: impl FnOnce(&mut Self) -> R) -> R {
        let mark = (self.w.res.bindings(), self.made.len());
        if let Some(call) = via {
            self.made.push((func.key(), call));
            let l = self.lang(call);
            if let Expr::Call { callee, args } = (l.expr)(l.unwrap(call.node), &self.file(call).text) {
                if let Target::Own { func, .. } = self.w.res.target(call.file, callee, self.w.adapters) {
                    self.bind_args(func, call.file, &args);
                }
            }
        }
        let out = f(self);
        self.w.res.unbind(mark.0);
        self.made.truncate(mark.1);
        out
    }

    /// The parameters of `func` stand for what a call gave (by place, or by the name a keyword argument gives).
    pub fn bind_args(&mut self, func: At<'w>, file: usize, args: &[Node<'w>]) {
        let l = self.w.p.files[file].lang;
        let src = &self.w.p.files[file].text;
        let (params, _) = self.params_of(func);
        let mut place = 0;
        for a in args {
            let (name, value) = match (l.named_arg)(*a, src) {
                Some((n, v)) => (Some(src[n.byte_range()].to_string()), v),
                None => (None, *a),
            };
            let name = name.or_else(|| {
                place += 1;
                params.get(place - 1).cloned()
            });
            if let Some(name) = name.filter(|n| params.contains(n)) {
                self.w.res.bind(func.key(), name, At { file, node: l.unwrap(value) });
            }
        }
    }

    /// Where a function always runs, whoever calls it, and what makes it so.
    pub fn fixed_place(&self, decl: At<'w>) -> Option<(Group, &'static str)> {
        let made = self.made_by(decl);
        self.w.adapters.list.iter().find_map(|a| a.runs_on(self, decl, made))
    }

    /// The project's functions a call is given (written in it, or named there).
    pub fn given_functions(&self, call: At<'w>) -> Vec<At<'w>> {
        let l = self.lang(call);
        let Expr::Call { args, .. } = (l.expr)(l.unwrap(call.node), &self.file(call).text) else { return vec![] };
        args.into_iter()
            .filter_map(|a| {
                let a = self.unwrap(self.at(call, l.arg_value(a, &self.file(call).text)));
                if self.is_function(a) { Some(a) } else { self.project_decl(a) }
            })
            .collect()
    }

    /// The call of a project function that returned `decl`, while it is walked.
    pub fn made_by(&self, decl: At<'w>) -> Option<At<'w>> {
        self.made.iter().rev().find(|(k, _)| *k == decl.key()).map(|(_, c)| *c)
    }

    /// The name a call's result is stored under (`save = guard(check, f)` → `save`).
    fn stored_as(&self, call: At<'w>) -> Option<String> {
        let l = self.lang(call);
        let mut n = call.node;
        while let Some(p) = n.parent() {
            if let Some((name_field, value_field)) = l.follow.declarator(p.kind()) {
                let value = p.child_by_field_name(value_field).map(|v| l.unwrap(v));
                return (value == Some(l.unwrap(call.node))).then(|| p.child_by_field_name(name_field)).flatten().map(|name| self.text(self.at(call, name)).to_string());
            }
            if !l.unwrap.contains(&p.kind()) {
                return None;
            }
            n = p;
        }
        None
    }

    /// A call of a name that is a parameter of the function that made the one being walked: it runs what that call was
    /// given (`guard(check, f)`, and `f(x)` inside what `guard` returns).
    fn bound_call(&mut self, e: At<'w>, callee: At<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let l = self.lang(callee);
        let name = self.unwrap(callee);
        if !l.is_name(name.node.kind()) {
            return None;
        }
        let loc = self.w.res.definition(callee.file, name.node).into_iter().next()?;
        let (df, dn) = self.w.res.at_loc(&loc)?;
        let dl = self.w.p.files[df].lang;
        let owner = std::iter::successors(dn.parent(), |x| x.parent()).find(|x| dl.is_function(x.kind()))?;
        let key = At { file: df, node: owner }.key();
        let text = self.w.p.files[df].text_of(dn);
        let given = self.w.res.bound_to(key, text)?;
        let given = self.unwrap(given);
        let decl = if self.is_function(given) { given } else { self.project_decl(given)? };
        Some(self.project_call(e, decl, &["a function given to the call that made this one".to_string()], ctx))
    }

    // ---------- small helpers ----------

    pub fn file(&self, at: At<'w>) -> &'w SourceFile {
        &self.w.p.files[at.file]
    }
    pub fn lang(&self, at: At<'w>) -> &'static Lang {
        self.w.p.files[at.file].lang
    }
    pub fn text(&self, at: At<'w>) -> &'w str {
        &self.w.p.files[at.file].text[at.node.byte_range()]
    }
    pub fn at(&self, like: At<'w>, node: Node<'w>) -> At<'w> {
        At { file: like.file, node }
    }
    pub fn place(&self, at: At<'w>) -> Ref {
        self.w.p.ref_of(at.file, at.node, None)
    }
    pub fn span(&self, a: At<'w>, b: At<'w>) -> Ref {
        self.w.p.ref_of(a.file, a.node, Some(b.node))
    }
    pub fn id(&mut self) -> String {
        self.n += 1;
        format!("i{}", self.n)
    }
    pub fn part(&mut self, p: Participant) -> String {
        if !self.has_part(&p.id) {
            self.parts.push(p.clone());
        }
        p.id
    }
    /// Whether the story already has this participant. A call that is dropped removes its callee only when it added
    /// it: a call kept earlier in the story can lead to the same one.
    pub fn has_part(&self, id: &str) -> bool {
        self.parts.iter().any(|x| x.id == id)
    }
    pub fn part_mut(&mut self, id: &str) -> Option<&mut Participant> {
        self.parts.iter_mut().find(|p| p.id == id)
    }
    pub fn drop_part(&mut self, id: &str) {
        self.parts.retain(|p| p.id != id);
    }
    pub fn signal(&mut self, text: String, at: Ref) {
        if !self.signals.iter().any(|s| s.text == text) {
            self.signals.push(Finding { text, at });
        }
    }
    pub fn gap(&mut self, text: String, at: Ref) {
        if !self.gaps.iter().any(|s| s.text == text) {
            self.gaps.push(Finding { text, at });
        }
    }
    pub fn unwrap(&self, at: At<'w>) -> At<'w> {
        At { file: at.file, node: self.lang(at).unwrap(at.node) }
    }
    pub fn is_function(&self, at: At<'w>) -> bool {
        self.lang(at).is_function(at.node.kind())
    }
    /// A condition as written, without the parentheses around it.
    pub fn cond_text(&self, cond: At<'w>) -> String {
        let n = if cond.node.kind() == "parenthesized_expression" { cond.node.named_child(0).unwrap_or(cond.node) } else { cond.node };
        one_line(self.text(self.at(cond, n)), 0)
    }

    pub fn decl_name(&self, decl: At<'w>) -> String {
        (self.lang(decl).function_name)(decl.node)
            .map(|n| self.text(self.at(decl, n)).to_string())
            // a function a call returned is known by the name that call's result is stored under
            .or_else(|| self.made_by(decl).and_then(|c| self.stored_as(c)))
            .unwrap_or_else(|| "anonymous".into())
    }
    pub fn pid(&self, decl: At<'w>) -> String {
        match self.made_by(decl).and_then(|c| Some((c, self.stored_as(c)?))) {
            Some((c, name)) => format!("{}#{name}", self.file(c).rel),
            None => self.w.id_of(decl),
        }
    }

    /// The function a callee expression reaches, if it is in the project's own code.
    pub fn project_decl(&self, callee: At<'w>) -> Option<At<'w>> {
        self.project_target(callee).map(|(f, _)| f)
    }

    /// The function a callee reaches, and the call of a project function that returned it, if one did.
    pub fn project_target_via(&self, callee: At<'w>) -> Option<(At<'w>, Option<At<'w>>)> {
        match self.w.res.target(callee.file, callee.node, self.w.adapters) {
            Target::Own { func, via, .. } => Some((func, via)),
            _ => None,
        }
    }

    /// The same, with the hops followed past the first definition (an alias, a returned value…).
    pub fn project_target(&self, callee: At<'w>) -> Option<(At<'w>, Vec<String>)> {
        match self.w.res.target(callee.file, callee.node, self.w.adapters) {
            Target::Own { func, hops, .. } => Some((func, hops)),
            _ => None,
        }
    }

    /// How an arrow to a resolved function is recorded. Answers from a server that was not ready are not exact.
    pub fn resolved_link(&self, site: At<'w>, decl: At<'w>, what: &str, hops: &[String]) -> Link {
        let r = self.place(decl);
        let server = self.w.res.server_for(site.file).map(|s| s.client.name.clone()).unwrap_or_default();
        let via = if hops.is_empty() { String::new() } else { format!(" (followed: {})", hops.join(" → ")) };
        if self.w.res.ready(site.file) {
            Link { kind: LinkKind::Exact, how: format!("{what} resolved by {server} to {}:{}{via}", r.file, r.from) }
        } else {
            Link { kind: LinkKind::Inferred, how: format!("{what} resolved by {server} to {}:{}{via}, before the server was ready", r.file, r.from) }
        }
    }

    pub fn chain(&self, e: At<'w>) -> Chain<'w> {
        let l = self.lang(e);
        let mut segs = vec![];
        let mut n = l.unwrap(e.node);
        loop {
            if let Expr::Call { callee, args } = (l.expr)(n, &self.file(e).text) {
                let c = l.unwrap(callee);
                if let Some((obj, prop)) = l.member_fields(c.kind()) {
                    if let (Some(o), Some(p)) = (c.child_by_field_name(obj), c.child_by_field_name(prop)) {
                        segs.insert(0, Seg { name: self.text(self.at(e, p)).to_string(), args: Some(args), node: n });
                        n = l.unwrap(o);
                        continue;
                    }
                }
            } else if let Some((obj, prop)) = l.member_fields(n.kind()) {
                if let (Some(o), Some(p)) = (n.child_by_field_name(obj), n.child_by_field_name(prop)) {
                    segs.insert(0, Seg { name: self.text(self.at(e, p)).to_string(), args: None, node: n });
                    n = l.unwrap(o);
                    continue;
                }
            }
            break;
        }
        Chain { root: n, segs }
    }

    pub fn seg_text(&self, file: usize, s: &Seg<'w>) -> String {
        let args = s.args.as_ref().map(|a| format!("({})", a.iter().map(|x| one_line(self.text(At { file, node: *x }), 0)).collect::<Vec<_>>().join(", ")));
        format!(".{}{}", s.name, args.unwrap_or_default())
    }

    /// The text of a string literal, with parts only known at runtime shown as "{…}".
    pub fn url_text(&self, at: At<'w>) -> Option<String> {
        let at = self.unwrap(at);
        string_value(self.lang(at), &self.file(at).text, at.node)
    }

    /// The words a string holds: one written here, or a name defined as one, on its own or as a key of an object
    /// (`labels.save`, defined as `save: "Save"`). None when any part is only known at runtime. A choice between
    /// values gives the words of each that has some (`pro ? "Manage" : "Upgrade"` → "Manage / Upgrade").
    pub fn words_of(&self, at: At<'w>) -> Option<String> {
        let at = self.unwrap(at);
        let l = self.lang(at);
        let words = |l: &Lang, text: &str, n: Node| string_value(l, text, n).filter(|s| !s.contains("{…}") && !s.trim().is_empty()).map(|s| s.trim().to_string());
        if l.strings.nodes.contains(&at.node.kind()) {
            return words(l, &self.file(at).text, at.node);
        }
        // a choice between words (`done ? "Close" : "Cancel"`): each of them
        if let Expr::Cond { then, els, .. } = (l.expr)(at.node, &self.file(at).text) {
            let mut said: Vec<String> = vec![];
            for w in std::iter::once(then).chain(els).filter_map(|x| self.words_of(self.at(at, x))) {
                if !said.contains(&w) {
                    said.push(w);
                }
            }
            return (!said.is_empty()).then(|| said.join(" / "));
        }
        let name = match l.member_fields(at.node.kind()) {
            Some((_, prop)) => at.node.child_by_field_name(prop)?,
            None if l.is_name(at.node.kind()) => at.node,
            None => return None,
        };
        let loc = self.w.res.definition(at.file, name).into_iter().next()?;
        let df = &self.w.p.files[self.w.p.file_index(&loc.file)?];
        let n = df.node_at(loc.line as usize, loc.col)?;
        let p = n.parent()?;
        let pair = df.lang.follow.object.filter(|(_, pair, key, _, _)| p.kind() == *pair && p.child_by_field_name(key) == Some(n)).and_then(|(_, _, _, value, _)| p.child_by_field_name(value));
        let value = match df.lang.declared(n) {
            Some((_, value)) => value?,
            None => pair?,
        };
        words(df.lang, &df.text, value)
    }

    /// `const save = () => …`, `save = lambda: …`: a function given a plain name, which is a definition like any other.
    /// One stored anywhere else (`handlers[key] = fn`, `obj.save = fn`) does something.
    pub fn names_function(&self, st: At<'w>) -> bool {
        let l = self.lang(st);
        let mut found = vec![];
        let mut c = st.node.walk();
        for n in std::iter::once(st.node).chain(st.node.named_children(&mut c)) {
            if let Some(fields) = l.follow.declarator(n.kind()) {
                found.push((n, fields));
            }
        }
        !found.is_empty()
            && found.iter().all(|(d, (name, value))| d.child_by_field_name(name).is_some_and(|n| l.is_name(n.kind())) && d.child_by_field_name(value).is_some_and(|v| l.is_function(l.unwrap(v).kind())))
    }

    /// The function of the project an inline function only hands on to: `() => save(item)` → `save`. One that does
    /// anything else has none.
    pub fn hands_on(&self, f: At<'w>) -> Option<At<'w>> {
        let l = self.lang(f);
        let src = &self.file(f).text;
        let body = (l.function_body)(f.node)?;
        let e = if l.blocks.contains(&body.kind()) {
            match (l.body)(body).as_slice() {
                [only] => match (l.stmt)(*only, src) {
                    Stmt::Expr(e) | Stmt::Return(Some(e)) => e,
                    _ => return None,
                },
                _ => return None,
            }
        } else {
            body
        };
        let e = l.unwrap(e);
        let e = (l.discard)(e, src).map(|x| l.unwrap(x)).unwrap_or(e);
        let Expr::Call { callee, .. } = (l.expr)(e, src) else { return None };
        self.project_target(self.at(f, l.unwrap(callee))).filter(|(d, _)| self.is_function(*d)).map(|(d, _)| d)
    }

    /// The same, when the call names that function itself: not a call some framework sends elsewhere
    /// (`mutation.mutate()` runs what the mutation was made with, and more).
    fn hands_on_directly(&self, f: At<'w>) -> Option<At<'w>> {
        let d = self.hands_on(f)?;
        let l = self.lang(f);
        let body = (l.function_body)(f.node)?;
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            if let Expr::Call { callee, .. } = (l.expr)(n, &self.file(f).text) {
                return self.project_target(self.at(f, l.unwrap(callee))).filter(|(x, hops)| x.key() == d.key() && hops.is_empty()).map(|(x, _)| x);
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
        None
    }

    /// What the code calls a result: `const { data, error } = await …` → "{ data, error }".
    pub fn receiver(&self, e: At<'w>) -> Option<String> {
        let l = self.lang(e);
        let mut n = e.node;
        while let Some(p) = n.parent().filter(|p| l.unwrap.contains(&p.kind())) {
            n = p;
        }
        let p = n.parent()?;
        let (name, value) = l.follow.declarator(p.kind())?;
        if p.child_by_field_name(value).is_some_and(|v| v == n) {
            return Some(one_line(self.text(self.at(e, p.child_by_field_name(name)?)), 0));
        }
        None
    }

    /// What the caller does with a call's result: nothing (the call is a statement of its own, or its value is thrown
    /// away with `void`), and the name it gives the result when it keeps it.
    pub fn result_use(&self, e: At<'w>) -> (bool, Option<String>) {
        let l = self.lang(e);
        let src = &self.file(e).text;
        let mut n = e.node;
        while let Some(p) = n.parent() {
            if l.unwrap.contains(&p.kind()) {
                n = p;
                continue;
            }
            if (l.discard)(p, src) == Some(n) {
                return (true, None);
            }
            break;
        }
        let unused = n.parent().is_some_and(|p| matches!((l.stmt)(p, src), Stmt::Expr(x) if x == n));
        (unused, if unused { None } else { self.receiver(e) })
    }

    /// A value as a short label: `{ error: "Not logged in" }`, `{ books, page }`, `null`.
    pub fn short(&self, e: At<'w>) -> String {
        let e = self.unwrap(e);
        let full = one_line(self.text(e), 0);
        let l = self.lang(e);
        if let Some((obj, pair, key, value, _)) = l.follow.object {
            if e.node.kind() == obj && full.chars().count() > 36 {
                let mut c = e.node.walk();
                let keys: Vec<String> = e
                    .node
                    .named_children(&mut c)
                    .filter(|p| p.kind() != "comment")
                    .map(|p| {
                        if p.kind() == pair {
                            let k = p.child_by_field_name(key).map(|k| self.text(self.at(e, k))).unwrap_or("…");
                            match p.child_by_field_name(value).filter(|v| l.strings.nodes.contains(&v.kind()) && v.kind() == l.strings.nodes[0]) {
                                Some(v) => format!("{k}: {}", self.text(self.at(e, v))),
                                None => k.to_string(),
                            }
                        } else if p.kind().contains("spread") {
                            format!("...{}", one_line(self.text(self.at(e, p.named_child(0).unwrap_or(p))), 12))
                        } else if l.is_name(p.kind()) || p.kind().contains("shorthand") {
                            self.text(self.at(e, p)).to_string()
                        } else {
                            p.child_by_field_name("name").map(|n| self.text(self.at(e, n)).to_string()).unwrap_or("…".into())
                        }
                    })
                    .collect();
                return format!("{{ {} }}", keys.join(", "));
            }
        }
        one_line(&full, 40)
    }

    /// Whether a condition is decided by values known as code text (`if (!loading)` with `loading = false`).
    pub fn decide(&self, cond: At<'w>, ctx: &Ctx) -> Option<bool> {
        let cond = self.unwrap(cond);
        let l = self.lang(cond);
        if let Some(inner) = (l.not_operand)(cond.node, &self.file(cond).text) {
            return self.decide(self.at(cond, inner), ctx).map(|v| !v);
        }
        if l.is_name(cond.node.kind()) {
            let v = ctx.known(self.text(cond))?;
            if l.truthy.contains(&v.as_str()) {
                return Some(true);
            }
            if l.falsy.contains(&v.as_str()) {
                return Some(false);
            }
        }
        None
    }

    // ---------- statements ----------

    pub fn stmts(&mut self, file: usize, list: &[Node<'w>], ctx: &Ctx) -> Vec<Item> {
        let l = self.w.p.files[file].lang;
        let src = &self.w.p.files[file].text;
        let mut out = vec![];
        let mut pending: Vec<Node<'w>> = vec![];
        macro_rules! flush {
            () => {
                if !pending.is_empty() {
                    let item = self.local(file, &pending, ctx);
                    out.push(item);
                    pending.clear();
                }
            };
        }
        macro_rules! add {
            ($st:expr, $items:expr) => {{
                let items: Vec<Item> = $items;
                if items.is_empty() {
                    pending.push($st);
                } else {
                    flush!();
                    out.extend(items);
                }
            }};
        }
        for &st in list {
            let at = At { file, node: st };
            match (l.stmt)(st, src) {
                Stmt::Return(e) => {
                    let items = self.ret(at, e.map(|e| At { file, node: e }), ctx);
                    add!(st, items);
                    break;
                }
                Stmt::Throw(e) => {
                    let items = self.throw(at, e.map(|e| At { file, node: e }), ctx);
                    add!(st, items);
                    break;
                }
                Stmt::If { cond, then, els } => {
                    let cond = At { file, node: cond };
                    let pre = self.expr(cond, ctx);
                    let then = self.stmts(file, &then, ctx);
                    let els = self.stmts(file, &els, ctx);
                    if !pre.is_empty() || !boring(&then) || !boring(&els) {
                        flush!();
                        out.extend(pre);
                        let id = self.id();
                        out.push(Item::Frame { id, kind: FrameKind::Alt, label: self.cond_text(cond), note: None, at: self.place(at), items: then, els: if els.is_empty() { None } else { Some(els) } });
                    } else {
                        pending.push(st);
                    }
                }
                // the rest of the block never runs
                Stmt::Jump => {
                    flush!();
                    out.push(Item::Own { id: self.id(), at: ctx.this.clone(), kind: SelfKind::Jump, label: one_line(self.text(at), 40), detail: None, place: self.place(at) });
                    break;
                }
                Stmt::Loop { body_node, body } => {
                    let items = self.stmts(file, &body, ctx);
                    // a loop that only skips rounds (`if not x: continue`) does nothing a story shows
                    if !only_jumps(&items) {
                        flush!();
                        let head = &src[st.start_byte()..body_node.start_byte()];
                        let id = self.id();
                        out.push(Item::Frame { id, kind: FrameKind::Loop, label: one_line(head.trim(), 48), note: None, at: self.place(at), items, els: None });
                    } else {
                        pending.push(st);
                    }
                }
                Stmt::Try { body, catches, finally } => {
                    flush!();
                    let mut items = self.stmts(file, &body, ctx);
                    // how the `try` can end, before the catches handle it: each catch takes what it handles, in order
                    let mut left = self.escaping(&items);
                    // what a catch handles does not end the function: the types the clauses name, or anything
                    if !catches.is_empty() {
                        let mut handled = Some(vec![]);
                        for (clause, _) in &catches {
                            match ((l.caught)(*clause, src), handled.as_mut()) {
                                (Some(types), Some(h)) => h.extend(types),
                                _ => handled = None,
                            }
                        }
                        self.handle(&mut items, handled.as_deref());
                    }
                    out.extend(items);
                    for (clause, c) in catches {
                        let types = (l.caught)(clause, src);
                        let (mut ends, rest): (Vec<End>, Vec<End>) = std::mem::take(&mut left).into_iter().partition(|e| match (&types, &e.ty) {
                            (None, _) => true,
                            (Some(t), Some(ty)) => t.contains(ty),
                            (Some(_), None) => false,
                        });
                        left = rest;
                        // a type it names that nothing here is seen to throw (the language's library or a package does)
                        for t in types.iter().flatten() {
                            if !ends.iter().any(|e| e.ty.as_ref() == Some(t)) {
                                ends.push(End { label: t.clone(), ty: Some(t.clone()) });
                            }
                        }
                        let inner = Ctx { catching: Some(Rc::new(Caught { name: (l.caught_as)(clause, src), ends })), ..ctx.clone() };
                        let items = self.stmts(file, &c, &inner);
                        if !boring(&items) {
                            // a plain catch written before it comes first
                            flush!();
                            let id = self.id();
                            // named by what it handles, when the clause names it
                            let label = (l.caught)(clause, src).map(|types| types.join(", ")).unwrap_or_default();
                            out.push(Item::Frame { id, kind: FrameKind::Catch, label, note: None, at: self.place(At { file, node: clause }), items, els: None });
                        } else {
                            pending.push(clause);
                        }
                    }
                    let items = self.stmts(file, &finally, ctx);
                    if !boring(&items) {
                        flush!();
                        out.extend(items);
                    }
                }
                Stmt::Block(list) => {
                    flush!();
                    let items = self.stmts(file, &list, ctx);
                    out.extend(items);
                }
                Stmt::Decl { .. } if self.names_function(At { file, node: st }) => {}
                Stmt::Decl { values, .. } | Stmt::Eval(values) => {
                    let mut items: Vec<Item> = values.into_iter().flat_map(|v| self.expr(At { file, node: v }, ctx)).collect();
                    items.extend(self.change_step(At { file, node: st }, ctx));
                    add!(st, items);
                }
                // a string on its own does nothing: it is documentation (or a directive), and comments and
                // documentation are never part of a story
                Stmt::Expr(e) if l.strings.nodes.contains(&e.kind()) => {}
                Stmt::Expr(e) => {
                    let mut items = self.expr(At { file, node: e }, ctx);
                    items.extend(self.change_step(At { file, node: st }, ctx));
                    add!(st, items);
                }
                // a nested function or class is declared here, not run here: it is no step. What it does is shown
                // where it is called, or as its own story.
                Stmt::Definition => {}
                Stmt::Other => pending.push(st),
            }
        }
        flush!();
        out
    }

    /// Plain statements that cross no boundary, folded into one step named after what they declare.
    pub fn local(&mut self, file: usize, list: &[Node<'w>], ctx: &Ctx) -> Item {
        let l = self.w.p.files[file].lang;
        let src = &self.w.p.files[file].text;
        let mut declared = vec![];
        let mut keyword = String::new();
        let mut first_decl = None;
        for (i, st) in list.iter().enumerate() {
            if let Stmt::Decl { keyword: k, names, .. } = (l.stmt)(*st, src) {
                if first_decl.is_none() {
                    first_decl = Some(i);
                    keyword = k;
                }
                // a part of a value is named as written (`data["kind"]`), not by the value's name
                for n in self.assigned_part(file, *st).map(|p| vec![p]).unwrap_or(names) {
                    if !declared.contains(&n) {
                        declared.push(n);
                    }
                }
            }
        }
        let bits: Vec<String> = list
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let at = At { file, node: *st };
                match (l.stmt)(*st, src) {
                    Stmt::Decl { .. } if first_decl == Some(i) => {
                        if keyword.is_empty() { format!("{} = …", declared.join(", ")) } else { format!("{keyword} {}", declared.join(", ")) }
                    }
                    Stmt::Decl { .. } => String::new(),
                    Stmt::If { cond, .. } => format!("if ({}) …", one_line(&self.cond_text(At { file, node: cond }), 30)),
                    Stmt::Expr(e) => one_line(self.text(At { file, node: e }), 40),
                    _ => one_line(self.text(at), 40),
                }
            })
            .filter(|b| !b.is_empty())
            .collect();
        let first = At { file, node: list[0] };
        let last = At { file, node: *list.last().unwrap() };
        Item::Own {
            id: self.id(),
            at: ctx.this.clone(),
            kind: SelfKind::Local,
            label: one_line(&bits.join("; "), 56),
            detail: Some(list.iter().map(|s| one_line(self.text(At { file, node: *s }), 0)).collect::<Vec<_>>().join("\n")),
            place: self.span(first, last),
        }
    }

    /// What an assignment statement gives a value to, as written, when that is a member or an index of a value
    /// (`data["kind"]`, `self.items`), not a name.
    fn assigned_part(&self, file: usize, st: Node<'w>) -> Option<String> {
        let f = &self.w.p.files[file];
        let l = f.lang;
        let assigned = |x: Node<'w>| l.assigns.iter().find(|(k, _)| *k == x.kind()).and_then(|(_, field)| x.child_by_field_name(field));
        let target = assigned(st).or_else(|| st.named_child(0).and_then(assigned))?;
        let root = self.root_node(file, target)?;
        f.text[root.end_byte()..target.end_byte()].starts_with(['.', '[', '?', '!']).then(|| one_line(f.text_of(target), 24))
    }

    pub fn ret(&mut self, st: At<'w>, e: Option<At<'w>>, ctx: &Ctx) -> Vec<Item> {
        let e = e.map(|e| self.unwrap(e));
        let at = self.place(st);
        if let (true, Some(e)) = (ctx.endpoint, e) {
            let adapters = self.w.adapters;
            if let Some(r) = adapters.list.iter().find_map(|a| a.response(self, e)) {
                let mut out: Vec<Item> = r.args.iter().flat_map(|a| self.expr(*a, ctx)).collect();
                // at the top of the story, whoever sent the request is not drawn: the answer stays on its lifeline
                let to = ctx.return_to.clone().unwrap_or_else(|| ctx.this.clone());
                let early = !self.tail(st);
                out.push(Item::Return { id: self.id(), from: ctx.this.clone(), to, label: r.label, detail: Some(one_line(self.text(st), 0)), at, throws: false, early, passes: None });
                return out;
            }
        }
        let mut pre = e.map(|e| self.expr(e, ctx)).unwrap_or_default();
        let label = e.map(|e| self.short(e)).unwrap_or("return".into());
        let early = !self.tail(st);
        match &ctx.return_to {
            Some(to) if *to != ctx.this => {
                pre.push(Item::Return { id: self.id(), from: ctx.this.clone(), to: to.clone(), label, detail: Some(one_line(self.text(st), 0)), at, throws: false, early, passes: None });
                pre
            }
            // the story's own function ends here, before its end: nothing visible receives it, so it stays on its
            // own lifeline. So does an answer that ends it only because what follows throws: its branch is the other
            // way out, which the throw's condition names. And what an endpoint answers is always shown: it is what the
            // request gets.
            None if (early || ctx.endpoint || self.before_throw(st)) && self.in_story_function(st, ctx) => {
                pre.push(Item::Return { id: self.id(), from: ctx.this.clone(), to: ctx.this.clone(), label, detail: Some(one_line(self.text(st), 0)), at, throws: false, early, passes: None });
                pre
            }
            _ => pre,
        }
    }

    /// A statement followed, at some level up to its function, by code that ends by throwing (`if ok: return data` …
    /// `raise …`).
    fn before_throw(&self, st: At<'w>) -> bool {
        let f = self.file(st);
        let l = f.lang;
        let mut cur = st.node;
        while let Some(p) = cur.parent() {
            if l.is_function(p.kind()) {
                return false;
            }
            if l.blocks.contains(&p.kind()) {
                let list = (l.body)(p);
                let after: Vec<_> = list.iter().skip_while(|s| **s != cur).skip(1).collect();
                if after.last().is_some_and(|s| matches!((l.stmt)(**s, &f.text), Stmt::Throw(_))) {
                    return true;
                }
            }
            cur = p;
        }
        false
    }

    /// Written directly in the story's own function (a screen, or the handler of a user action on it), not in a
    /// function inside it.
    fn in_story_function(&self, st: At<'w>, ctx: &Ctx) -> bool {
        let l = self.lang(st);
        let mut n = st.node.parent();
        while let Some(p) = n {
            if l.is_function(p.kind()) {
                return ctx.stack.contains(&self.at(st, p).key());
            }
            n = p.parent();
        }
        false
    }

    /// A statement at the end of its function: nothing written after it, at any level up to the function, would run
    /// had it not ended the function, other than code that ends by throwing (`return data; throw Error(…)`: the throw
    /// is the rule), and it stops no loop. A `return` that is not is an early exit, a rule. Where every branch
    /// returns, none of them is early.
    fn tail(&self, st: At<'w>) -> bool {
        let f = self.file(st);
        let l = f.lang;
        let mut cur = st.node;
        while let Some(p) = cur.parent() {
            if l.is_function(p.kind()) {
                return true;
            }
            if matches!((l.stmt)(p, &f.text), Stmt::Loop { .. }) {
                return false;
            }
            if l.blocks.contains(&p.kind()) {
                let list = (l.body)(p);
                let after = list.iter().skip_while(|s| **s != cur).skip(1).collect::<Vec<_>>();
                let throws = after.last().is_some_and(|s| matches!((l.stmt)(**s, &f.text), Stmt::Throw(_)));
                // a return anywhere in it (not in a function written inside) is another way through
                let returns = after.iter().any(|s| {
                    let mut stack = vec![**s];
                    while let Some(n) = stack.pop() {
                        if matches!((l.stmt)(n, &f.text), Stmt::Return(_)) {
                            return true;
                        }
                        let mut c = n.walk();
                        stack.extend(n.named_children(&mut c).filter(|k| !l.is_function(k.kind())));
                    }
                    false
                });
                if !after.is_empty() && !(throws && !returns) {
                    return false;
                }
            }
            cur = p;
        }
        true
    }

    /// `raise E(…)` / `throw new E(…)`: the function ends here, and so does each caller that does not catch it. Named
    /// by what the framework answers with (`403 …`), else by the type thrown (`E`), else by the statement. At the top
    /// of a story nothing visible catches it: it stays on the function's own lifeline.
    pub fn throw(&mut self, st: At<'w>, e: Option<At<'w>>, ctx: &Ctx) -> Vec<Item> {
        let e = e.map(|e| self.unwrap(e));
        let to = ctx.return_to.clone().unwrap_or_else(|| ctx.this.clone());
        // in a `catch`, throwing what it caught again (`rethrow`, a bare `raise`, `throw e`) passes on how the `try`
        // ended: no new way to end
        if let Some(c) = &ctx.catching
            && e.is_none_or(|e| c.name.as_deref() == Some(self.text(e)))
        {
            let id = self.id();
            self.ends.insert(id.clone(), c.ends.clone());
            let passes = Some(c.ends.iter().map(|e| e.label.clone()).collect());
            let detail = Some(one_line(self.text(st), 0));
            return vec![Item::Return { id, from: ctx.this.clone(), to, label: one_line(self.text(st), 40), detail, at: self.place(st), throws: true, early: false, passes }];
        }
        let ty = e.and_then(|e| self.thrown_type(e));
        let adapters = self.w.adapters;
        let label = e
            .and_then(|e| adapters.list.iter().find_map(|a| a.thrown(self, e)))
            .or_else(|| ty.clone())
            .unwrap_or_else(|| one_line(self.text(st), 40));
        let id = self.id();
        self.ends.insert(id.clone(), vec![End { label: label.clone(), ty }]);
        vec![Item::Return { id, from: ctx.this.clone(), to, label, detail: Some(one_line(self.text(st), 0)), at: self.place(st), throws: true, early: false, passes: None }]
    }

    /// The type a thrown value names: what is called or constructed (`E(…)`, `new E(…)`), or the name thrown (`E`).
    fn thrown_type(&self, e: At<'w>) -> Option<String> {
        let l = self.lang(e);
        let callee = match l.call_field(e.node.kind()) {
            Some(field) => l.unwrap(e.node.child_by_field_name(field)?),
            None => e.node,
        };
        (l.is_name(callee.kind()) || l.member_fields(callee.kind()).is_some()).then(|| crate::lang::last_part(self.text(self.at(e, callee))))
    }

    /// Steps inside a `try` whose catches handle `handled` (None: anything): a throw they handle is a plain step, and
    /// a call no longer ends with what they handle. What runs later (a frame of its own) is not inside the `try`.
    fn handle(&mut self, items: &mut [Item], handled: Option<&[String]>) {
        let catches = |end: &End| match (handled, &end.ty) {
            (None, _) => true,
            (Some(types), Some(t)) => types.contains(t),
            (Some(_), None) => false,
        };
        for it in items.iter_mut() {
            match it {
                Item::Return { id, from, detail, at, throws: true, .. } => {
                    if self.ends.get(id.as_str()).is_some_and(|e| e.iter().all(catches)) {
                        let label = one_line(detail.as_deref().unwrap_or(""), 56);
                        *it = Item::Own { id: id.clone(), at: from.clone(), kind: SelfKind::Local, label, detail: detail.clone(), place: at.clone() };
                    }
                }
                Item::Call(c) => {
                    if let Some(e) = self.ends.get_mut(&c.id) {
                        e.retain(|x| !catches(x));
                        c.ends = e.iter().map(|x| x.label.clone()).collect();
                    }
                }
                Item::Frame { kind: FrameKind::Effect, .. } => {}
                Item::Frame { items, els, .. } => {
                    self.handle(items, handled);
                    if let Some(els) = els {
                        self.handle(els, handled);
                    }
                }
                _ => {}
            }
        }
    }

    /// How the steps can end the function they are in besides returning: what they throw and what the calls among
    /// them end with, each once. What runs later (a frame of its own) ends nothing here.
    fn escaping(&self, items: &[Item]) -> Vec<End> {
        let mut out: Vec<End> = vec![];
        fn walk(b: &Builder, items: &[Item], out: &mut Vec<End>) {
            for it in items {
                match it {
                    Item::Return { id, throws: true, .. } | Item::Call(Call { id, .. }) => {
                        for e in b.ends.get(id.as_str()).into_iter().flatten() {
                            if !out.iter().any(|x| x.label == e.label) {
                                out.push(e.clone());
                            }
                        }
                    }
                    Item::Frame { kind: FrameKind::Effect, .. } => {}
                    Item::Frame { items, els, .. } => {
                        walk(b, items, out);
                        walk(b, els.as_deref().unwrap_or(&[]), out);
                    }
                    _ => {}
                }
            }
        }
        walk(self, items, &mut out);
        out
    }

    /// A call that has walked its callee's body: how it can end, from that body.
    fn ends_with(&mut self, call: &mut Call, body: &[Item]) {
        let ends = self.escaping(body);
        call.ends = ends.iter().map(|e| e.label.clone()).collect();
        self.ends.insert(call.id.clone(), ends);
    }

    // ---------- expressions ----------

    pub fn expr(&mut self, e: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let l = self.lang(e);
        if l.is_function(e.node.kind()) {
            return vec![];
        }
        let adapters = self.w.adapters;
        for a in &adapters.list {
            if let Some(items) = a.expr(self, e, ctx) {
                return items;
            }
        }
        match (l.expr)(e.node, &self.file(e).text) {
            Expr::Call { callee, args } => {
                if let Some(items) = self.call(e, self.at(e, callee), &args, ctx) {
                    return items;
                }
            }
            Expr::Cond { cond, then, els, negated } => {
                let cond = self.at(e, cond);
                let pre = self.expr(cond, ctx);
                let a = self.expr(self.at(e, then), ctx);
                let b = els.map(|x| self.expr(self.at(e, x), ctx)).unwrap_or_default();
                if boring(&a) && boring(&b) {
                    return pre;
                }
                let c = one_line(self.text(cond), 0);
                let label = if negated { format!("not ({c})") } else { c };
                let mut out = pre;
                out.push(Item::Frame { id: self.id(), kind: FrameKind::Alt, label, note: None, at: self.place(e), items: a, els: if b.is_empty() { None } else { Some(b) } });
                return out;
            }
            Expr::Function => return vec![],
            Expr::Other => {}
        }
        let mut out = vec![];
        let mut c = e.node.walk();
        let kids: Vec<Node<'w>> = e.node.named_children(&mut c).collect();
        for k in kids {
            out.extend(self.expr(self.at(e, k), ctx));
        }
        out
    }

    pub fn call(&mut self, e: At<'w>, callee: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Option<Vec<Item>> {
        let adapters = self.w.adapters;
        let ch = self.chain(e);
        let mut before: Vec<Item> = vec![];
        for a in &adapters.list {
            before.extend(a.before(self, e, &ch, ctx));
        }
        let out = self.call_steps(e, callee, args, ctx);
        if before.is_empty() {
            return out;
        }
        // what the call makes run comes first, then what it shows itself; a call shown as nothing still walks what is in it
        before.extend(match out {
            Some(items) => items,
            None => {
                let mut c = e.node.walk();
                let kids: Vec<Node<'w>> = e.node.named_children(&mut c).collect();
                kids.into_iter().flat_map(|k| self.expr(self.at(e, k), ctx)).collect()
            }
        });
        Some(before)
    }

    /// Something a walk shows once: the first time `key` is asked about it is true.
    pub fn first_time(&mut self, key: NodeKey) -> bool {
        self.once.insert(key)
    }

    fn call_steps(&mut self, e: At<'w>, callee: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Option<Vec<Item>> {
        let l = self.lang(e);
        let c = self.unwrap(callee);
        if l.is_function(c.node.kind()) {
            return Some(self.callback(c, ctx));
        }
        let ch = self.chain(e);
        if let Some(last) = ch.segs.last().filter(|s| s.node == l.unwrap(e.node)) {
            if l.promise.contains(&last.name.as_str()) {
                return Some(self.promise(e, ctx));
            }
            if l.iterate.contains(&last.name.as_str()) {
                if let Some(f) = last.args.as_ref().and_then(|a| a.first()).filter(|f| l.is_function(l.unwrap(**f).kind())) {
                    if let Some(items) = self.iteration(e, self.at(e, *f), &last.name, ctx) {
                        return Some(items);
                    }
                }
            }
        }
        let adapters = self.w.adapters;
        for a in &adapters.list {
            if let Some(items) = a.call(self, e, &ch, ctx) {
                return Some(items);
            }
        }
        let target = self.w.res.target(callee.file, callee.node, adapters);
        if let Target::Unresolved(why) = &target
            && why.starts_with("parameter")
            && let Some(items) = self.bound_call(e, callee, ctx)
        {
            return Some(items);
        }
        let unresolved = match target {
            Target::Own { func, hops, via } => {
                let versions = self.overrides(e, func);
                if !versions.is_empty() {
                    return Some(self.dispatch(e, func, &hops, versions, ctx));
                }
                return Some(self.with_via(func, via, |b| b.project_call(e, func, &hops, ctx)));
            }
            Target::Unresolved(why) => Some(why),
            Target::Declared(_) => None,
            Target::Outside(loc) => {
                // a name the project's own folder declares in a file that holds no code to read (a stub, `.pyi`): what
                // runs is written elsewhere (or made while it runs), and is not in the story
                if self.w.p.is_own(&loc.file) && self.package_of(e, &ch).is_none() && !ctx.quiet {
                    let rel = loc.file.strip_prefix(&self.w.p.root).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| loc.file.to_string_lossy().into_owned());
                    let name = crate::resolve::callee_name(callee.node, l).map(|n| self.text(self.at(callee, n)).to_string()).unwrap_or_default();
                    let at = self.place(e);
                    self.gap(format!("{name}: declared in {rel}, which holds no code to follow (made while the program runs, or written elsewhere)"), at);
                }
                None
            }
        };
        let handled = self.package_of(e, &ch).is_some_and(|(p, _)| adapters.list.iter().any(|a| a.reads(l) && a.handles(&p)));
        let drawn = self.package_call(e, &ch, ctx);
        if drawn.is_none() && !ctx.quiet {
            self.not_followed(e, callee, unresolved);
        }
        let handed = if handled || ctx.quiet { vec![] } else { self.handed(e, args, ctx) };
        match (drawn, handed.is_empty()) {
            (drawn, true) => drawn,
            (drawn, false) => Some(drawn.into_iter().flatten().chain(handed).collect()),
        }
    }

    /// A call to a function taken from what the project's code returns or provides (`const { save } = useStore()`)
    /// that is not found there: a gap, never a silent plain statement. A value only known at runtime otherwise (a
    /// function the caller gives, one stored in a list or a field) is not one: what it runs depends on the run.
    fn not_followed(&mut self, e: At<'w>, callee: At<'w>, why: Option<String>) {
        let Some(why) = why.filter(|w| w.starts_with("taken from")) else { return };
        if self.package_of(e, &self.chain(e)).is_some() {
            return;
        }
        let name = crate::resolve::callee_name(callee.node, self.lang(callee)).map(|n| self.text(self.at(callee, n)).to_string()).unwrap_or_default();
        let at = self.place(e);
        self.gap(format!("{}: where {name} is written is not resolved ({why})", one_line(self.text(e), 40)), at);
    }

    /// The class a method is written in, if it is one.
    fn class_of(&self, decl: At<'w>) -> Option<At<'w>> {
        let l = self.lang(decl);
        let mut n = decl.node.parent();
        while let Some(x) = n {
            if l.reach.classes.contains(&x.kind()) {
                return Some(self.at(decl, x));
            }
            if l.is_function(x.kind()) {
                return None;
            }
            n = x.parent();
        }
        None
    }

    /// The versions of the method `decl` that the project's classes extending its class write themselves: a call to
    /// it at `site` can run any of them, depending on the object's class at runtime. Classes written in tests (test
    /// doubles) count only for a call written in a test.
    pub fn overrides(&self, site: At<'w>, decl: At<'w>) -> Vec<At<'w>> {
        let l = self.lang(decl);
        let (Some(class), Some(name)) = (self.class_of(decl), (l.function_name)(decl.node)) else { return vec![] };
        let name = self.text(self.at(decl, name));
        if self.on_class(site) {
            return vec![];
        }
        let test = |file: usize| self.w.adapters.list.iter().any(|a| a.test(&self.w.p.files[file]));
        let in_test = test(site.file);
        let mut out = vec![];
        for sub in self.w.subclasses(class).into_iter().filter(|s| in_test || !test(s.file)) {
            let sf = self.file(sub);
            for m in (sf.lang.reach.members)(sub.node).into_iter().filter(|m| sf.text_of(*m) == name) {
                // a method, or a field given a function (`load = async (id) => …`)
                let field = || sf.lang.declared(m).and_then(|(_, v)| v).map(|v| sf.lang.unwrap(v)).filter(|v| sf.lang.is_function(v.kind()));
                if let Some(d) = sf.lang.function_of(m).or_else(field) {
                    out.push(self.at(sub, d));
                }
            }
        }
        out
    }

    /// A call made on a class itself (`Item.create()`: a static method, a constructor), not on one of its objects: it
    /// reaches that class's member only.
    fn on_class(&self, site: At<'w>) -> bool {
        let l = self.lang(site);
        let Expr::Call { callee, .. } = (l.expr)(l.unwrap(site.node), &self.file(site).text) else { return false };
        let c = l.unwrap(callee);
        let Some(object) = l.member_fields(c.kind()).and_then(|(obj, _)| c.child_by_field_name(obj)) else { return false };
        let Some(name) = crate::resolve::callee_name(object, l) else { return false };
        let Some(loc) = self.w.res.definition(site.file, name).into_iter().next() else { return false };
        let Some(file) = self.w.p.file_index(&loc.file) else { return false };
        let f = &self.w.p.files[file];
        let at = f.node_at(loc.line as usize, loc.col);
        at.and_then(|n| n.parent()).is_some_and(|p| f.lang.reach.classes.contains(&p.kind()) && p.child_by_field_name("name") == at)
    }

    /// A method with nothing of its own to run: no body, or only documentation, `pass`, `...` or a statement that
    /// throws (an abstract method).
    fn is_abstract(&self, decl: At<'w>) -> bool {
        let l = self.lang(decl);
        let src = &self.file(decl).text;
        let Some(body) = (l.function_body)(decl.node) else { return true };
        if !l.blocks.contains(&body.kind()) {
            return false;
        }
        (l.body)(body).into_iter().all(|st| match (l.stmt)(st, src) {
            Stmt::Other => true,
            Stmt::Expr(x) => !matches!((l.expr)(x, src), Expr::Call { .. }),
            _ => false,
        })
    }

    /// A call to a method that classes of the project write their own versions of. Which one runs depends on the
    /// object's class at runtime, so each version is an alternative named by its class: the method called first
    /// (unless it has nothing of its own to run), then each version. A version's arrow is inferred.
    fn dispatch(&mut self, e: At<'w>, decl: At<'w>, hops: &[String], versions: Vec<At<'w>>, ctx: &Ctx) -> Vec<Item> {
        let class_name = |b: &Self, d: At<'w>| b.class_of(d).and_then(|c| c.node.child_by_field_name("name")).map(|n| b.text(b.at(d, n)).to_string()).unwrap_or_else(|| b.decl_name(d));
        let called = format!("{}.{}", class_name(self, decl), self.decl_name(decl));
        let mut alts: Vec<(String, Vec<Item>)> = vec![];
        if !self.is_abstract(decl) {
            let items = self.project_call(e, decl, hops, ctx);
            alts.push((class_name(self, decl), items));
        }
        for v in versions {
            let mut items = self.project_call(e, v, &[], ctx);
            let class = class_name(self, v);
            for it in items.iter_mut() {
                if let Item::Call(c) = it {
                    c.link = Link { kind: LinkKind::Inferred, how: format!("the code calls {called}; {class} extends that class and writes its own") };
                }
            }
            alts.push((class, items));
        }
        // the versions share a name: each column is named by its class too
        for d in std::iter::once(decl).chain(self.overrides(e, decl)) {
            let (id, name) = (self.pid(d), format!("{}.{}", class_name(self, d), self.decl_name(d)));
            if let Some(p) = self.part_mut(&id) {
                p.name = name;
            }
        }
        let note = format!("{} runs the version of the object's class, known only at runtime", one_line(self.text(e), 40));
        let at = self.place(e);
        let mut els: Option<Vec<Item>> = None;
        for (label, items) in alts.into_iter().rev() {
            let frame = Item::Frame { id: self.id(), kind: FrameKind::Alt, label, note: Some(note.clone()), at: at.clone(), items, els };
            els = Some(vec![frame]);
        }
        els.unwrap_or_default()
    }

    /// Functions of the project's own handed by name to a call outside it, where that call's signature (as the
    /// language server shows it) takes a function (`executor.submit(job, x)`, `Thread(target=job)`): that call decides
    /// when they run, so each is a frame of its own, named by the call. A function only stored (`d.setdefault(k, f)`)
    /// or a call whose signature is unknown hands nothing on. It is shown only when what the function does crosses a
    /// boundary; a screen or an endpoint handed on is not, since it has its own story where its framework runs it.
    fn handed(&mut self, e: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Vec<Item> {
        let l = self.lang(e);
        let Expr::Call { callee, .. } = (l.expr)(l.unwrap(e.node), &self.file(e).text) else { return vec![] };
        let mut out = vec![];
        let mut params: Option<Vec<crate::lang::Param>> = None;
        let mut position = 0;
        for a in args {
            // a named argument (`target=job`) hands over its value
            let kw = (l.named_arg)(*a, &self.file(e).text);
            let named = kw.map(|(n, _)| self.text(self.at(e, n)).to_string());
            let at = position;
            if named.is_none() {
                position += 1;
            }
            let v = l.unwrap(kw.map(|(_, v)| v).unwrap_or(*a));
            if !l.is_name(v.kind()) && l.member_fields(v.kind()).is_none() {
                continue;
            }
            let route = |d: At<'w>| self.w.routes.iter().any(|r| r.decl.key() == d.key());
            let Some((decl, hops)) = self.project_target(self.at(e, v)).filter(|(d, _)| self.is_function(*d) && !route(*d)) else { continue };
            let params = params.get_or_insert_with(|| {
                let sig = crate::resolve::callee_name(callee, l).and_then(|n| self.w.res.hover(e.file, n));
                sig.map(|s| (l.signature)(&s).0).unwrap_or_default()
            });
            let takes = crate::lang::param_for(params, at, named.as_deref()).is_some_and(|p| l.callable.iter().any(|m| p.ty.contains(m)));
            if !takes {
                continue;
            }
            let Some(items) = self.trial(|b| b.project_call(b.at(e, v), decl, &hops, ctx)) else { continue };
            let name = one_line(self.text(self.at(e, callee)), 32);
            out.push(Item::Frame { id: self.id(), kind: FrameKind::Effect, label: one_line(self.text(e), 48), note: Some(format!("when {name} runs it")), at: self.place(e), items, els: None });
        }
        out
    }

    /// A call into an installed package crosses the project's edge: an arrow to the package. Where a chain of calls
    /// leaves the project decides (`db.table(…).select(…)` belongs to the package `table` comes from); a chain that
    /// leaves it into the language's own library is not drawn. What the call is given is walked first.
    /// The call is written inside the arguments of another call into a package, in the same statement, and nothing waits
    /// for it: a value being built (a column, a schema, a condition), not work of its own.
    fn builds_value_for_package(&self, e: At<'w>) -> bool {
        let l = self.lang(e);
        let src = &self.file(e).text;
        let mut child = e.node;
        while let Some(p) = child.parent() {
            if l.is_function(p.kind()) || !matches!((l.stmt)(p, src), Stmt::Other) {
                return false;
            }
            // waited for: a real request or query
            if p.kind().starts_with("await") {
                return false;
            }
            if let Some(field) = l.call_field(p.kind()) {
                let inside_callee = p.child_by_field_name(field).is_some_and(|c| c.start_byte() <= child.start_byte() && child.end_byte() <= c.end_byte());
                if !inside_callee {
                    let outer = self.at(e, p);
                    return self.package_of(outer, &self.chain(outer)).is_some();
                }
            }
            child = p;
        }
        false
    }

    pub fn package_call(&mut self, e: At<'w>, ch: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        if ctx.quiet {
            return None;
        }
        let (package, link) = self.package_of(e, ch)?;
        let l = self.lang(e);
        if self.w.adapters.list.iter().any(|a| a.reads(l) && a.handles(&package)) {
            return None;
        }
        let mut items = vec![];
        // the root: a call of a plain name gives its arguments (a call of the project's own is walked as itself);
        // any other expression is walked as itself; a plain name gives nothing
        match self.root_args(e, ch) {
            Some((callee, args)) if ch.root == l.unwrap(e.node) || self.project_target(self.at(e, callee)).is_none() => {
                for a in args {
                    items.extend(self.expr(self.at(e, a), ctx));
                }
            }
            Some(_) => items.extend(self.expr(self.at(e, ch.root), ctx)),
            None if l.is_name(ch.root.kind()) => {}
            None => items.extend(self.expr(self.at(e, ch.root), ctx)),
        }
        for s in &ch.segs {
            for a in s.args.iter().flatten() {
                items.extend(self.expr(self.at(e, *a), ctx));
            }
        }
        // a value built to be given to another package's call (`op.create_table(t, sa.Column(…))`) is part of that call
        if self.builds_value_for_package(e) {
            return Some(items);
        }
        let to = self.part(Participant { id: format!("pkg:{package}"), name: package.clone(), kind: PartKind::Service, group: Group::Outside, sub: Some("package".into()), at: None });
        let code = one_line(self.text(e), 0);
        let (unused, into) = self.result_use(e);
        items.push(Item::Call(Call {
            id: self.id(),
            from: ctx.this.clone(),
            to,
            label: one_line(&code, 56),
            detail: Some(code),
            link,
            effect: None,
            at: self.place(e),
            target: None,
            body: None,
            note: None,
            opens: None,
            story: None,
            table: None,
            touches: None,
            unused,
            into,
            ends: vec![],
            changes: vec![],
        }));
        Some(items)
    }

    /// The callee and arguments of a chain's root when the root is a call of a plain name (`get(url)`).
    fn root_args(&self, e: At<'w>, ch: &Chain<'w>) -> Option<(Node<'w>, Vec<Node<'w>>)> {
        let l = self.lang(e);
        l.call_field(ch.root.kind())?;
        match (l.expr)(ch.root, &self.file(e).text) {
            Expr::Call { callee, args } => Some((callee, args)),
            _ => None,
        }
    }

    /// The installed package a chain of calls goes into, and how that is known: the first name along the chain that
    /// the language server places outside the project decides. A chain that starts at a name imported from a package
    /// that is not installed counts as that package, shown by name only.
    pub fn package_of(&self, e: At<'w>, ch: &Chain<'w>) -> Option<(String, Link)> {
        let f = self.file(e);
        let l = f.lang;
        let res = self.w.res;
        let root = match self.root_args(e, ch) {
            Some((callee, _)) => l.unwrap(callee),
            None => ch.root,
        };
        // the names along the chain, from its root outwards
        let mut names: Vec<Node<'w>> = crate::resolve::callee_name(root, l).into_iter().collect();
        for s in &ch.segs {
            let m = match l.call_field(s.node.kind()) {
                Some(fd) => s.node.child_by_field_name(fd).map(|c| l.unwrap(c)),
                None => Some(s.node),
            };
            names.extend(m.and_then(|m| l.member_fields(m.kind()).and_then(|(_, prop)| m.child_by_field_name(prop))));
        }
        for n in names {
            let Some(loc) = res.definition(e.file, n).into_iter().next() else { continue };
            if self.w.p.is_own(&loc.file) {
                continue;
            }
            let package = (l.package)(&loc.file)?;
            let how = format!("the language server places it in the installed package {package}");
            return Some((package, Link { kind: LinkKind::Exact, how }));
        }
        // the plain name the chain starts at, imported from a package whose import reaches nothing. A name the server
        // cannot follow (one the module takes from a package that is missing) may still come from a module that is
        // there, the project's own or installed: the module is asked too.
        let mut first = root;
        while let Some((obj, _)) = l.member_fields(first.kind()) {
            first = l.unwrap(first.child_by_field_name(obj)?);
        }
        if !l.is_name(first.kind()) {
            return None;
        }
        // a name of this file given what a function of such a package returns (`const { auth } = useAuth()`,
        // `client = Client()`), followed back a few steps to the name imported
        let mut name = first;
        let mut via: Vec<String> = vec![];
        let imp = loop {
            if let Some(i) = f.import_at(f.text_of(name), name) {
                break i;
            }
            if via.len() >= 3 {
                return None;
            }
            let loc = res.definition(e.file, name).into_iter().next().filter(|loc| loc.file == f.abs)?;
            let at = f.node_at(loc.line as usize, loc.col)?;
            // a value of a type written where it is declared (`(db: Client)`), or a type that stands for another: the
            // type's own name, where it comes from
            if let Some(v) = (l.typed)(at) {
                if v == name {
                    return None;
                }
                via.push(format!("{} is of the type {}", f.text_of(name), f.text_of(v)));
                name = v;
                continue;
            }
            let decl = std::iter::successors(at.parent(), |n| n.parent()).take(3).find(|n| l.follow.declarator(n.kind()).is_some())?;
            let mut v = l.unwrap(decl.child_by_field_name(l.follow.declarator(decl.kind())?.1)?);
            if let Some(cf) = l.call_field(v.kind()) {
                v = l.unwrap(v.child_by_field_name(cf)?);
            }
            while let Some((obj, _)) = l.member_fields(v.kind()) {
                v = l.unwrap(v.child_by_field_name(obj)?);
            }
            if !l.is_name(v.kind()) || v == name {
                return None;
            }
            via.push(format!("{} is made by {}", f.text_of(name), f.text_of(v)));
            name = v;
        };
        // a value it made, or one of its types, is followed only into a method called on it (`client.send()`): one
        // read off it first (`result.items.map(…)`) is data, whose methods are the language's
        if imp.is_path() || (!via.is_empty() && ch.segs.first().is_some_and(|s| s.args.is_none())) {
            return None;
        }
        let reaches = |(row, col): (usize, usize)| res.definition_at(e.file, row, col).into_iter().any(|loc| loc.file != f.abs);
        if reaches((imp.line, imp.col)) || reaches(imp.module_at) {
            return None;
        }
        let package = crate::resolve::package_of(&imp.module, l.package_sep);
        let from = if via.is_empty() { String::new() } else { format!("{}, ", via.join(", ")) };
        Some((package, Link { kind: LinkKind::Inferred, how: format!("{from}imported from {}, which is not installed", imp.module) }))
    }

    /// `p.then(…).then(…).catch(…)`: the base call first, then each handler in order.
    pub fn promise(&mut self, e: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let l = self.lang(e);
        let src = &self.file(e).text;
        let mut handlers = vec![];
        let mut cur = l.unwrap(e.node);
        loop {
            let Expr::Call { callee, args } = (l.expr)(cur, src) else { break };
            let c = l.unwrap(callee);
            let Some((obj, prop)) = l.member_fields(c.kind()) else { break };
            let (Some(o), Some(p)) = (c.child_by_field_name(obj), c.child_by_field_name(prop)) else { break };
            let name = &src[p.byte_range()];
            if !l.promise.contains(&name) {
                break;
            }
            handlers.insert(0, (name.to_string(), args.first().copied()));
            cur = l.unwrap(o);
        }
        let mut out = self.expr(self.at(e, cur), ctx);
        let failure = l.promise.last().copied().unwrap_or("");
        for (name, f) in handlers {
            let Some(f) = f.map(|f| self.unwrap(self.at(e, f))).filter(|f| self.is_function(*f)) else { continue };
            let inner = Ctx { return_to: None, ..ctx.clone() };
            let items = self.callback(f, &inner);
            // what handles a failure (`.catch(…)`) handles any
            if name == failure {
                out.push(Item::Frame { id: self.id(), kind: FrameKind::Catch, label: String::new(), note: None, at: self.place(f), items, els: None });
            } else {
                out.extend(items);
            }
        }
        out
    }

    /// `xs.map(x => …)` with something worth showing inside becomes a loop frame.
    pub fn iteration(&mut self, e: At<'w>, f: At<'w>, method: &str, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = self.unwrap(f);
        let inner = Ctx { return_to: None, in_loop: true, ..ctx.clone() };
        let items = self.callback(f, &inner);
        if boring(&items) {
            return None;
        }
        let l = self.lang(e);
        let callee = l.unwrap(e.node.child_by_field_name(l.call_field(e.node.kind())?)?);
        let (obj, _) = l.member_fields(callee.kind())?;
        let target = self.at(e, callee.child_by_field_name(obj)?);
        let params = (l.params)(f.node, &self.file(f).text).join(", ");
        let mut out = self.expr(target, ctx);
        out.push(Item::Frame { id: self.id(), kind: FrameKind::Loop, label: format!("{}.{method}({params})", one_line(self.text(target), 24)), note: None, at: self.place(e), items, els: None });
        Some(out)
    }

    /// The body of a function, callback or handler, walked as the current participant.
    pub fn callback(&mut self, f: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let l = self.lang(f);
        let Some(body) = (l.function_body)(f.node) else { return vec![] };
        if l.blocks.contains(&body.kind()) {
            let list = (l.body)(body);
            return self.stmts(f.file, &list, ctx);
        }
        let items = self.expr(self.at(f, body), ctx);
        if !items.is_empty() {
            return items;
        }
        vec![Item::Own {
            id: self.id(),
            at: ctx.this.clone(),
            kind: SelfKind::Local,
            label: one_line(self.text(self.at(f, body)), 48),
            detail: Some(one_line(self.text(f), 0)),
            place: self.place(f),
        }]
    }

    // ---------- project functions ----------

    fn nested_in(&self, decl: At<'w>, stack: &[NodeKey]) -> bool {
        let mut n = decl.node.parent();
        while let Some(p) = n {
            if stack.contains(&At { file: decl.file, node: p }.key()) {
                return true;
            }
            n = p.parent();
        }
        false
    }

    /// The parameters a function is given, as it names them, without its own object; and the name of its object, where
    /// the language passes it as the first parameter (Python's `self`).
    fn params_of(&self, decl: At<'w>) -> (Vec<String>, Option<String>) {
        let f = self.file(decl);
        let mut params = (f.lang.params)(decl.node, &f.text);
        let own = match f.lang.own {
            Own::FirstParam if self.is_method(decl) && !params.is_empty() => Some(params.remove(0)),
            _ => None,
        };
        (params, own)
    }

    /// A value something else holds too: a name, or a member or an index of one (`cart`, `self.items[k]`), not one
    /// made where it is written (a literal, a call).
    fn held(&self, file: usize, v: Node<'w>) -> bool {
        let text = &self.w.p.files[file].text;
        self.root_node(file, v).is_some_and(|r| {
            let rest = &text[r.end_byte()..v.end_byte()];
            rest.is_empty() || (rest.starts_with(['.', '[', '?', '!']) && !rest.ends_with(')'))
        })
    }

    /// The name or keyword a target starts at (`p` in `p.x[k]`, `this` in `this.items`).
    fn root_node(&self, file: usize, n: Node<'w>) -> Option<Node<'w>> {
        let l = self.w.p.files[file].lang;
        let keyword = |n: Node| matches!(l.own, Own::Keyword(k) if n.kind() == k);
        let mut cur = l.unwrap(n);
        while !l.is_name(cur.kind()) && !keyword(cur) {
            cur = l.unwrap(cur.named_child(0)?);
        }
        Some(cur)
    }

    /// What a target written in `func` reaches from outside it: a value it is given, or its own object (named by its
    /// parameter or keyword, or a field written without it, which the language server places in a class). `whole`:
    /// the target may be that value itself (a method called on it), not only a part of it (written into): giving a
    /// parameter a new value changes nothing the caller holds.
    fn root_of(&self, func: At<'w>, target: Node<'w>, whole: bool) -> Option<Change> {
        let f = self.file(func);
        let l = f.lang;
        let cur = self.root_node(func.file, target)?;
        let rest = &f.text[cur.end_byte()..target.end_byte()];
        // only a member or an index of it (`a, b = …` gives names new values)
        if !rest.is_empty() && !rest.starts_with(['.', '[', '?', '!']) {
            return None;
        }
        let path = one_line(rest, 30);
        let name = f.text_of(cur);
        let (params, own) = self.params_of(func);
        if matches!(l.own, Own::Keyword(k) if cur.kind() == k) || own.as_deref() == Some(name) {
            return (whole || !path.is_empty()).then_some(Change { root: Root::Own, path });
        }
        if params.iter().any(|p| p == name) {
            return (whole || !path.is_empty()).then(|| Change { root: Root::Param(name.to_string()), path });
        }
        (self.is_field(At { file: func.file, node: cur }) || self.is_inherited_field(func, At { file: func.file, node: cur }))
            .then(|| Change { root: Root::Own, path: one_line(&f.text[cur.start_byte()..target.end_byte()], 30) })
    }

    /// A name written in a method, without its object, that the language server places outside the project: a field
    /// of a class the project's class extends (a framework's `state`). A name a method gives a value is a local of
    /// it where it is declared there, so this is no local.
    fn is_inherited_field(&self, func: At<'w>, name: At<'w>) -> bool {
        self.is_method(func) && self.w.res.definition(name.file, name.node).into_iter().next().is_some_and(|loc| !self.w.p.is_own(&loc.file))
    }

    /// A name the language server places directly in a class (a field), not in a function or at the top of a file.
    fn is_field(&self, name: At<'w>) -> bool {
        let Some(loc) = self.w.res.definition(name.file, name.node).into_iter().next() else { return false };
        let Some((file, node)) = self.w.res.at_loc(&loc) else { return false };
        let l = self.w.p.files[file].lang;
        let mut n = Some(node);
        while let Some(p) = n {
            if l.is_function(p.kind()) {
                return false;
            }
            if l.reach.classes.contains(&p.kind()) {
                return true;
            }
            n = p.parent();
        }
        false
    }

    /// What a statement or an assignment written in `func` changes of what reaches it from outside: something written
    /// into it, or a method of the language's own library called on it as a statement of its own, for its effect
    /// (`items.append(x)`). A call into the project or a package is an arrow of its own, and says what it changes
    /// there; one the language server cannot follow is not taken for a change.
    fn changed_at(&self, func: At<'w>, n: Node<'w>) -> Option<Change> {
        let f = self.file(func);
        let l = f.lang;
        let assigned = |x: Node<'w>| l.assigns.iter().find(|(k, _)| *k == x.kind()).and_then(|(_, field)| x.child_by_field_name(field));
        if let Some(target) = assigned(n).or_else(|| n.named_child(0).and_then(assigned)) {
            return self.root_of(func, target, false);
        }
        let Stmt::Expr(e) = (l.stmt)(n, &f.text) else { return None };
        let Expr::Call { callee, .. } = (l.expr)(l.unwrap(e), &f.text) else { return None };
        let callee = l.unwrap(callee);
        let object = l.member_fields(callee.kind()).and_then(|(obj, _)| callee.child_by_field_name(obj))?;
        match self.w.res.target(func.file, callee, self.w.adapters) {
            Target::Outside(loc) if (l.package)(&loc.file).is_none() => self.root_of(func, object, true),
            _ => None,
        }
    }

    /// What a function changes of what reaches it from outside: written in it, or changed by the project's functions
    /// it calls (followed through them), each once, in the order written. Functions written inside run elsewhere.
    pub fn changes_of(&mut self, decl: At<'w>) -> Vec<Change> {
        if let Some(c) = self.changes.get(&decl.key()) {
            return c.clone();
        }
        // a function that reaches itself again adds nothing more
        self.changes.insert(decl.key(), vec![]);
        let f = self.file(decl);
        let l = f.lang;
        let mut nodes = vec![];
        let mut stack: Vec<Node<'w>> = (l.function_body)(decl.node).into_iter().collect();
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c).filter(|ch| !l.is_function(ch.kind())));
            nodes.push(n);
        }
        let calls: Vec<(Node<'w>, Node<'w>)> = nodes.iter().filter_map(|n| match (l.expr)(*n, &f.text) {
            Expr::Call { callee, .. } => Some((*n, callee)),
            _ => None,
        }).collect();
        self.w.res.prefetch(&calls.iter().filter_map(|(_, c)| Some((decl.file, crate::resolve::callee_name(*c, l)?))).collect::<Vec<_>>());
        let mut found: Vec<(usize, Change)> = nodes.iter().filter_map(|n| Some((n.start_byte(), self.changed_at(decl, *n)?))).collect();
        for (call, callee) in calls {
            let Target::Own { func, .. } = self.w.res.target(decl.file, callee, self.w.adapters) else { continue };
            for ch in self.changes_of(func) {
                if let Some(c) = self.carried(At { file: decl.file, node: call }, func, &ch, decl) {
                    found.push((call.start_byte(), c));
                }
            }
        }
        found.sort_by_key(|(at, _)| *at);
        let mut out: Vec<Change> = vec![];
        for (_, c) in found {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        self.changes.insert(decl.key(), out.clone());
        out
    }

    /// The value a call gives for what its callee changes: the argument for a parameter (by name, else by position),
    /// or the object a method is called on. Some(None): a method called without its object, on the caller's own.
    fn value_for(&self, call: At<'w>, callee: At<'w>, root: &Root) -> Option<Option<Node<'w>>> {
        let f = self.file(call);
        let l = f.lang;
        let Expr::Call { callee: c, args } = (l.expr)(l.unwrap(call.node), &f.text) else { return None };
        match root {
            Root::Param(name) => {
                let named = args.iter().find_map(|a| (l.named_arg)(*a, &f.text).filter(|(n, _)| f.text_of(*n) == name).map(|(_, v)| v));
                let positional: Vec<Node<'w>> = args.iter().copied().filter(|a| (l.named_arg)(*a, &f.text).is_none()).collect();
                let at = self.params_of(callee).0.iter().position(|p| p == name).and_then(|i| positional.get(i).copied());
                named.or(at).map(Some)
            }
            Root::Own => {
                let c = l.unwrap(c);
                match l.member_fields(c.kind()) {
                    Some((obj, _)) => c.child_by_field_name(obj).map(Some),
                    None => (self.is_method(callee) && self.function_of(call).is_some_and(|f| self.is_method(f))).then_some(None),
                }
            }
        }
    }

    /// A change a called function makes, in the caller's terms, when what it changes reaches the caller from outside
    /// too.
    fn carried(&self, call: At<'w>, callee: At<'w>, ch: &Change, caller: At<'w>) -> Option<Change> {
        match self.value_for(call, callee, &ch.root)? {
            Some(v) => {
                let base = self.root_of(caller, v, true)?;
                Some(Change { root: base.root, path: join_path(&base.path, &ch.path) })
            }
            None => Some(Change { root: Root::Own, path: ch.path.clone() }),
        }
    }

    /// What a call changes, as the call writes it: the value it gives (`balances`, `item.parts`), the object it is
    /// called on and the part (`cart.items`), or, for a method called without its object, the caller's own field
    /// (`items`). A value that is no name (a literal) is left out: nothing else holds it.
    fn changed_by(&mut self, e: At<'w>, decl: At<'w>) -> Vec<String> {
        let changes = self.changes_of(decl);
        let f = self.file(e);
        let mut out: Vec<String> = vec![];
        for ch in changes {
            let text = match self.value_for(e, decl, &ch.root) {
                Some(Some(v)) if self.held(e.file, v) => join_path(&one_line(f.text_of(v), 30), &ch.path),
                Some(None) => ch.path.trim_start_matches('.').to_string(),
                _ => continue,
            };
            if !text.is_empty() && !out.contains(&text) {
                out.push(text);
            }
        }
        out
    }

    /// A statement written in a function that changes what reaches the function from outside: a step of its own.
    fn change_step(&mut self, st: At<'w>, ctx: &Ctx) -> Option<Item> {
        let func = self.function_of(st)?;
        self.changed_at(func, st.node)?;
        Some(Item::Own {
            id: self.id(),
            at: ctx.this.clone(),
            kind: SelfKind::Changes,
            label: one_line(self.text(st), 56),
            detail: Some(one_line(self.text(st), 0)),
            place: self.place(st),
        })
    }

    /// The function a statement is written in.
    fn function_of(&self, at: At<'w>) -> Option<At<'w>> {
        let l = self.lang(at);
        let mut n = at.node.parent();
        while let Some(p) = n {
            if l.is_function(p.kind()) {
                return Some(self.at(at, p));
            }
            n = p.parent();
        }
        None
    }

    /// Written directly in a class: a method.
    fn is_method(&self, decl: At<'w>) -> bool {
        let l = self.lang(decl);
        let mut n = decl.node.parent();
        while let Some(p) = n {
            if l.reach.classes.contains(&p.kind()) {
                return true;
            }
            if l.is_function(p.kind()) {
                return false;
            }
            n = p.parent();
        }
        false
    }

    /// The function returns nothing: no `return` with a value of its own, and no expression body.
    fn returns_nothing(&self, decl: At<'w>) -> bool {
        let l = self.lang(decl);
        let Some(body) = (l.function_body)(decl.node) else { return true };
        if !l.blocks.contains(&body.kind()) {
            return false;
        }
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                if l.is_function(ch.kind()) {
                    continue;
                }
                if ch.kind() == l.follow.returns && ch.named_child(0).is_some_and(|v| v.kind() != "comment") {
                    return false;
                }
                stack.push(ch);
            }
        }
        true
    }

    pub fn project_call(&mut self, e: At<'w>, decl: At<'w>, hops: &[String], ctx: &Ctx) -> Vec<Item> {
        // a function declared inside the current one is part of it, not a separate participant
        if self.nested_in(decl, &ctx.stack) {
            self.reached(decl, e, ctx, None);
            return self.callback(decl, ctx);
        }
        let adapters = self.w.adapters;
        let name = self.decl_name(decl);
        let same_lane = adapters.list.iter().any(|a| a.same_lane(self, decl, ctx));
        // a function that runs in one place whoever calls it (a server action) is a call across
        let fixed = if same_lane { None } else { self.fixed_place(decl) };
        let group = fixed.map(|(g, _)| g).unwrap_or(ctx.group);
        let (unused, into) = self.result_use(e);
        let added = !same_lane && !self.has_part(&self.pid(decl));
        let to = if same_lane {
            ctx.this.clone()
        } else {
            let sub = self.file(decl).rel.clone();
            self.part(Participant { id: self.pid(decl), name: name.clone(), kind: PartKind::Function, group, sub: Some(sub), at: Some(self.place(decl)) })
        };
        let what = match fixed {
            Some((_, why)) if group != ctx.group => format!("call to {why}"),
            _ => "call".to_string(),
        };
        let mut call = Call {
            id: self.id(),
            from: ctx.this.clone(),
            to: to.clone(),
            label: one_line(self.text(e), 40),
            detail: Some(one_line(self.text(e), 0)),
            link: self.resolved_link(e, decl, &what, hops),
            effect: None,
            at: self.place(e),
            target: Some(self.place(decl)),
            body: None,
            note: None,
            opens: None,
            story: None,
            table: None,
            touches: None,
            unused,
            into,
            ends: vec![],
            changes: vec![],
        };
        self.reached(decl, e, ctx, Some(&call.id));
        if ctx.stack.contains(&decl.key()) {
            return vec![Item::Call(call)];
        }
        let mut stack = ctx.stack.clone();
        stack.push(decl.key());
        // a function that says nothing unless it crosses a boundary (a hook) runs quietly on its caller's lifeline; a
        // function made inside one runs where it is called, so it is quiet only where its caller is (while drawing)
        let silent = adapters.list.iter().any(|a| a.silent(self, decl));
        let inner_ctx = Ctx { this: to.clone(), group, return_to: Some(ctx.this.clone()), endpoint: false, known: Rc::default(), stack, lane: same_lane, quiet: ctx.quiet || (same_lane && silent), in_loop: ctx.in_loop, catching: None };
        let id = call.id.clone();
        // what the call gives is what the function's parameters stand for while it is walked
        let bound = self.w.res.bindings();
        if let Expr::Call { args, .. } = (self.lang(e).expr)(self.lang(e).unwrap(e.node), &self.file(e).text) {
            self.bind_args(decl, e.file, &args);
        }
        let inner = self.inside(&id, |b| {
            b.trial(|b| {
                let adapters = b.w.adapters;
                match adapters.list.iter().find_map(|a| a.walk_function(b, decl, &inner_ctx, false)) {
                    Some(items) => items,
                    None => b.callback(decl, &inner_ctx),
                }
            })
        });
        self.w.res.unbind(bound);
        match inner {
            None if silent => {
                if added {
                    self.drop_part(&to);
                }
                vec![]
            }
            Some(inner) => {
                self.walked.push((name, decl));
                call.touches = Some(touched(&inner));
                self.ends_with(&mut call, &inner);
                call.changes = self.changed_by(e, decl);
                call.body = Some(inner);
                vec![Item::Call(call)]
            }
            None => {
                // a pure helper: one arrow there and one back. A helper that returns nothing and changes nothing it is
                // given (logging), and plain formatting while a screen is drawn, say nothing about the story.
                let changes = self.changed_by(e, decl);
                let nothing = self.returns_nothing(decl);
                if ctx.quiet || (nothing && changes.is_empty()) {
                    if added {
                        self.drop_part(&to);
                    }
                    return vec![];
                }
                call.changes = changes;
                if nothing {
                    return vec![Item::Call(call)];
                }
                let (label, detail) = self.returned(e, decl);
                let ret = Item::Return { id: self.id(), from: to, to: ctx.this.clone(), label, detail: Some(detail), at: self.place(decl), throws: false, early: false, passes: None };
                vec![Item::Call(call), ret]
            }
        }
    }

    /// A request the code sends to one of the project's own endpoints (`fetch("/api/items")`, a test client's
    /// `client.post("/items")`): an arrow to the endpoint, with what the endpoint does under it.
    pub fn request(&mut self, e: At<'w>, route: &Route<'w>, label: String, how: String, ctx: &Ctx) -> Call {
        let name = endpoint_name(route);
        let decl = route.decl;
        let (unused, into) = self.result_use(e);
        let to = self.part(Participant { id: name.clone(), name: name.clone(), kind: PartKind::Endpoint, group: route.group, sub: Some(route.file.clone()), at: Some(self.place(decl)) });
        let mut call = Call {
            id: self.id(),
            from: ctx.this.clone(),
            to: to.clone(),
            label,
            detail: Some(one_line(self.text(e), 0)),
            link: Link { kind: LinkKind::Exact, how },
            effect: None,
            at: self.place(e),
            target: Some(self.place(decl)),
            body: None,
            note: None,
            opens: None,
            story: None,
            table: None,
            touches: None,
            unused,
            into,
            ends: vec![],
            changes: vec![],
        };
        if !ctx.stack.contains(&decl.key()) {
            let mut stack = ctx.stack.clone();
            stack.push(decl.key());
            let inner = Ctx { return_to: Some(ctx.this.clone()), endpoint: true, ..Ctx::new(to, route.group, stack) };
            let adapters = self.w.adapters;
            let id = call.id.clone();
            let body = self.inside(&id, |b| match adapters.list.iter().find_map(|a| a.walk_function(b, decl, &inner, false)) {
                Some(items) => items,
                None => b.callback(decl, &inner),
            });
            call.touches = Some(touched(&body));
            self.ends_with(&mut call, &body);
            // the sender gets the answer back as a value: what ends the endpoint does not end the function that sent
            // the request (the call still names it)
            self.ends.remove(&call.id);
            call.body = Some(body);
            self.walked.push((name, decl));
        }
        call
    }

    /// What a plain helper hands back, as a label: its return type as the language server shows it (what an awaited
    /// call resolves to), else the name the caller gives the result.
    fn returned(&self, e: At<'w>, decl: At<'w>) -> (String, String) {
        let l = self.lang(decl);
        let ty = (l.function_name)(decl.node).and_then(|n| self.w.res.hover(decl.file, n)).and_then(|h| (l.signature)(&h).1);
        if let Some(mut t) = ty {
            let awaited = e.node.parent().is_some_and(|p| p.kind() == "await_expression" || p.kind() == "await");
            if let (true, Some((open, close))) = (awaited, l.awaited) {
                if let Some(inner) = t.strip_prefix(open).and_then(|x| x.strip_suffix(close)) {
                    t = inner.to_string();
                }
            }
            let server = self.w.res.server_for(decl.file).map(|s| s.client.name.clone()).unwrap_or_default();
            return (t, format!("return type, as {server} shows it"));
        }
        match self.receiver(e) {
            Some(r) => (r, "the returned value, named as the caller names it".into()),
            None => ("result".into(), "a returned value; its type is not known".into()),
        }
    }

    /// Walks a callee; keeps the result only if it crosses a boundary (network, database, request input), can end
    /// the work early (it throws, and does not catch it: that is a rule the story keeps) or changes what a screen
    /// shows.
    pub fn trial(&mut self, walk: impl FnOnce(&mut Self) -> Vec<Item>) -> Option<Vec<Item>> {
        let before: HashSet<String> = self.parts.iter().map(|p| p.id.clone()).collect();
        let walked = self.walked.len();
        let items = walk(self);
        if self.crosses(&items) || !self.escaping(&items).is_empty() || changes_state(&items) {
            return Some(items);
        }
        self.parts.retain(|p| before.contains(&p.id));
        self.walked.truncate(walked);
        None
    }

    pub fn crosses(&self, items: &[Item]) -> bool {
        items.iter().any(|i| match i {
            Item::Call(c) => {
                // a call that stays on its caller's lifeline crosses nothing by itself; one that reaches another story
                // leads to what that story reaches
                let kind = self.parts.iter().find(|p| p.id == c.to).map(|p| p.kind);
                // a function that runs in another place than its caller (a server action) is a crossing itself
                let place = |id: &str| self.parts.iter().find(|p| p.id == id).map(|p| p.group).filter(|g| matches!(g, Group::Browser | Group::Device | Group::Server));
                let moves = place(&c.from).zip(place(&c.to)).is_some_and(|(a, b)| a != b);
                (c.from != c.to && !matches!(kind, Some(PartKind::Function | PartKind::Component))) || moves || c.opens.is_some() || c.body.as_ref().is_some_and(|b| self.crosses(b))
            }
            Item::Own { kind, .. } => *kind == SelfKind::Reads,
            Item::Frame { items, els, .. } => self.crosses(items) || els.as_ref().is_some_and(|e| self.crosses(e)),
            Item::Return { .. } => false,
        })
    }

    // ---------- entries ----------

    /// What a screen or page is called: its route's name, else its code's.
    pub fn route_name(&self, route: &Route<'w>) -> String {
        let name = |r: &Route<'w>| r.name.clone().unwrap_or_else(|| self.decl_name(r.decl));
        let own = name(route);
        // components often share a name (`Page`): two screens of one name are told apart by their paths
        if route.kind == RouteKind::Page && self.w.routes.iter().filter(|r| r.kind == RouteKind::Page && name(r) == own).count() > 1 { route.path.clone() } else { own }
    }

    /// The screen whose opening starts at `decl`.
    pub fn page_at(&self, decl: At<'w>) -> Option<Route<'w>> {
        self.w.routes.iter().find(|r| r.kind == RouteKind::Page && r.decl.key() == decl.key()).cloned()
    }

    /// A screen's opening is a story of its own: always for a screen opened by its address, else where it crosses a
    /// boundary, as `discover` keeps it. A place that reaches that screen (rendering it, opening it) is then a
    /// reference to the story, which is told there, once. Worked out once per screen, on its own; a screen reached
    /// again while it is worked out counts as not told.
    pub fn told(&self, route: &Route<'w>) -> bool {
        if route.screen {
            return true;
        }
        let key = route.decl.key();
        let known = self.w.told.borrow().get(&key).copied();
        if let Some(v) = known {
            return v;
        }
        self.w.told.borrow_mut().insert(key, false);
        let mut probe = Builder::new(self.w);
        probe.screen_opens(route);
        let v = probe.meaningful;
        self.w.told.borrow_mut().insert(key, v);
        v
    }

    pub fn screen(&mut self, route: &Route<'w>) -> String {
        let name = self.route_name(route);
        self.part(Participant {
            id: self.pid(route.decl),
            name,
            kind: PartKind::Screen,
            group: route.group,
            sub: Some(format!("{} · {}", route.path, route.file)),
            at: Some(self.place(route.decl)),
        })
    }

    pub fn screen_opens(&mut self, route: &Route<'w>) -> Story {
        let decl = route.decl;
        let this = self.screen(route);
        let ctx = Ctx::new(this, route.group, vec![decl.key()]);
        let adapters = self.w.adapters;
        let items = match adapters.list.iter().find_map(|a| a.walk_function(self, decl, &ctx, true)) {
            Some(items) => items,
            None => self.callback(decl, &ctx),
        };
        let name = self.route_name(route);
        self.walked.insert(0, (name.clone(), decl));
        let at = self.place(decl);
        self.starts(&ctx.this, at);
        let trigger = Trigger { kind: "screen-open".into(), route: route.path.clone(), at: self.place(decl), place: Some(route.group), ..Default::default() };
        self.finish(format!("open:{}", route.path), format!("{name} opens"), trigger, items)
    }

    pub fn user_action(&mut self, route: &Route<'w>, t: &TriggerSite<'w>) -> Story {
        let decl = route.decl;
        let this = self.screen(route);
        // an inline handler that only hands on to a function of the screen's own (`(_) => submit()`) runs that
        // function as its story, as a handler named there does
        let runs = self.hands_on_directly(t.handler).filter(|d| self.nested_in(*d, &[decl.key()])).unwrap_or(t.handler);
        let ctx = Ctx::new(this.clone(), route.group, vec![decl.key(), runs.key()]);
        let mut items = vec![Item::Own {
            id: self.id(),
            at: this,
            kind: SelfKind::Trigger,
            label: format!("{} {}{}", t.event, t.element, if t.text.is_empty() { String::new() } else { format!(" · “{}”", t.text) }),
            detail: Some(one_line(self.text(t.holder), 0)),
            place: self.place(t.attr),
        }];
        let via = if runs == t.handler { t.via } else { None };
        // a handler that runs in another place (a server action) is a call to it; any other runs where the screen does
        let walked = self.with_via(runs, via, |b| match b.fixed_place(runs).filter(|(g, _)| *g != route.group) {
            Some(_) => b.project_call(t.attr, runs, &[], &Ctx { stack: vec![decl.key()], ..ctx.clone() }),
            None => b.callback(runs, &ctx),
        });
        items.extend(walked);
        let adapters = self.w.adapters;
        for a in &adapters.list {
            a.check(self, t.handler);
        }
        // a handler written inline has no name of its own; one referred to by name is known by it
        let hname = if t.named_handler { self.with_via(t.handler, via, |b| b.decl_name(t.handler)) } else { String::new() };
        if !hname.is_empty() {
            self.walked.push((hname.clone(), t.handler));
            self.starts_in(t.handler, route.group);
        }
        // one that only hands on to a function of the project (`() => save(item)`) is known by that function's name
        let hname = if hname.is_empty() { self.hands_on(t.handler).map(|d| self.decl_name(d)).filter(|n| n != "anonymous").unwrap_or_default() } else { hname };
        let name = self.route_name(route);
        let title = if t.text.is_empty() { format!("{} on {name}", t.element) } else { format!("“{}” on {name}", t.text) };
        let trigger = Trigger {
            kind: "user-action".into(),
            route: route.path.clone(),
            at: self.place(t.attr),
            event: Some(t.event_name.clone()),
            control: t.control.clone(),
            text: if t.text.is_empty() { None } else { Some(t.text.clone()) },
            handler: if hname.is_empty() { None } else { Some(hname.clone()) },
            place: Some(route.group),
            ..Default::default()
        };
        // a handler with no name is known by its event and the words on its control
        let key = if !hname.is_empty() { hname } else if t.text.is_empty() { t.event.clone() } else { format!("{} “{}”", t.event, t.text) };
        self.finish(format!("action:{}#{key}", route.path), title, trigger, items)
    }

    /// A request to an endpoint, from outside the project: the function that answers it, its dependencies first.
    pub fn endpoint_story(&mut self, route: &Route<'w>) -> Story {
        let decl = route.decl;
        let name = endpoint_name(route);
        let this = self.part(Participant { id: name.clone(), name: name.clone(), kind: PartKind::Endpoint, group: route.group, sub: Some(route.file.clone()), at: Some(self.place(decl)) });
        let ctx = Ctx { endpoint: true, ..Ctx::new(this, route.group, vec![decl.key()]) };
        let adapters = self.w.adapters;
        let items = match adapters.list.iter().find_map(|a| a.walk_function(self, decl, &ctx, true)) {
            Some(items) => items,
            None => self.callback(decl, &ctx),
        };
        let handler = self.decl_name(decl);
        self.walked.insert(0, (handler.clone(), decl));
        let at = self.place(decl);
        self.starts(&ctx.this, at.clone());
        let trigger = Trigger { kind: "endpoint".into(), route: route.file.clone(), at, handler: Some(handler), place: Some(route.group), ..Default::default() };
        self.finish(format!("endpoint:{name}"), name, trigger, items)
    }

    /// A story that starts at any named function: what the core shows with no framework adapter. `place`: where it
    /// runs, when that is known.
    pub fn function_story(&mut self, decl: At<'w>, place: Option<Group>) -> Story {
        let name = self.decl_name(decl);
        let file = self.file(decl).rel.clone();
        let group = place.unwrap_or(Group::Code);
        let this = self.part(Participant { id: self.pid(decl), name: name.clone(), kind: PartKind::Function, group, sub: Some(file.clone()), at: Some(self.place(decl)) });
        let ctx = Ctx::new(this, group, vec![decl.key()]);
        let items = self.callback(decl, &ctx);
        self.walked.insert(0, (name.clone(), decl));
        let at = self.place(decl);
        self.starts(&ctx.this, at);
        let trigger = Trigger { kind: "function".into(), route: file, at: self.place(decl), place, ..Default::default() };
        self.finish(format!("function:{}", self.pid(decl)), format!("{name} runs"), trigger, items)
    }

    /// A function a framework or a library starts by itself (a job on a schedule, what an app runs as it starts):
    /// listed under the file that hands it over, named by when it runs.
    pub fn start_story(&mut self, s: &Start<'w>, place: Option<Group>) -> Story {
        let mut story = self.function_story(s.func, place);
        let name = self.decl_name(s.func);
        story.id = format!("{}:{}", s.kind, self.pid(s.func));
        story.title = format!("{name} runs {}", s.when);
        let t = &mut story.trigger;
        t.kind = s.kind.into();
        t.route = self.file(s.by).rel.clone();
        t.handler = Some(name);
        t.when = Some(s.when.clone());
        t.around = s.around.clone();
        t.by = Some(one_line(self.text(s.by), 120));
        t.by_at = Some(self.place(s.by));
        story
    }

    /// The story of an entry point no adapter names: the function, with how it is reached. `place`: where it runs,
    /// when that is known.
    pub fn entry_story(&mut self, e: &entries::Entry<'w>, place: Option<Group>) -> Story {
        let mut s = self.function_story(e.decl, place);
        if !e.named {
            // an unnamed function is known by the call it is written in
            let id = self.pid(e.decl);
            if let Some(p) = self.parts.iter_mut().find(|p| p.id == id) {
                p.name = e.name.clone();
            }
            if let Some(p) = s.participants.iter_mut().find(|p| p.id == id) {
                p.name = e.name.clone();
            }
            s.id = format!("function:{}#{}", self.file(e.decl).rel, e.name);
            s.title = format!("{} runs", e.name);
        }
        // an unnamed function is known by the words written in the call that registers it (`it("adds two", …)`)
        let words = match (&e.how, e.named) {
            (entries::How::Passed(call), false) => match (self.lang(*call).expr)(call.node, &self.file(*call).text) {
                Expr::Call { args, .. } => args.into_iter().take_while(|a| a.end_byte() <= e.decl.node.start_byte()).find_map(|a| self.url_text(self.at(*call, a))),
                _ => None,
            },
            _ => None,
        };
        let by = match &e.how {
            // the call it is passed to, short: `scheduler.add_job(…)`; an unnamed function is written in it
            entries::How::Passed(call) if e.named => match (self.lang(*call).expr)(call.node, &self.file(*call).text) {
                Expr::Call { callee, .. } => Some(format!("{}(…)", one_line(self.text(self.at(*call, callee)), 60))),
                _ => None,
            },
            entries::How::Passed(_) => Some(e.name.clone()),
            // a decorator that is a call, short: its first argument only (`@app.get("/items", …)`)
            entries::How::Decorated(d) => {
                let l = self.lang(*d);
                let call = d.node.named_child(0).filter(|c| l.call_field(c.kind()).is_some());
                match call.map(|c| (l.expr)(c, &self.file(*d).text)) {
                    Some(Expr::Call { callee, args }) => {
                        let first = args.first().map(|a| one_line(self.text(self.at(*d, *a)), 60)).unwrap_or_default();
                        let more = if args.len() > 1 { ", …" } else { "" };
                        Some(format!("@{}({first}{more})", one_line(self.text(self.at(*d, callee)), 60)))
                    }
                    _ => Some(one_line(self.text(*d), 120)),
                }
            }
            how => how.at().map(|at| one_line(self.text(at), 120)),
        };
        let t = &mut s.trigger;
        t.handler = if e.named { Some(e.name.clone()) } else { None };
        t.text = words.filter(|w| !w.trim().is_empty());
        t.how = Some(e.how.kind().to_string());
        t.by = by;
        t.by_at = e.how.at().map(|at| self.place(at));
        t.extends = e.extends.clone();
        s
    }

    /// The code written at the top of a file: it runs when the file is run or first loaded.
    pub fn file_story(&mut self, file: usize, list: &[Node<'w>]) -> Story {
        let f = &self.w.p.files[file];
        let name = f.rel.rsplit('/').next().unwrap_or(&f.rel).to_string();
        let root = At { file, node: f.tree.root_node() };
        let this = self.part(Participant { id: format!("{}#", f.rel), name: name.clone(), kind: PartKind::Function, group: Group::Code, sub: Some(f.rel.clone()), at: Some(self.place(root)) });
        // not the file itself on the stack: every function in it would count as nested in what is being walked
        let ctx = Ctx::new(this, Group::Code, vec![]);
        let items = self.stmts(file, list, &ctx);
        let at = match (list.first(), list.last()) {
            (Some(a), Some(b)) => self.span(At { file, node: *a }, At { file, node: *b }),
            _ => self.place(root),
        };
        self.starts(&ctx.this, at.clone());
        let trigger = Trigger { kind: "file".into(), route: f.rel.clone(), at, ..Default::default() };
        self.finish(format!("file:{}", f.rel), format!("{name} runs"), trigger, items)
    }

    /// Lines of a function that hold only comments or documentation strings (1-based).
    fn doc_lines(&self, decl: At<'w>) -> HashSet<usize> {
        let f = self.file(decl);
        let l = f.lang;
        let mut out = HashSet::new();
        let mut stack = vec![decl.node];
        while let Some(n) = stack.pop() {
            let doc = n.kind() == "comment" || matches!((l.stmt)(n, &f.text), Stmt::Expr(e) if l.strings.nodes.contains(&e.kind()));
            if doc {
                out.extend(n.start_position().row + 1..=n.end_position().row + 1);
                continue;
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
        out
    }

    // ---------- output ----------

    pub fn finish(&mut self, id: String, title: String, trigger: Trigger, items: Vec<Item>) -> Story {
        let mut refs: Vec<Ref> = vec![];
        let mut files = BTreeSet::new();
        fn collect(list: &[Item], refs: &mut Vec<Ref>, files: &mut BTreeSet<String>) {
            for i in list {
                let r = i.place().clone();
                files.insert(r.file.clone());
                match i {
                    Item::Frame { items, els, .. } => {
                        refs.push(Ref { to: r.from, ..r });
                        collect(items, refs, files);
                        collect(els.as_deref().unwrap_or(&[]), refs, files);
                    }
                    Item::Call(c) => {
                        refs.push(r);
                        if let Some(t) = &c.target {
                            files.insert(t.file.clone());
                        }
                        collect(c.body.as_deref().unwrap_or(&[]), refs, files);
                    }
                    _ => refs.push(r),
                }
            }
        }
        collect(&items, &mut refs, &mut files);
        for p in &self.parts {
            if let Some(r) = &p.at {
                files.insert(r.file.clone());
            }
        }

        // every line inside a walked function should sit behind some step (signature and lone brackets aside)
        let coverage = self
            .walked
            .iter()
            .filter(|(_, decl)| self.is_function(*decl))
            .map(|(name, decl)| {
                let r = self.place(*decl);
                let f = self.file(*decl);
                let doc = self.doc_lines(*decl);
                let mut missing = vec![];
                let mut total = 0;
                for l in r.from + 1..=r.to {
                    let t = f.line(l - 1).trim();
                    if t.is_empty() || t.chars().all(|c| "])}>;,".contains(c)) || t.starts_with(f.lang.line_comment) || doc.contains(&l) {
                        continue;
                    }
                    total += 1;
                    if !refs.iter().any(|x| x.file == r.file && x.from <= l && l <= x.to) {
                        missing.push(l);
                    }
                }
                Coverage { func: name.clone(), file: r.file, total, covered: total - missing.len(), missing }
            })
            .collect();

        let order = |g: Group| match g {
            Group::Device | Group::Browser | Group::Code => 0,
            Group::Server => 1,
            Group::Supabase => 2,
            Group::Outside => 3,
        };
        let mut participants = self.parts.clone();
        participants.sort_by_key(|p| order(p.group));
        self.files = files;
        self.meaningful = self.crosses(&items);
        let mut items = items;
        let mut temp = vec![];
        reach::step_ids(&items, &mut temp);
        crate::model::name_steps(&mut items);
        let mut named = vec![];
        reach::step_ids(&items, &mut named);
        let map: HashMap<String, String> = temp.into_iter().zip(named).collect();
        self.uses = self.uses_of(&items, &map);
        Story { id, title, trigger, also: vec![], participants, items, signals: self.signals.clone(), gaps: self.gaps.clone(), coverage }
    }
}

/// "reads items", "writes books", "calls count_items": what a folded call goes on to touch.
pub fn touched(items: &[Item]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    fn walk(list: &[Item], out: &mut Vec<String>) {
        for i in list {
            match i {
                Item::Call(c) => {
                    if let Some(t) = &c.table {
                        let e = match c.effect {
                            Some(Effect::Reads) => "reads",
                            Some(Effect::Writes) => "writes",
                            Some(Effect::Deletes) => "deletes",
                            None => "calls",
                        };
                        let s = format!("{e} {t}");
                        if !out.contains(&s) {
                            out.push(s);
                        }
                    }
                    walk(c.body.as_deref().unwrap_or(&[]), out);
                }
                Item::Frame { items, els, .. } => {
                    walk(items, out);
                    walk(els.as_deref().unwrap_or(&[]), out);
                }
                _ => {}
            }
        }
    }
    walk(items, &mut out);
    out
}

/// Some step, however deep, is a call whose result nobody uses and that leaves its caller's lifeline: it does something.
fn has_effect(items: &[Item]) -> bool {
    items.iter().any(|i| match i {
        Item::Call(c) => (c.unused && c.from != c.to) || c.body.as_ref().is_some_and(|b| has_effect(b)),
        Item::Frame { items, els, .. } => has_effect(items) || els.as_ref().is_some_and(|e| has_effect(e)),
        _ => false,
    })
}

/// The same steps, whatever ids they were given.
fn same_steps(a: &[Item], b: &[Item]) -> bool {
    fn plain(items: &[Item]) -> serde_json::Value {
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(m) => {
                    m.remove("id");
                    m.values_mut().for_each(strip);
                }
                serde_json::Value::Array(list) => list.iter_mut().for_each(strip),
                _ => {}
            }
        }
        let mut v = serde_json::to_value(items).unwrap_or_default();
        strip(&mut v);
        v
    }
    a.len() == b.len() && plain(a) == plain(b)
}

/// An endpoint as its participant is named: "POST /items".
pub fn endpoint_name(r: &Route) -> String {
    format!("{} {}", r.method.as_deref().unwrap_or("ANY"), r.path)
}

/// Where the first call to the participant `to` is written, opened calls included.
fn first_call(items: &[Item], to: &str) -> Option<Ref> {
    for i in items {
        if let Item::Call(c) = i
            && c.to == to
        {
            return Some(c.at.clone());
        }
        if let Some(r) = i.lists().into_iter().find_map(|(_, l)| first_call(l, to)) {
            return Some(r);
        }
    }
    None
}

/// Where the code at `at` runs: where the function it is written in runs, as the stories told so far reach it
/// (`places`: each function's id, with the places stories reached it in). A function value does not leave its
/// program, so a function handed over there runs there too. None at the top of a file, or where stories reach that
/// function in no place or in more than one.
fn place_at(w: &World, places: &HashMap<String, Vec<Group>>, at: At) -> Option<Group> {
    let l = w.p.files[at.file].lang;
    let mut n = at.node.parent();
    while let Some(x) = n.filter(|x| !l.is_function(x.kind())) {
        n = x.parent();
    }
    match places.get(&w.id_of(At { file: at.file, node: n? }))?.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// Every story in the project worth showing that crosses a boundary: each screen opening and user action an adapter
/// finds, each endpoint, what a framework or a library starts by itself, then each entry point no adapter names.
pub fn discover(w: &World, missing: Vec<NotInstalled>) -> ProjectModel {
    let t0 = std::time::Instant::now();
    let mut stories = vec![];
    let mut files = BTreeSet::new();
    let only = std::env::var("ONLY").ok();
    let pages: Vec<&Route> = w.routes.iter().filter(|r| r.kind == RouteKind::Page && only.as_ref().is_none_or(|o| *o == r.path)).collect();
    // two stories can share a key (two inline handlers of one event with no words on their controls): later ones
    // get a count, in source order
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut uses = vec![];
    // the places each of the project's functions is reached in, by every story told so far, kept or not
    let places: RefCell<HashMap<String, Vec<Group>>> = RefCell::default();
    let mut keep = |b: &mut Builder, mut s: Story| {
        for u in &b.uses {
            let own = matches!(u.thing.kind, PartKind::Function | PartKind::Screen | PartKind::Component | PartKind::Endpoint);
            if own && matches!(u.thing.group, Group::Device | Group::Browser | Group::Server) {
                let mut all = places.borrow_mut();
                let list = all.entry(u.thing.id.clone()).or_default();
                if !list.contains(&u.thing.group) {
                    list.push(u.thing.group);
                }
            }
        }
        // a test, a request or what starts by itself is worth showing whatever it reaches: leaving one out would hide it
        let started = matches!(s.trigger.kind.as_str(), "endpoint" | "schedule" | "lifecycle");
        if b.meaningful || started || (s.trigger.kind == "test" && !s.items.is_empty()) {
            // two ways to start the same work on one screen (a button, and the keyboard's submit in a field) are one
            // story: after the trigger, the steps are the same. It is named by the one with words on its control,
            // else the first written.
            let same = |a: &Story| a.trigger.kind == "user-action" && a.trigger.route == s.trigger.route && same_steps(&a.items[1..], &s.items[1..]);
            if s.trigger.kind == "user-action" && !s.items.is_empty() && let Some(first) = stories.iter_mut().find(|a: &&mut Story| !a.items.is_empty() && same(a)) {
                if first.trigger.text.is_none() && s.trigger.text.is_some() {
                    std::mem::swap(&mut first.trigger, &mut s.trigger);
                    std::mem::swap(&mut first.title, &mut s.title);
                    std::mem::swap(&mut first.items[0], &mut s.items[0]);
                }
                first.also.push(s.trigger);
                first.also.sort_by_key(|t| (t.at.file.clone(), t.at.from));
                return;
            }
            let n = ids.entry(s.id.clone()).or_default();
            *n += 1;
            if *n > 1 {
                s.id = format!("{}~{n}", s.id);
            }
            files.extend(b.files.iter().cloned());
            for mut u in std::mem::take(&mut b.uses) {
                u.place.story = s.id.clone();
                uses.push(u);
            }
            stories.push(s);
        }
    };
    // functions an adapter starts a story at: left to it, it names them better
    let mut covered: HashSet<NodeKey> = w.routes.iter().map(|r| r.decl.key()).collect();
    let test = |file: usize| w.adapters.list.iter().any(|a| a.test(&w.p.files[file]));
    // places with user actions that are no screen (a component with a button): each action is told there, once
    let controls: Vec<Route> = w.adapters.list.iter().flat_map(|a| a.controls(w)).filter(|r| !test(r.decl.file) && only.as_ref().is_none_or(|o| *o == r.path)).collect();
    for (r, screen) in pages.into_iter().map(|r| (r, true)).chain(controls.iter().map(|r| (r, false))) {
        if screen {
            let mut b = Builder::new(w);
            let s = b.screen_opens(r);
            // a screen opened by its address is a place a person sees: shown whatever its opening reaches
            b.meaningful |= r.screen;
            keep(&mut b, s);
        }
        let mut finder = Builder::new(w);
        let adapters = w.adapters;
        let triggers: Vec<TriggerSite> = adapters.list.iter().flat_map(|a| a.triggers(&mut finder, r.decl)).collect();
        for t in &triggers {
            covered.insert(t.handler.key());
            // what the function that made the handler was given is what the handler runs: told in this story
            if let Some(via) = t.via {
                covered.extend(finder.given_functions(via).into_iter().map(|f| f.key()));
            }
            let mut a = Builder::new(w);
            let s = a.user_action(r, t);
            keep(&mut a, s);
        }
    }
    // every endpoint, whether a story in the project calls it or not: what a request runs is told there, and a story
    // that calls it shows it in place too (it waits for the answer)
    if only.is_none() {
        for r in w.routes.iter().filter(|r| r.kind == RouteKind::Endpoint) {
            let mut b = Builder::new(w);
            let s = b.endpoint_story(r);
            keep(&mut b, s);
        }
    }
    // what a framework or a library starts by itself: those whose place the adapter gives first, so that a job
    // handed over in their code runs where they do
    let mut starts: Vec<Start> = if only.is_none() { w.adapters.list.iter().flat_map(|a| a.starts(w)).collect() } else { vec![] };
    starts.sort_by_key(|s| (s.place.is_none(), s.by.file, s.by.node.start_byte()));
    for s in &starts {
        covered.insert(s.func.key());
        let place = s.place.or_else(|| place_at(w, &places.borrow(), s.by));
        let mut b = Builder::new(w);
        let story = b.start_story(s, place);
        if s.optional && !b.meaningful {
            continue;
        }
        keep(&mut b, story);
    }
    // entry points no adapter names: what the project's own code never calls, and top-level code that does work.
    // A story that starts in a file a test runner reads is a test. One handed over (passed to a call, stored) inside
    // a function runs where that function does.
    covered.extend(w.adapters.list.iter().flat_map(|a| a.runs(w)).map(|f| f.key()));
    if only.is_none() {
        let found = entries::find(w, &covered);
        for e in &found.entries {
            let handed = match &e.how {
                entries::How::Passed(at) | entries::How::Value(at) => Some(*at),
                _ => None,
            };
            let place = handed.filter(|_| !test(e.decl.file)).and_then(|at| place_at(w, &places.borrow(), at));
            let mut b = Builder::new(w);
            let mut s = b.entry_story(e, place);
            if test(e.decl.file) {
                s.trigger.kind = "test".into();
            }
            keep(&mut b, s);
        }
        for (file, list) in &found.files {
            let mut b = Builder::new(w);
            let mut s = b.file_story(*file, list);
            // code that only makes values (`const schema = make(…)`, `app = App()` of a plain function) is set-up: it is a
            // story only where a statement does something (a call written on its own, a condition, a loop, an awaited
            // value) or what it calls does (a call whose result nobody uses, outside the function that makes it)
            let f = &w.p.files[*file];
            // an awaited value outside any function written there
            fn awaits(f: &SourceFile, n: Node) -> bool {
                if f.lang.is_function(n.kind()) {
                    return false;
                }
                n.kind().starts_with("await") || n.named_children(&mut n.walk()).any(|c| awaits(f, c))
            }
            fn acts(f: &SourceFile, st: Node) -> bool {
                match (f.lang.stmt)(st, &f.text) {
                    Stmt::Decl { values, .. } => values.into_iter().any(|v| awaits(f, v)),
                    Stmt::Block(inner) => inner.into_iter().any(|x| acts(f, x)),
                    // a string on its own is a directive (`"use server"`) or a docstring
                    Stmt::Expr(e) => !f.lang.strings.nodes.contains(&f.lang.unwrap(e).kind()),
                    _ => true,
                }
            }
            let acts = list.iter().any(|st| acts(f, *st));
            if !acts && !has_effect(&s.items) {
                continue;
            }
            if test(*file) {
                s.trigger.kind = "test".into();
            }
            keep(&mut b, s);
        }
    }
    // what an app runs as it starts first, schedules before the endpoints, tests after everything else, each kind in
    // the order found
    stories.sort_by_key(|s| match s.trigger.kind.as_str() {
        "lifecycle" => 0,
        "screen-open" | "user-action" => 1,
        "schedule" => 2,
        "endpoint" => 3,
        "test" => 5,
        _ => 4,
    });
    let order: Vec<String> = stories.iter().map(|s| s.id.clone()).collect();
    reach::through_references(&stories, &mut uses);
    let index = reach::index(uses, &order);
    // a package that is not installed is said in the stories that call into it, where they first do
    for s in &mut stories {
        for m in &missing {
            let to = format!("pkg:{}", m.package);
            if !s.participants.iter().any(|p| p.id == to) {
                continue;
            }
            let at = first_call(&s.items, &to).unwrap_or(Ref { file: String::new(), from: 1, to: 1 });
            s.gaps.push(Finding { text: format!("{} is not installed: calls into it are shown by name only", m.package), at });
        }
    }
    let mut src = BTreeMap::new();
    for f in files {
        if let Some(sf) = w.p.files.iter().find(|x| x.rel == f) {
            src.insert(f, sf.text.clone());
        }
    }
    ProjectModel { name: w.p.name.clone(), stories, index, files: src, stats: Stats { ms: t0.elapsed().as_millis(), llm_calls: 0 }, setup: vec![], missing }
}

#[cfg(test)]
mod tests {
    use super::join_path;

    #[test]
    fn parts_join_the_value_they_are_written_after() {
        assert_eq!(join_path("cart", ".items"), "cart.items");
        assert_eq!(join_path("cart", "items"), "cart.items");
        assert_eq!(join_path("seen", ""), "seen");
        assert_eq!(join_path("", "items[0]"), "items[0]");
        assert_eq!(join_path("self.cart", "[key]"), "self.cart[key]");
    }
}
