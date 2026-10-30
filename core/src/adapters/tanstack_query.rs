// TanStack Query adapter. Handles: a mutation made with `useMutation` (from `@tanstack/react-query`, or the older
// `react-query`) in the file that calls it, run through what it returns (`m.mutate(x)`, `m.mutateAsync(x)`, or
// `mutate` taken from it): the call runs the function given as `mutationFn` (in the older form, the function given
// first), then what runs when it ends (`onSuccess`, `onError`, `onSettled`, given to `useMutation` or to the call),
// each a frame of what runs later; and a query (`useQuery`, `useSuspenseQuery`, `useInfiniteQuery`), whose
// `queryFn` runs later, when the data is first needed and again when it is out of date, its options written in the
// call or returned by a function of the project. Not yet: a mutation made in another file (a project's own hook
// around `useMutation`). Its other calls stay plain calls into the package.
use super::{Adapter, Later, is_imported};
use crate::lang::Expr;
use crate::model::{FrameKind, Item};
use crate::resolve::Resolver;
use crate::source::{At, SourceFile, one_line};
use crate::story::{Builder, Chain, Ctx, boring};
use tree_sitter::Node;

pub struct TanStackQuery;

const PACKAGES: &[&str] = &["@tanstack/react-query", "react-query"];
const RUNS: &[&str] = &["mutate", "mutateAsync"];
const QUERIES: &[&str] = &["useQuery", "useSuspenseQuery", "useInfiniteQuery", "useSuspenseInfiniteQuery"];

/// The object a function returns: its expression body, or what its `return` statements (not those of functions
/// written inside it) give.
fn returned_object<'p>(f: &'p SourceFile, func: Node<'p>) -> Option<Node<'p>> {
    let l = f.lang;
    let body = (l.function_body)(func)?;
    if body.kind() != "statement_block" {
        return Some(l.unwrap(body)).filter(|b| b.kind() == "object");
    }
    let mut stack = vec![body];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            if l.is_function(ch.kind()) {
                continue;
            }
            if ch.kind() == "return_statement"
                && let Some(v) = ch.named_child(0).map(|v| l.unwrap(v)).filter(|v| v.kind() == "object")
            {
                return Some(v);
            }
            stack.push(ch);
        }
    }
    None
}
/// What runs when a mutation ends, and when.
const ENDS: &[(&str, &str)] = &[("onSuccess", "when the mutation succeeds"), ("onError", "when the mutation fails"), ("onSettled", "when the mutation ends, either way")];

/// The `key: value` pairs of an object literal, with methods (`key() { … }`) as their own value.
fn options<'p>(f: &'p SourceFile, obj: Node<'p>) -> Vec<(String, Node<'p>)> {
    let obj = f.lang.unwrap(obj);
    if obj.kind() != "object" {
        return vec![];
    }
    let mut c = obj.walk();
    let props: Vec<Node> = obj.named_children(&mut c).collect();
    props
        .into_iter()
        .filter_map(|p| match p.kind() {
            "pair" => Some((f.text_of(p.child_by_field_name("key")?).to_string(), p.child_by_field_name("value")?)),
            "method_definition" => Some((f.text_of(p.child_by_field_name("name")?).to_string(), p)),
            "shorthand_property_identifier" => Some((f.text_of(p).to_string(), p)),
            _ => None,
        })
        .collect()
}

/// The function a `useMutation(…)` call runs: its options' `mutationFn`, or the function it is given first.
fn mutation_fn<'p>(f: &'p SourceFile, call: Node<'p>) -> Option<Node<'p>> {
    let l = f.lang;
    let Expr::Call { callee, args } = (l.expr)(l.unwrap(call), &f.text) else { return None };
    if !PACKAGES.iter().any(|p| is_imported(f, callee, &|m| m == *p, "useMutation")) {
        return None;
    }
    let first = l.unwrap(*args.first()?);
    if first.kind() != "object" {
        return Some(first);
    }
    let mut c = first.walk();
    let options: Vec<Node> = first.named_children(&mut c).collect();
    options.into_iter().find_map(|p| match p.kind() {
        "pair" if p.child_by_field_name("key").is_some_and(|k| f.text_of(k) == "mutationFn") => p.child_by_field_name("value"),
        "method_definition" if p.child_by_field_name("name").is_some_and(|k| f.text_of(k) == "mutationFn") => Some(p),
        "shorthand_property_identifier" if f.text_of(p) == "mutationFn" => Some(p),
        _ => None,
    })
}

/// The declaration a defined name belongs to (`const m = …`), and, for a name taken from an object
/// (`const { key: name } = …`), that key.
fn declarator<'p>(f: &'p SourceFile, n: Node<'p>) -> Option<(Node<'p>, Option<String>)> {
    let p = n.parent()?;
    match (n.kind(), p.kind()) {
        (_, "variable_declarator") => Some((p, None)),
        ("shorthand_property_identifier_pattern", "object_pattern") => Some((p.parent().filter(|d| d.kind() == "variable_declarator")?, Some(f.text_of(n).to_string()))),
        (_, "pair_pattern") => {
            let key = f.text_of(p.child_by_field_name("key")?).to_string();
            Some((p.parent()?.parent().filter(|d| d.kind() == "variable_declarator")?, Some(key)))
        }
        _ => None,
    }
}

impl Adapter for TanStackQuery {
    fn name(&self) -> &'static str {
        "TanStack Query"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn applies(&self, package: &str) -> bool {
        PACKAGES.contains(&package)
    }
    fn redirect<'p>(&self, res: &Resolver<'p>, file: usize, callee: Node<'p>) -> Option<(At<'p>, &'static str)> {
        let made = mutation_of(res, file, callee)?;
        let run = mutation_fn(&res.project.files[made.file], made.node)?;
        Some((At { file: made.file, node: run }, "the mutationFn given to useMutation"))
    }

    fn later<'w>(&self, b: &Builder<'w>, call: At<'w>) -> Option<Later<'w>> {
        let f = b.file(call);
        let l = f.lang;
        let Expr::Call { callee, args } = (l.expr)(call.node, &f.text) else { return None };
        let name = QUERIES.iter().find(|n| PACKAGES.iter().any(|p| is_imported(f, callee, &|m| m == *p, n)))?;
        // the options: written here, or returned by a function of the project (`itemsQuery()`)
        let first = l.unwrap(*args.first()?);
        let (at, obj) = match (l.expr)(first, &f.text) {
            Expr::Call { callee: maker, .. } => {
                let d = b.project_decl(b.at(call, maker))?;
                (d, returned_object(b.file(d), d.node)?)
            }
            _ => (call, first),
        };
        let of = b.file(at);
        let opts = options(of, obj);
        let run = opts.iter().find(|(k, _)| k == "queryFn")?.1;
        let run = of.lang.unwrap(run);
        let func = if of.lang.is_function(run.kind()) || run.kind() == "method_definition" { b.at(at, run) } else { b.project_decl(b.at(at, run))? };
        let key = opts.iter().find(|(k, _)| k == "queryKey").map(|(_, v)| one_line(of.text_of(*v), 0));
        Some(Later { name: name.to_string(), func, deps: key, when: "when the data is first needed, and again when it is out of date".into(), given: vec![] })
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, _chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        let Expr::Call { callee, args } = (l.expr)(l.unwrap(e.node), &f.text) else { return None };
        let made = mutation_of(b.w.res, e.file, callee)?;
        // the mutation's function, as a call of it
        let mut out = match b.project_target(b.at(e, callee)) {
            Some((decl, hops)) => b.project_call(e, decl, &hops, ctx),
            None => vec![],
        };
        // then what runs when it ends: given to `useMutation`, then to this call
        let mf = b.file(made);
        let Expr::Call { args: made_args, .. } = (mf.lang.expr)(made.node, &mf.text) else { return Some(out) };
        let given: Vec<(At<'w>, String, Node<'w>)> = made_args
            .first()
            .map(|o| options(mf, *o).into_iter().map(|(k, v)| (made, k, v)).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(args.get(1).map(|o| options(f, *o).into_iter().map(|(k, v)| (e, k, v)).collect::<Vec<_>>()).unwrap_or_default())
            .collect();
        for (at, key, value) in given {
            let Some((_, when)) = ENDS.iter().find(|(k, _)| *k == key) else { continue };
            let vf = b.file(at);
            let mut v = vf.lang.unwrap(value);
            // `handle.bind(…)` runs `handle`
            if let Expr::Call { callee: c, .. } = (vf.lang.expr)(v, &vf.text)
                && let Some((obj, prop)) = vf.lang.member_fields(vf.lang.unwrap(c).kind())
                && vf.lang.unwrap(c).child_by_field_name(prop).is_some_and(|p| vf.text_of(p) == "bind")
            {
                v = vf.lang.unwrap(vf.lang.unwrap(c).child_by_field_name(obj)?);
            }
            let func = if vf.lang.is_function(v.kind()) || v.kind() == "method_definition" { Some(b.at(at, v)) } else { b.project_decl(b.at(at, v)) };
            let Some(func) = func else { continue };
            let items = b.callback(func, &Ctx { return_to: None, ..ctx.clone() });
            if boring(&items) {
                continue;
            }
            out.push(Item::Frame { id: b.id(), kind: FrameKind::Effect, label: format!("{key}(…)"), note: Some(when.to_string()), at: b.place(b.at(at, value)), items, els: None });
        }
        Some(out)
    }
}

/// The `useMutation(…)` call a callee runs through: `m.mutate` with `m = useMutation(…)`, or `mutate` taken from it,
/// written in the file of the call.
fn mutation_of<'p>(res: &Resolver<'p>, file: usize, callee: Node<'p>) -> Option<At<'p>> {
    let f = &res.project.files[file];
    if !f.imports.iter().any(|i| PACKAGES.contains(&i.module.as_str())) {
        return None;
    }
    let l = f.lang;
    let c = l.unwrap(callee);
    // `m.mutate(…)` with `m = useMutation(…)`, or `mutate(…)` taken from it
    let (name, member) = match l.member_fields(c.kind()) {
        Some((obj, prop)) => {
            let o = c.child_by_field_name(obj)?;
            if !RUNS.contains(&f.text_of(c.child_by_field_name(prop)?)) || !l.is_name(o.kind()) {
                return None;
            }
            (o, true)
        }
        None if l.is_name(c.kind()) => (c, false),
        None => return None,
    };
    let loc = res.definition(file, name).into_iter().next()?;
    let df = res.project.file_index(&loc.file)?;
    let dfile = &res.project.files[df];
    let (decl, key) = declarator(dfile, dfile.node_at(loc.line as usize, loc.col)?)?;
    let fits = if member { key.is_none() } else { key.is_some_and(|k| RUNS.contains(&k.as_str())) };
    if !fits {
        return None;
    }
    let made = dfile.lang.unwrap(decl.child_by_field_name("value")?);
    let Expr::Call { callee, .. } = (dfile.lang.expr)(made, &dfile.text) else { return None };
    PACKAGES.iter().any(|p| is_imported(dfile, callee, &|m| m == *p, "useMutation")).then_some(At { file: df, node: made })
}
