// Flutter state adapter (Riverpod, provider). Handles: the packages a Flutter app keeps its state and its shared
// objects in. Reading one (`ref.watch(p)`, `ref.read(p)`, `context.read<T>()`), listening to it and resetting it are
// how a widget reads and changes the app's state, as `useState` is in React, not calls across a boundary: they are not
// drawn as calls into a package. Resetting one (`ref.invalidate(p)`, `ref.refresh(p)`) changes the app's state, so it
// is a step of its own, as a state setter is. What a provider gives is the project's own code, so a call on it
// (`ref.read(p.notifier).load()`) is followed as any other.
use super::Adapter;
use crate::lang::{Expr, Lang};
use crate::model::{FrameKind, Item, PartKind, Participant, SelfKind};
use crate::source::{At, one_line};
use crate::story::{Builder, Chain, Ctx, World, boring};
use tree_sitter::Node;

pub struct FlutterState;

const PACKAGES: &[&str] = &["riverpod", "flutter_riverpod", "hooks_riverpod", "riverpod_annotation", "provider"];
/// Riverpod's calls that read a provider: the first read runs the provider's code
const READS: &[&str] = &["watch", "read", "listen", "listenManual"];
/// What can follow a provider's name where it is read, and still be the provider (`todos.notifier`)
const PARTS: &[&str] = &["notifier", "future", "stream", "select", "selectAsync"];

/// The first function written inside a provider's definition (`FutureProvider((ref) async { … })`): the code the
/// provider runs when it is first read.
fn first_function<'t>(l: &Lang, n: Node<'t>) -> Option<Node<'t>> {
    let mut queue = std::collections::VecDeque::from([n]);
    while let Some(x) = queue.pop_front() {
        if l.is_function(x.kind()) {
            return Some(x);
        }
        let mut c = x.walk();
        queue.extend(x.named_children(&mut c));
    }
    None
}

/// The name of the provider an expression reads: `todos`, `todos(id)`, `todos.notifier`.
fn provider_name<'t>(l: &Lang, text: &str, n: Node<'t>) -> Option<Node<'t>> {
    let mut n = l.unwrap(n);
    for _ in 0..4 {
        if l.is_name(n.kind()) {
            return Some(n);
        }
        n = match (l.expr)(n, text) {
            Expr::Call { callee, .. } => l.unwrap(callee),
            _ => {
                let (obj, prop) = l.member_fields(n.kind())?;
                let (obj, prop) = (n.child_by_field_name(obj)?, n.child_by_field_name(prop)?);
                if PARTS.contains(&&text[prop.byte_range()]) { l.unwrap(obj) } else { return Some(prop).filter(|p| l.is_name(p.kind())) }
            }
        };
    }
    None
}

/// Riverpod's calls that throw away what a provider holds, so that it is made again when next read
const RESETS: &[&str] = &["invalidate", "refresh", "invalidateSelf"];

impl Adapter for FlutterState {
    fn name(&self) -> &'static str {
        "Flutter state (Riverpod, provider)"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["dart"]
    }
    fn handles(&self, package: &str) -> bool {
        PACKAGES.contains(&package)
    }

    fn runs<'w>(&self, w: &World<'w>) -> Vec<At<'w>> {
        // what a provider runs when it is read is told where it is read: it is no entry point of its own
        let mut out = vec![];
        for (i, f) in w.p.files.iter().enumerate().filter(|(_, f)| f.lang.server == "dart" && f.imports.iter().any(|m| PACKAGES.iter().any(|p| m.module.starts_with(&format!("package:{p}/"))))) {
            let l = f.lang;
            let mut c = f.tree.root_node().walk();
            for st in f.tree.root_node().named_children(&mut c) {
                let mut stack = vec![st];
                while let Some(n) = stack.pop() {
                    if l.is_name(n.kind()) && let Some((_, Some(v))) = l.declared(n) {
                        if f.text_of(v).trim_start().split(['.', '<', '(']).next().is_some_and(|root| root.ends_with("Provider")) && let Some(func) = first_function(l, v) {
                            out.push(At { file: i, node: func });
                        }
                    }
                    let mut k = n.walk();
                    stack.extend(n.named_children(&mut k).filter(|x| !l.is_function(x.kind())));
                }
            }
        }
        out
    }

    fn before<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Vec<Item> {
        let l = b.lang(e);
        let mut out = vec![];
        for seg in chain.segs.iter().filter(|s| READS.contains(&s.name.as_str())) {
            let Some(arg) = seg.args.as_ref().and_then(|a| a.first()) else { continue };
            // the call is Riverpod's own (a method of the project's may be named `read`)
            let Expr::Call { callee, .. } = (l.expr)(seg.node, &b.file(e).text) else { continue };
            let Some(loc) = crate::resolve::callee_name(callee, l).and_then(|n| b.w.res.definition(e.file, n).into_iter().next()) else { continue };
            if b.w.p.is_own(&loc.file) || !(l.package)(&loc.file).is_some_and(|p| self.handles(&p)) {
                continue;
            }
            let Some(name) = provider_name(l, &b.file(e).text, *arg) else { continue };
            let Some((df, dn)) = b.w.res.definition(e.file, name).into_iter().next().and_then(|loc| b.w.res.at_loc(&loc)) else { continue };
            let dl = b.w.p.files[df].lang;
            let Some((_, Some(value))) = dl.declared(dn) else { continue };
            if !b.w.p.files[df].text_of(value).trim_start().split(['.', '<', '(']).next().is_some_and(|root| root.ends_with("Provider")) {
                continue;
            }
            let Some(func) = first_function(dl, value).map(|f| At { file: df, node: f }) else { continue };
            if ctx.stack.contains(&func.key()) || !b.first_time(func.key()) {
                continue;
            }
            let mut stack = ctx.stack.clone();
            stack.push(func.key());
            // the provider is a lifeline of its own: its code runs when it is read, not as the reader's
            let sub = b.file(func).rel.clone();
            let this = b.part(Participant { id: b.pid(func), name: b.decl_name(func), kind: PartKind::Function, group: ctx.group, sub: Some(sub), at: Some(b.place(func)) });
            let items = b.callback(func, &Ctx { this, return_to: None, stack, quiet: false, ..ctx.clone() });
            if items.is_empty() || boring(&items) {
                continue;
            }
            out.push(Item::Frame {
                id: b.id(),
                kind: FrameKind::Effect,
                label: one_line(b.text(b.at(e, seg.node)), 48),
                note: Some(format!("the first time {} is read, its code runs", b.text(b.at(e, name)))),
                at: b.place(e),
                items,
                els: None,
            });
        }
        out
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let l = b.lang(e);
        let last = chain.segs.last().filter(|s| s.node == l.unwrap(e.node) && RESETS.contains(&s.name.as_str()))?;
        // the method called is the package's own (a provider's object may have a `refresh` of the project's)
        let Expr::Call { callee, .. } = (l.expr)(l.unwrap(e.node), &b.file(e).text) else { return None };
        let loc = crate::resolve::callee_name(callee, l).and_then(|n| b.w.res.definition(e.file, n).into_iter().next())?;
        if b.w.p.is_own(&loc.file) || !(l.package)(&loc.file).is_some_and(|p| self.handles(&p)) {
            return None;
        }
        let what = last.args.as_ref().and_then(|a| a.first()).map(|a| one_line(b.text(b.at(e, *a)), 40)).unwrap_or_else(|| "this provider".into());
        Some(vec![Item::Own {
            id: b.id(),
            at: ctx.this.clone(),
            kind: SelfKind::State,
            label: one_line(b.text(e), 48),
            detail: Some(format!("resets {what}: what reads it makes it again")),
            place: b.place(e),
        }])
    }
}
