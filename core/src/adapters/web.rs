// Web platform adapter (JavaScript runtimes). Handles: `fetch(url, { method })`, the method also through what the
// options spread (`{ ...init }`), to an endpoint of this project when a routing adapter knows the URL, otherwise to
// an outside address, with signals when the answer's status is never read or a failure is never caught; a call
// named by an HTTP method whose URL an endpoint of this project answers (`api.get("/items")`,
// `client.delete({ url: "/items/{id}" })`), as an inferred request to it, whatever library sends it; and responses
// built with the standard `Response`.
use super::{Adapter, Response};
use crate::lang::Expr;
use crate::model::*;
use crate::source::{At, one_line};
use crate::story::routes::{RouteKind, match_request, match_route, url_of};
use crate::story::{Builder, Chain, Ctx};
use tree_sitter::Node;

pub struct Web;

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

/// `json(body, { status })` / `redirect(url, status)`: "404 { error }", "redirect /login".
pub fn response_label<'w>(b: &Builder<'w>, e: At<'w>, args: Vec<Node<'w>>, redirect: bool) -> Response<'w> {
    let f = b.file(e);
    let init = args.get(1).map(|a| f.lang.unwrap(*a));
    let mut status = if redirect { "307".to_string() } else { "200".to_string() };
    if let Some(init) = init {
        if init.kind() == "object" {
            for p in kids(init).into_iter().filter(|p| p.kind() == "pair") {
                if p.child_by_field_name("key").is_some_and(|k| f.text_of(k) == "status") {
                    status = p.child_by_field_name("value").map(|v| f.text_of(v).to_string()).unwrap_or(status);
                }
            }
        }
        if redirect && init.kind() == "number" {
            status = f.text_of(init).to_string();
        }
    }
    let body = args.first().map(|a| b.short(b.at(e, *a))).unwrap_or_default();
    Response { label: one_line(&format!("{} {body}", if redirect { "redirect".to_string() } else { status }), 0), args: args.into_iter().map(|a| b.at(e, a)).collect() }
}

impl Web {
    /// The global `fetch`: not imported, and not a function of the project.
    fn is_fetch(&self, b: &Builder, callee: At) -> bool {
        let f = b.file(callee);
        let c = f.lang.unwrap(callee.node);
        f.lang.is_name(c.kind()) && f.text_of(c) == "fetch" && f.import_at("fetch", c).is_none() && b.project_decl(b.at(callee, c)).is_none()
    }

    fn global(&self, b: &Builder, n: At, name: &str) -> bool {
        let f = b.file(n);
        f.lang.is_name(n.node.kind()) && f.text_of(n.node) == name && f.import_at(name, n.node).is_none() && b.project_decl(n).is_none()
    }

    /// The method an options object gives (`{ method: "DELETE" }`), also through what it spreads (`{ ...init }`) or a
    /// name that stands for one: a parameter, for what the call being walked gave it. The last one written wins, as
    /// it does when the code runs.
    fn method_of<'w>(&self, b: &Builder<'w>, n: At<'w>, depth: u8) -> Option<String> {
        let f = b.file(n);
        let node = f.lang.unwrap(n.node);
        if depth > 4 {
            return None;
        }
        if node.kind() != "object" {
            let v = crate::story::routes::value_of(b.w.p, b.w.res, n.file, node)?;
            return self.method_of(b, v, depth + 1);
        }
        let mut method = None;
        for p in kids(node) {
            match p.kind() {
                "pair" if p.child_by_field_name("key").is_some_and(|k| f.text_of(k) == "method") => {
                    if let Some(s) = p.child_by_field_name("value").and_then(|v| b.url_text(b.at(n, v))) {
                        method = Some(s.to_uppercase());
                    }
                }
                "spread_element" => {
                    if let Some(m) = p.named_child(0).and_then(|x| self.method_of(b, b.at(n, x), depth + 1)) {
                        method = Some(m);
                    }
                }
                _ => {}
            }
        }
        method
    }

    fn fetch<'w>(&self, b: &mut Builder<'w>, e: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Vec<Item> {
        // the address as written, or held in a name defined as one (`const API = "https://…"`)
        let url = args.first().and_then(|a| url_of(b.w.p, b.w.res, e.file, *a));
        let method = args.get(1).and_then(|a| self.method_of(b, b.at(e, *a), 0)).unwrap_or_else(|| "GET".to_string());
        let place = b.place(e);
        let (unused, into) = b.result_use(e);
        let detail = one_line(b.text(e), 0);
        if let Some(u) = url.as_ref().filter(|u| u.starts_with('/')) {
            if let Some(route) = match_route(&b.w.routes, u, RouteKind::Endpoint, Some(&method)).cloned() {
                let how = format!("{}: {} → {}, export {method}", route.how, route.path, route.file);
                let call = b.request(e, &route, format!("{method} {u}"), how, ctx);
                self.check_answer(b, e, &call);
                return vec![Item::Call(call)];
            }
            b.gap(format!("{method} {u}: no route in the project answers this URL"), place.clone());
        }
        let host = url.as_ref().filter(|u| u.starts_with("http://") || u.starts_with("https://")).and_then(|u| u.split('/').nth(2)).map(|h| h.to_string());
        let to = b.part(Participant { id: format!("out:{}", host.clone().unwrap_or("?".into())), name: host.clone().unwrap_or("unknown address".into()), kind: PartKind::Service, group: Group::Outside, sub: None, at: None });
        if host.is_none() {
            b.gap(format!("{detail}: the address is only known at runtime"), place.clone());
        }
        let link = if host.is_some() { Link { kind: LinkKind::Exact, how: "URL written in the code".into() } } else { Link { kind: LinkKind::Inferred, how: "address not known statically".into() } };
        vec![Item::Call(Call {
            id: b.id(),
            from: ctx.this.clone(),
            to,
            label: format!("{method} {}", url.unwrap_or("…".into())),
            detail: Some(detail),
            link,
            effect: None,
            at: place,
            target: None,
            body: None,
            note: None,
            opens: None,
            story: None,
            table: None,
            touches: None,
            unused,
            into: into.clone(),
            ends: vec![],
            changes: vec![],
        })]
    }

    /// A call named by an HTTP method, given a URL that an endpoint of the project answers with that method: first,
    /// or as the `url` of the object it is given first. It is a request to that endpoint whatever sends it (a library,
    /// a client generated from an API schema, a wrapper of the project's own), but only its name and its URL say so,
    /// so the link is inferred. Other calls of that name are left as they are.
    fn named_request<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        let seg = chain.segs.last().filter(|s| s.node == l.unwrap(e.node))?;
        let method = seg.name.to_uppercase();
        if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].contains(&method.as_str()) {
            return None;
        }
        let args = seg.args.clone().unwrap_or_default();
        let first = l.unwrap(*args.first()?);
        let given = first.kind() == "object";
        let at_url = if given {
            kids(first).into_iter().filter(|p| p.kind() == "pair").find(|p| p.child_by_field_name("key").is_some_and(|k| f.text_of(k) == "url")).and_then(|p| p.child_by_field_name("value"))?
        } else {
            first
        };
        let url = url_of(b.w.p, b.w.res, e.file, at_url)?;
        let (route, exact) = match_request(&b.w.routes, &url, &method).map(|(r, x)| (r.clone(), x))?;
        // what the request is given (a body, options) is worked out first
        let mut out: Vec<Item> = args.iter().skip(if given { 0 } else { 1 }).flat_map(|a| b.expr(b.at(e, *a), ctx)).collect();
        let base = if exact { String::new() } else { format!("; the address before {} is only known at runtime", url.trim_start_matches("{…}")) };
        let how = format!("a call named {} with a URL that {} answers ({method} {} in {}){base}; that it sends a request is read from its name", seg.name, route.how, route.path, route.file);
        let mut call = b.request(e, &route, format!("{method} {url}"), how, ctx);
        call.link.kind = LinkKind::Inferred;
        out.push(Item::Call(call));
        Some(out)
    }

    /// Is the answer's status read, and is a failure caught? Either as `fetch(…).then(res => …).catch(…)` or as
    /// `const res = await fetch(…)` inside `try`.
    fn check_answer<'w>(&self, b: &mut Builder<'w>, e: At<'w>, call: &Call) {
        let f = b.file(e);
        let l = f.lang;
        let failure = l.promise.last().copied().unwrap_or("");
        let (mut checked, mut caught) = (false, false);
        let mut res: Option<String> = None;
        let mut scope: Option<Node> = None;
        let mut cur = e.node;
        // walk out through `.then(…)` / `.catch(…)` written on the call
        while let Some(member) = cur.parent().filter(|p| l.member_fields(p.kind()).is_some()) {
            let (obj, prop) = l.member_fields(member.kind()).unwrap();
            let Some(name) = member.child_by_field_name(prop).map(|p| f.text_of(p)) else { break };
            if member.child_by_field_name(obj) != Some(cur) || !l.promise.contains(&name) {
                break;
            }
            let Some(outer) = member.parent().filter(|p| l.call_field(p.kind()).is_some()) else { break };
            let handler = match (l.expr)(outer, &f.text) {
                Expr::Call { args, .. } => args.first().copied(),
                _ => None,
            };
            if name == failure {
                caught = true;
            }
            if name == l.promise[0] && res.is_none() {
                if let Some(h) = handler.filter(|h| l.is_function(h.kind())) {
                    if let Some(p) = (l.params)(h, &f.text).first().filter(|p| p.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$')) {
                        res = Some(p.clone());
                        scope = Some(h);
                    }
                }
            }
            cur = outer;
        }
        if res.is_none() {
            res = b.receiver(e).filter(|r| r.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '$'));
            let mut p = e.node.parent();
            while let Some(x) = p {
                if l.is_function(x.kind()) {
                    scope = Some(x);
                    break;
                }
                p = x.parent();
            }
        }
        if let (Some(res), Some(scope)) = (&res, scope) {
            let mut stack = vec![scope];
            while let Some(n) = stack.pop() {
                if let Some((obj, prop)) = l.member_fields(n.kind()) {
                    if n.child_by_field_name(obj).is_some_and(|o| f.text_of(o) == res) && n.child_by_field_name(prop).is_some_and(|p| ["ok", "status"].contains(&f.text_of(p))) {
                        checked = true;
                        break;
                    }
                }
                stack.extend(kids(n));
            }
        }
        let mut n = e.node;
        while let Some(p) = n.parent() {
            if l.is_function(p.kind()) {
                break;
            }
            if p.kind() == "try_statement" && p.child_by_field_name("body") == Some(n) {
                caught = true;
            }
            n = p;
        }
        let mut codes: Vec<String> = vec![];
        fn returns(list: &[Item], from: &str, out: &mut Vec<String>) {
            for i in list {
                match i {
                    Item::Return { from: f, label, .. } if f == from => {
                        if let Some(c) = label.get(..3).filter(|c| c.starts_with(['4', '5']) && c.chars().all(|x| x.is_ascii_digit())) {
                            if label.len() == 3 || !label.as_bytes()[3].is_ascii_alphanumeric() {
                                if !out.contains(&c.to_string()) {
                                    out.push(c.to_string());
                                }
                            }
                        }
                    }
                    Item::Frame { items, els, .. } => {
                        returns(items, from, out);
                        returns(els.as_deref().unwrap_or(&[]), from, out);
                    }
                    _ => {}
                }
            }
        }
        returns(call.body.as_deref().unwrap_or(&[]), &call.to, &mut codes);
        let codes = if codes.is_empty() { String::new() } else { format!(" ({})", codes.join(", ")) };
        let Expr::Call { callee, args } = (l.expr)(e.node, &f.text) else { return };
        let text = format!("{}({})", f.text_of(callee), args.iter().map(|a| b.short(b.at(e, *a))).collect::<Vec<_>>().join(", "));
        let place = b.place(e);
        if !checked && !caught {
            b.signal(format!("{text} never reads res.ok or res.status and has no catch: error answers{codes} and network failures are handled like success"), place);
        } else if !checked {
            b.signal(format!("{text} never reads res.ok or res.status: error answers{codes} are handled like success"), place);
        } else if !caught {
            b.signal(format!("{text} has no catch: a network failure is not handled here"), place);
        }
    }
}

impl Adapter for Web {
    fn name(&self) -> &'static str {
        "web platform"
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        if !chain.segs.is_empty() {
            return self.named_request(b, e, chain, ctx);
        }
        let f = b.file(e);
        let Expr::Call { callee, args } = (f.lang.expr)(f.lang.unwrap(e.node), &f.text) else { return None };
        if !self.is_fetch(b, b.at(e, callee)) {
            return None;
        }
        Some(self.fetch(b, e, &args, ctx))
    }

    fn response<'w>(&self, b: &mut Builder<'w>, e: At<'w>) -> Option<Response<'w>> {
        let f = b.file(e);
        let l = f.lang;
        if e.node.kind() == "new_expression" {
            let c = e.node.child_by_field_name("constructor")?;
            if !self.global(b, b.at(e, c), "Response") {
                return None;
            }
            return Some(response_label(b, e, e.node.child_by_field_name("arguments").map(kids).unwrap_or_default(), false));
        }
        let Expr::Call { callee, args } = (l.expr)(e.node, &f.text) else { return None };
        let c = l.unwrap(callee);
        let (obj, prop) = l.member_fields(c.kind())?;
        let fun = f.text_of(c.child_by_field_name(prop)?);
        if !self.global(b, b.at(e, c.child_by_field_name(obj)?), "Response") || !["json", "redirect"].contains(&fun) {
            return None;
        }
        Some(response_label(b, e, args, fun == "redirect"))
    }
}
