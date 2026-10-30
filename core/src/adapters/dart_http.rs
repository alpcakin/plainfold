// Dart `http` package adapter. Handles: requests sent with it (`http.get(url)`, `http.post(url, body: …)`, the same on
// a `Client`), to an endpoint of the project when one answers the URL's path and method, otherwise to the address.
// The URL is followed to where it is written: a name to the value it is given, `Uri.parse(text)` and
// `Uri.parse(text).replace(…)` to the text, `Uri.http(host, path)` / `Uri.https(…)` to both, and names written in the
// text to the strings they are defined as. An address only known at runtime followed by a path the project answers
// is an inferred link: the code does not show that the address is the project's own server.
use super::Adapter;
use crate::lang::Expr;
use crate::model::*;
use crate::source::{At, one_line};
use crate::story::routes::{match_request, url_of};
use crate::story::{Builder, Chain, Ctx};
use tree_sitter::Node;

pub struct DartHttp;

const METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "read", "readBytes"];

impl DartHttp {
    /// The URL an expression stands for, as far as the code shows it ("{…}" for a part only known at runtime).
    fn url<'w>(&self, b: &Builder<'w>, n: At<'w>, depth: usize) -> Option<String> {
        if depth > 4 {
            return None;
        }
        let f = b.file(n);
        let l = f.lang;
        let n = b.unwrap(n);
        if l.strings.nodes.contains(&n.node.kind()) {
            return url_of(b.w.p, b.w.res, n.file, n.node);
        }
        if l.is_name(n.node.kind()) {
            // a variable: the value it is given where it is declared
            let loc = b.w.res.definition(n.file, n.node).into_iter().next()?;
            let i = b.w.p.file_index(&loc.file)?;
            let df = &b.w.p.files[i];
            let at = df.node_at(loc.line as usize, loc.col)?;
            return match df.lang.declared(at) {
                Some((_, Some(v))) => self.url(b, At { file: i, node: v }, depth + 1),
                _ => url_of(b.w.p, b.w.res, n.file, n.node),
            };
        }
        let Expr::Call { callee, args } = (l.expr)(n.node, &f.text) else { return url_of(b.w.p, b.w.res, n.file, n.node) };
        let callee = l.unwrap(callee);
        let (obj, prop) = l.member_fields(callee.kind())?;
        let (o, m) = (callee.child_by_field_name(obj)?, f.text_of(callee.child_by_field_name(prop)?));
        let positional: Vec<Node> = args.iter().copied().filter(|a| (l.named_arg)(*a, &f.text).is_none()).collect();
        match m {
            "parse" | "tryParse" if f.text_of(o) == "Uri" => self.url(b, b.at(n, *positional.first()?), depth + 1),
            "http" | "https" if f.text_of(o) == "Uri" => {
                let host = self.url(b, b.at(n, *positional.first()?), depth + 1).unwrap_or("{…}".into());
                let path = positional.get(1).and_then(|p| self.url(b, b.at(n, *p), depth + 1)).unwrap_or_default();
                Some(format!("{m}://{host}{}{path}", if path.starts_with('/') || path.is_empty() { "" } else { "/" }))
            }
            // the same address with other query parameters
            "replace" => self.url(b, b.at(n, o), depth + 1),
            _ => None,
        }
    }
}

impl Adapter for DartHttp {
    fn name(&self) -> &'static str {
        "http (Dart)"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["dart"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "http"
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        let seg = chain.segs.last().filter(|s| s.node == l.unwrap(e.node) && METHODS.contains(&s.name.as_str()))?;
        let (package, _) = b.package_of(e, chain)?;
        if package != "http" {
            return None;
        }
        let args = seg.args.clone().unwrap_or_default();
        let method = seg.name.to_uppercase();
        let url = args.iter().find(|a| (l.named_arg)(**a, &f.text).is_none()).and_then(|a| self.url(b, b.at(e, *a), 0));
        // what the request is given (headers, a body) is worked out first
        let mut out: Vec<Item> = args.iter().skip(1).flat_map(|a| b.expr(b.at(e, l.arg_value(*a, &f.text)), ctx)).collect();
        let place = b.place(e);
        let shown = url.clone().unwrap_or("…".into());
        if let Some((route, exact)) = url.as_deref().and_then(|u| match_request(&b.w.routes, u, &method)).map(|(r, x)| (r.clone(), x)) {
            let how = if exact {
                format!("{}: {method} {} → {}", route.how, route.path, route.file)
            } else {
                format!("the address before {} is only known at runtime; its path and method are those of {method} {} in {}", shown.trim_start_matches("{…}"), route.path, route.file)
            };
            let mut call = b.request(e, &route, format!("{method} {shown}"), how, ctx);
            if !exact {
                call.link.kind = LinkKind::Inferred;
            }
            out.push(Item::Call(call));
            return Some(out);
        }
        let host = url.as_ref().filter(|u| u.starts_with("http://") || u.starts_with("https://")).and_then(|u| u.split('/').nth(2)).filter(|h| !h.contains("{…}")).map(|h| h.to_string());
        let endpoints = b.w.routes.iter().any(|r| r.kind == crate::story::routes::RouteKind::Endpoint);
        if let (true, Some(path)) = (endpoints, url.as_deref().and_then(|u| u.strip_prefix("{…}")).filter(|p| p.starts_with('/'))) {
            b.gap(format!("{method} {shown}: no route in the project answers {path}"), place.clone());
        } else if host.is_none() {
            b.gap(format!("{}: the address is only known at runtime", one_line(b.text(e), 60)), place.clone());
        }
        let to = b.part(Participant { id: format!("out:{}", host.clone().unwrap_or("?".into())), name: host.clone().unwrap_or("unknown address".into()), kind: PartKind::Service, group: Group::Outside, sub: None, at: None });
        let link = if host.is_some() { Link { kind: LinkKind::Exact, how: "URL written in the code".into() } } else { Link { kind: LinkKind::Inferred, how: "address not known statically".into() } };
        let (unused, into) = b.result_use(e);
        out.push(Item::Call(Call {
            id: b.id(),
            from: ctx.this.clone(),
            to,
            label: format!("{method} {shown}"),
            detail: Some(one_line(b.text(e), 0)),
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
            into,
            ends: vec![],
            changes: vec![],
        }));
        Some(out)
    }
}
