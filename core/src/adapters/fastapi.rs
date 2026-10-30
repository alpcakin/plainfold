// FastAPI adapter. Handles: the routes of a FastAPI app (`@app.get("/items")`, `@router.post(…)` on an `APIRouter`),
// their paths with the prefixes routers are given (`APIRouter(prefix=…)`, `include_router(router, prefix=…)`, a
// prefix named by a constant followed to its value), and dependencies (`Depends(f)` / `Security(f)` as a parameter's
// default, inside its `Annotated[…]` type or a type alias of one, and in a route's `dependencies=[…]`). FastAPI runs a
// dependency before the function that asks for it, so it is walked there, first, and is not an entry point of its own.
// Requests sent through its test client (`client.post("/items")` on a `TestClient(app)`, held by a variable, a typed
// parameter or the pytest fixture a test's parameter names) are arrows to the endpoint that answers them.
// `HTTPException(status_code=…, detail=…)` raised anywhere is the response the request gets: named by its status.
// What the app runs as it starts and stops is a story of its own on the server: the `lifespan` given to the app
// (`FastAPI(lifespan=f)`; its code after `yield` runs as the app stops), and the handlers of its `startup` and
// `shutdown` events (`@app.on_event("startup")`, `app.add_event_handler("shutdown", f)`).
use super::{Adapter, Start, is_imported};
use crate::lang::{Expr, Lang};
use crate::model::Item;
use crate::resolve::{Resolver, callee_name};
use crate::source::{At, Project, SourceFile, one_line};
use crate::story::routes::{Route, RouteKind, Seg, constant, match_route, url_of};
use crate::story::{Builder, Chain, Ctx, World, string_value};
use std::collections::HashMap;
use tree_sitter::Node;

pub struct FastApi;

const METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options", "trace"];

fn fastapi(m: &str) -> bool {
    m == "fastapi" || m.starts_with("fastapi.")
}

/// Where FastAPI's test client comes from (it is Starlette's).
fn testclient(m: &str) -> bool {
    m == "fastapi.testclient" || m == "starlette.testclient"
}

fn kids(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

/// A call's arguments: the positional ones in order, and the keyword ones by name.
fn arguments<'t>(f: &SourceFile, call: Node<'t>) -> (Vec<Node<'t>>, HashMap<String, Node<'t>>) {
    let (mut pos, mut named) = (vec![], HashMap::new());
    let Expr::Call { args, .. } = (f.lang.expr)(call, &f.text) else { return (pos, named) };
    for a in args {
        match (a.kind(), a.child_by_field_name("name"), a.child_by_field_name("value")) {
            ("keyword_argument", Some(k), Some(v)) => {
                named.insert(f.text_of(k).to_string(), v);
            }
            _ => pos.push(a),
        }
    }
    (pos, named)
}

/// A router or the app: the file and where its name is written where it is made.
type Key = (usize, usize);

struct Router {
    app: bool,
    /// its own prefix (`APIRouter(prefix=…)`)
    prefix: String,
    /// where it is included: into which router, with what prefix
    into: Vec<(Key, String)>,
}

/// Where a name is defined, as a key (the file and where the defined name starts).
fn key_of(p: &Project, res: &Resolver, file: usize, n: Node) -> Option<Key> {
    let name = callee_name(n, p.files[file].lang)?;
    let loc = res.definition(file, name).into_iter().next()?;
    let i = p.file_index(&loc.file)?;
    let at = p.files[i].node_at(loc.line as usize, loc.col)?;
    Some((i, at.start_byte()))
}

/// A string written in the code, or a name whose definition gives it one (`settings.API_PREFIX`, defined as
/// `API_PREFIX: str = "/api"`); otherwise the code itself, marked as only known at runtime.
fn text_of(p: &Project, res: &Resolver, file: usize, n: Node) -> String {
    let f = &p.files[file];
    if let Some(s) = string_value(f.lang, &f.text, n) {
        return s;
    }
    constant(p, res, file, n).unwrap_or_else(|| format!("{{{}}}", f.text_of(n)))
}

fn segments(path: &str) -> Vec<Seg> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| match s.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
            Some(name) => Seg::Dynamic(name.split(':').next().unwrap_or(name).to_string()),
            None => Seg::Literal(s.to_string()),
        })
        .collect()
}

impl FastApi {
    /// Every app and router made in the project, with its own prefix and where it is included.
    fn routers(&self, p: &Project, res: &Resolver) -> HashMap<Key, Router> {
        let mut out: HashMap<Key, Router> = HashMap::new();
        let mut includes: Vec<(usize, Node, Node, Option<Node>)> = vec![];
        for (i, f) in p.files.iter().enumerate() {
            let l = f.lang;
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                stack.extend(kids(n));
                if let Some((name_f, value_f)) = l.follow.declarator(n.kind()) {
                    let (Some(name), Some(value)) = (n.child_by_field_name(name_f), n.child_by_field_name(value_f)) else { continue };
                    let Expr::Call { callee, .. } = (l.expr)(l.unwrap(value), &f.text) else { continue };
                    let app = is_imported(f, callee, &fastapi, "FastAPI");
                    if app || is_imported(f, callee, &fastapi, "APIRouter") {
                        let (_, named) = arguments(f, l.unwrap(value));
                        let prefix = named.get("prefix").map(|v| text_of(p, res, i, *v)).unwrap_or_default();
                        out.insert((i, name.start_byte()), Router { app, prefix, into: vec![] });
                    }
                } else if let Some(fd) = l.call_field(n.kind()) {
                    let Some(callee) = n.child_by_field_name(fd).map(|c| l.unwrap(c)) else { continue };
                    let Some((obj, prop)) = l.member_fields(callee.kind()) else { continue };
                    if callee.child_by_field_name(prop).is_some_and(|x| f.text_of(x) == "include_router") {
                        let (pos, named) = arguments(f, n);
                        if let (Some(parent), Some(child)) = (callee.child_by_field_name(obj), pos.first()) {
                            includes.push((i, parent, *child, named.get("prefix").copied()));
                        }
                    }
                }
            }
        }
        for (i, parent, child, prefix) in includes {
            let (Some(pk), Some(ck)) = (key_of(p, res, i, parent), key_of(p, res, i, child)) else { continue };
            let prefix = prefix.map(|v| text_of(p, res, i, v)).unwrap_or_default();
            if let Some(r) = out.get_mut(&ck) {
                r.into.push((pk, prefix));
            }
        }
        out
    }

    /// The paths a router's routes start with: through every app it is included in (a router included nowhere
    /// starts at its own prefix).
    fn prefixes(&self, all: &HashMap<Key, Router>, k: Key, depth: usize) -> Vec<String> {
        let Some(r) = all.get(&k) else { return vec![String::new()] };
        if r.app || depth > 8 {
            return vec![r.prefix.clone()];
        }
        let above: Vec<String> = r.into.iter().flat_map(|(pk, pre)| self.prefixes(all, *pk, depth + 1).into_iter().map(move |a| format!("{a}{pre}"))).collect();
        let above = if above.is_empty() { vec![String::new()] } else { above };
        above.into_iter().map(|a| format!("{a}{}", r.prefix)).collect()
    }

    /// `TestClient(app)`: a value that sends requests to the app without a network.
    fn is_client(&self, f: &SourceFile, value: Node) -> bool {
        let l = f.lang;
        match (l.expr)(l.unwrap(value), &f.text) {
            Expr::Call { callee, .. } => is_imported(f, callee, &testclient, "TestClient"),
            _ => false,
        }
    }

    /// The name holds a test client: a parameter typed as one or given one by the pytest fixture it names, or a
    /// variable set to one (`client = TestClient(app)`, `with TestClient(app) as client`).
    fn holds_client<'w>(&self, b: &Builder<'w>, name: At<'w>) -> bool {
        let p = b.w.p;
        let f = b.file(name);
        let l = f.lang;
        let text = f.text_of(name.node);
        let mut n = name.node.parent();
        while let Some(x) = n.filter(|x| !l.is_function(x.kind())) {
            n = x.parent();
        }
        if let Some(params) = n.and_then(|func| func.child_by_field_name("parameters")) {
            for prm in kids(params) {
                let id = if l.is_name(prm.kind()) { Some(prm) } else { prm.named_child(0).filter(|x| l.is_name(x.kind())) };
                if id.is_none_or(|id| f.text_of(id) != text) {
                    continue;
                }
                if let Some(t) = prm.child_by_field_name("type") {
                    let t = if t.named_child_count() == 1 { t.named_child(0).unwrap_or(t) } else { t };
                    return is_imported(f, t, &testclient, "TestClient");
                }
                return super::tests::fixture_value(p, name.file, text).is_some_and(|v| self.is_client(b.file(v), v.node));
            }
        }
        let Some(loc) = b.w.res.definition(name.file, name.node).into_iter().next() else { return false };
        let Some(i) = p.file_index(&loc.file) else { return false };
        let df = &p.files[i];
        let Some(at) = df.node_at(loc.line as usize, loc.col) else { return false };
        let value = match df.lang.declared(at) {
            Some((_, value)) => value,
            None => at.parent().filter(|t| t.kind() == "as_pattern_target").and_then(|t| t.parent()).and_then(|a| a.named_child(0)),
        };
        value.is_some_and(|v| self.is_client(df, v))
    }

    /// `client.post("/items", json=…)` on a test client: a request to the endpoint that answers it.
    fn client_request<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let [seg] = chain.segs.as_slice() else { return None };
        let f = b.file(e);
        let l = f.lang;
        if !l.is_name(chain.root.kind()) || seg.node != l.unwrap(e.node) {
            return None;
        }
        let (pos, named) = arguments(f, seg.node);
        let (method, url) = match seg.name.as_str() {
            "request" => (pos.first().and_then(|m| string_value(l, &f.text, *m))?.to_uppercase(), pos.get(1).or(named.get("url")).copied()?),
            m if METHODS.contains(&m) => (m.to_uppercase(), pos.first().or(named.get("url")).copied()?),
            _ => return None,
        };
        if !self.holds_client(b, b.at(e, chain.root)) {
            return None;
        }
        let url = url_of(b.w.p, b.w.res, e.file, url)?;
        let Some(route) = match_route(&b.w.routes, &url, RouteKind::Endpoint, Some(&method)).cloned() else {
            if url.starts_with('/') && !url.contains("{…}") {
                b.gap(format!("{method} {url}: no route in the project answers this URL"), b.place(e));
            }
            return None;
        };
        let how = format!("FastAPI test client: {method} {url} → {} in {}", route.path, route.file);
        Some(vec![Item::Call(b.request(e, &route, format!("{method} {url}"), how, ctx))])
    }

    /// The `Depends(…)` / `Security(…)` call a node is, and what it depends on.
    fn depends<'t>(&self, f: &SourceFile, n: Node<'t>) -> Option<Node<'t>> {
        let l = f.lang;
        let n = l.unwrap(n);
        let Expr::Call { callee, .. } = (l.expr)(n, &f.text) else { return None };
        if !is_imported(f, callee, &fastapi, "Depends") && !is_imported(f, callee, &fastapi, "Security") {
            return None;
        }
        let (pos, named) = arguments(f, n);
        pos.first().copied().or_else(|| named.get("dependency").copied())
    }

    /// Every `Depends(…)` call written inside a node.
    fn depends_in<'t>(&self, f: &SourceFile, n: Node<'t>, out: &mut Vec<Node<'t>>) {
        if self.depends(f, n).is_some() {
            out.push(n);
            return;
        }
        for k in kids(n) {
            self.depends_in(f, k, out);
        }
    }

    /// The dependencies a function asks for, in order: its route's `dependencies=[…]`, then each parameter's, by
    /// its default, its type, or the type alias its type names. Each is the `Depends(…)` call, where it is written.
    fn dependencies<'w>(&self, b: &Builder<'w>, decl: At<'w>) -> Vec<At<'w>> {
        let p = b.w.p;
        let f = b.file(decl);
        let mut out = vec![];
        if let Some(dd) = decl.node.parent().filter(|d| d.kind() == "decorated_definition") {
            for d in kids(dd).into_iter().filter(|d| d.kind() == "decorator") {
                let mut found = vec![];
                if let Some(call) = d.named_child(0).filter(|c| f.lang.call_field(c.kind()).is_some())
                    && let Some(list) = arguments(f, call).1.get("dependencies")
                {
                    self.depends_in(f, *list, &mut found);
                }
                out.extend(found.into_iter().map(|n| b.at(decl, n)));
            }
        }
        let Some(params) = decl.node.child_by_field_name("parameters") else { return out };
        for prm in kids(params) {
            if let Some(v) = prm.child_by_field_name("value").filter(|v| self.depends(f, *v).is_some()) {
                out.push(b.at(decl, v));
                continue;
            }
            let Some(t) = prm.child_by_field_name("type") else { continue };
            let mut found = vec![];
            self.depends_in(f, t, &mut found);
            if !found.is_empty() {
                out.extend(found.into_iter().map(|n| b.at(decl, n)));
                continue;
            }
            // `user: CurrentUser`, where `CurrentUser = Annotated[User, Depends(get_current_user)]`
            let alias = (|| {
                // the type written, inside the node that marks it as one
                let t = if t.named_child_count() == 1 && !f.lang.is_name(t.kind()) { t.named_child(0)? } else { t };
                let name = callee_name(f.lang.unwrap(t), f.lang)?;
                let loc = b.w.res.definition(decl.file, name).into_iter().next()?;
                let i = p.file_index(&loc.file)?;
                let df = &p.files[i];
                let at = df.node_at(loc.line as usize, loc.col)?;
                let mut found = vec![];
                self.depends_in(df, df.lang.declared(at)?.1?, &mut found);
                Some(found.into_iter().map(|n| At { file: i, node: n }).collect::<Vec<_>>())
            })();
            out.extend(alias.unwrap_or_default());
        }
        out
    }
}

impl Adapter for FastApi {
    fn name(&self) -> &'static str {
        "FastAPI"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["python"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "fastapi"
    }

    fn thrown<'w>(&self, b: &mut Builder<'w>, e: At<'w>) -> Option<String> {
        let f = b.file(e);
        let l = f.lang;
        let Expr::Call { callee, args } = (l.expr)(l.unwrap(e.node), &f.text) else { return None };
        // FastAPI's own, or Starlette's that it is built on
        let module = |m: &str| fastapi(m) || m == "starlette" || m.starts_with("starlette.");
        if !is_imported(f, callee, &module, "HTTPException") {
            return None;
        }
        let arg = |name: &str, at: usize| -> Option<Node<'w>> {
            let named = args.iter().find_map(|a| (l.named_arg)(*a, &f.text).filter(|(n, _)| f.text_of(*n) == name).map(|(_, v)| v));
            named.or_else(|| args.iter().filter(|a| (l.named_arg)(**a, &f.text).is_none()).nth(at).copied())
        };
        let written = f.text_of(arg("status_code", 0)?);
        // `status.HTTP_403_FORBIDDEN` is 403
        let status = written
            .split_once("HTTP_")
            .map(|(_, rest)| rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .filter(|d| d.len() == 3)
            .unwrap_or_else(|| written.to_string());
        let detail = arg("detail", 1).map(|d| b.short(b.at(e, d))).unwrap_or_default();
        Some(one_line(&format!("{status} {detail}"), 0))
    }

    fn routes<'w>(&self, p: &'w Project, res: &'w Resolver<'w>) -> Vec<Route<'w>> {
        let routers = self.routers(p, res);
        if routers.is_empty() {
            return vec![];
        }
        let mut out = vec![];
        for (i, f) in p.files.iter().enumerate() {
            let l: &Lang = f.lang;
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                stack.extend(kids(n));
                if !l.reach.decorators.contains(&n.kind()) {
                    continue;
                }
                let Some(func) = n.parent().and_then(|d| d.child_by_field_name("definition")).filter(|d| l.is_function(d.kind())) else { continue };
                let Some(call) = n.named_child(0).filter(|c| l.call_field(c.kind()).is_some()) else { continue };
                let Some(callee) = call.child_by_field_name(l.call_field(call.kind()).unwrap()).map(|c| l.unwrap(c)) else { continue };
                let Some((obj, prop)) = l.member_fields(callee.kind()) else { continue };
                let (Some(o), Some(m)) = (callee.child_by_field_name(obj), callee.child_by_field_name(prop)) else { continue };
                let method = f.text_of(m);
                let (pos, named) = arguments(f, call);
                let methods: Vec<String> = match method {
                    m if METHODS.contains(&m) => vec![m.to_uppercase()],
                    "api_route" => named.get("methods").map(|list| kids(*list).into_iter().filter_map(|x| string_value(l, &f.text, x)).map(|s| s.to_uppercase()).collect()).unwrap_or_else(|| vec!["GET".into()]),
                    "websocket" => vec!["WEBSOCKET".into()],
                    _ => continue,
                };
                let Some(k) = key_of(p, res, i, o).filter(|k| routers.contains_key(k)) else { continue };
                let path = pos.first().or(named.get("path")).map(|v| text_of(p, res, i, *v)).unwrap_or_default();
                for prefix in self.prefixes(&routers, k, 0) {
                    let full = format!("{prefix}{path}");
                    let full = if full.is_empty() { "/".to_string() } else { full };
                    for m in &methods {
                        out.push(Route {
                            kind: RouteKind::Endpoint,
                            segs: segments(&full),
                            path: full.clone(),
                            method: Some(m.clone()),
                            file: f.rel.clone(),
                            decl: At { file: i, node: func },
                            group: crate::model::Group::Server,
                            how: "FastAPI routing",
                            name: None,
                            screen: false,
                        });
                    }
                }
            }
        }
        out.sort_by_key(|r| (r.decl.file, r.decl.node.start_byte()));
        out
    }

    fn starts<'w>(&self, w: &World<'w>) -> Vec<Start<'w>> {
        let p = w.p;
        let routers = self.routers(p, w.res);
        let event = |f: &SourceFile, n: Node| match string_value(f.lang, &f.text, n).as_deref() {
            Some("startup") => Some("when the app starts"),
            Some("shutdown") => Some("when the app stops"),
            _ => None,
        };
        let mut found: Vec<(usize, Node, Node, &'static str)> = vec![];
        for (i, f) in p.files.iter().enumerate() {
            let l = f.lang;
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                stack.extend(kids(n));
                let Some(fd) = l.call_field(n.kind()) else { continue };
                let Some(callee) = n.child_by_field_name(fd).map(|c| l.unwrap(c)) else { continue };
                let (pos, named) = arguments(f, n);
                if is_imported(f, callee, &fastapi, "FastAPI") {
                    if let Some(v) = named.get("lifespan") {
                        found.push((i, n, *v, "when the app starts and stops"));
                    }
                    continue;
                }
                // `app.on_event("startup")` on a definition, `app.add_event_handler("startup", f)`, on an app or a router
                let Some((obj, prop)) = l.member_fields(callee.kind()) else { continue };
                let (Some(o), Some(m)) = (callee.child_by_field_name(obj), callee.child_by_field_name(prop)) else { continue };
                let Some(when) = pos.first().and_then(|e| event(f, *e)) else { continue };
                if key_of(p, w.res, i, o).is_none_or(|k| !routers.contains_key(&k)) {
                    continue;
                }
                match f.text_of(m) {
                    "on_event" => {
                        let Some(dec) = n.parent().filter(|d| l.reach.decorators.contains(&d.kind())) else { continue };
                        if let Some(func) = dec.parent().and_then(|d| d.child_by_field_name("definition")).filter(|d| l.is_function(d.kind())) {
                            found.push((i, dec, func, when));
                        }
                    }
                    "add_event_handler" => {
                        if let Some(h) = pos.get(1) {
                            found.push((i, n, *h, when));
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut out = vec![];
        for (i, by, func, when) in found {
            let func = if p.files[i].lang.is_function(func.kind()) {
                At { file: i, node: func }
            } else {
                match w.res.target(i, func, w.adapters) {
                    crate::resolve::Target::Own { func, .. } if p.files[func.file].lang.is_function(func.node.kind()) => func,
                    _ => continue,
                }
            };
            out.push(Start { kind: "lifecycle", func, by: At { file: i, node: by }, when: when.into(), place: Some(crate::model::Group::Server), optional: false, around: None });
        }
        out.sort_by_key(|s| (s.by.file, s.by.node.start_byte()));
        out
    }

    fn runs<'w>(&self, w: &World<'w>) -> Vec<At<'w>> {
        let mut out = vec![];
        for (i, f) in w.p.files.iter().enumerate() {
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                stack.extend(kids(n));
                if let Some(dep) = self.depends(f, n)
                    && let crate::resolve::Target::Own { func, .. } = w.res.target(i, dep, w.adapters)
                {
                    out.push(func);
                }
            }
        }
        out
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        self.client_request(b, e, chain, ctx)
    }

    fn walk_function<'w>(&self, b: &mut Builder<'w>, decl: At<'w>, ctx: &Ctx, _entry: bool) -> Option<Vec<Item>> {
        let deps = self.dependencies(b, decl);
        if deps.is_empty() {
            return None;
        }
        let mut items = vec![];
        for d in deps {
            let f = b.file(d);
            let Some(target) = self.depends(f, d.node) else { continue };
            if let Some(func) = b.project_decl(b.at(d, target)) {
                items.extend(b.project_call(d, func, &[], ctx));
            }
        }
        items.extend(b.callback(decl, ctx));
        Some(items)
    }
}

#[cfg(test)]
mod tests {
    use super::segments;
    use crate::story::routes::Seg;

    #[test]
    fn path_parameters_are_dynamic_segments() {
        assert_eq!(segments("/api/items/{item_id}"), [Seg::Literal("api".into()), Seg::Literal("items".into()), Seg::Dynamic("item_id".into())]);
        assert_eq!(segments("/files/{path:path}"), [Seg::Literal("files".into()), Seg::Dynamic("path".into())]);
    }
}
