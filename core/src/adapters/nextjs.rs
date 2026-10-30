// Next.js adapter (app router). Handles: which files are screens (`page`) and endpoints (`route`) and the URL
// each answers; whether a screen runs in the browser (`"use client"`) or on the server; what Next.js runs by itself
// (a `layout` around every page under its folder, `generateMetadata` before a page is shown, `generateStaticParams`
// when the site is built, `middleware` (called `proxy` from Next.js 16) before
// each request its `config.matcher` names); responses built with
// `NextResponse`; reading the incoming request (`cookies()`, `headers()`); navigation with `useRouter` and
// `<Link href>` (a call that opens another screen, and a signal when no screen answers the URL); and the calls of
// `next/navigation` that end the work by throwing inside Next.js (`notFound()`, `redirect(url)`, `forbidden()`),
// which are rules named by what the request gets.
use super::{Adapter, Response, Start, is_imported};
use crate::lang::Expr;
use crate::model::*;
use crate::source::{At, Project, SourceFile, one_line};
use crate::story::routes::{Route, RouteKind, Seg, match_route};
use crate::story::{Builder, Chain, Ctx, World};
use regex::Regex;
use std::sync::LazyLock;
use tree_sitter::Node;

pub struct NextJs;

const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
/// Calls that end the work by throwing what Next.js answers with, and what the request gets.
const ENDS: &[(&str, &str)] = &[("notFound", "404"), ("forbidden", "403"), ("unauthorized", "401"), ("redirect", "redirect"), ("permanentRedirect", "redirect")];
static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:src/)?app/((?:.*/)?)(page|route|layout)\.(tsx?|jsx?)$").unwrap());
static MIDDLEWARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:src/)?(?:middleware|proxy)\.(ts|js)$").unwrap());

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

/// Exported function declarations of a file: (name, declaration, is default export).
fn exported<'t>(f: &'t SourceFile) -> Vec<(String, Node<'t>, bool)> {
    let mut out = vec![];
    for st in kids(f.tree.root_node()) {
        if st.kind() != "export_statement" {
            continue;
        }
        let default = (0..st.child_count()).filter_map(|i| st.child(i as u32)).any(|c| c.kind() == "default");
        if let Some(d) = st.child_by_field_name("declaration").filter(|d| d.kind() == "function_declaration") {
            if let Some(n) = d.child_by_field_name("name") {
                out.push((f.text_of(n).to_string(), d, default));
            }
        }
    }
    out
}

fn segs(path: &str) -> Vec<Seg> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|s| if s.starts_with('[') && s.ends_with(']') { Seg::Dynamic(s.trim_matches(['[', ']']).to_string()) } else { Seg::Literal(s.to_string()) })
        .collect()
}

impl NextJs {
    /// What a call that ends the work answers (`404`, `redirect /login`), when it is one.
    fn ending<'w>(&self, b: &Builder<'w>, e: At<'w>) -> Option<String> {
        let f = b.file(e);
        let l = f.lang;
        let Expr::Call { callee, args } = (l.expr)(l.unwrap(e.node), &f.text) else { return None };
        let c = l.unwrap(callee);
        if !l.is_name(c.kind()) {
            return None;
        }
        let (_, gets) = ENDS.iter().find(|(n, _)| is_imported(f, c, &|m| m == "next/navigation", n))?;
        if *gets != "redirect" {
            return Some(gets.to_string());
        }
        let to = args.first().map(|a| b.url_text(b.at(e, *a)).unwrap_or_else(|| b.short(b.at(e, *a)))).unwrap_or_default();
        Some(one_line(&format!("redirect {to}"), 40))
    }

    /// `router` made by `useRouter()` from Next.js.
    fn is_router(&self, b: &Builder, id: At) -> bool {
        let Some(loc) = b.w.res.definition(id.file, id.node).into_iter().next() else { return false };
        let Some(file) = b.w.p.file_index(&loc.file) else { return false };
        let f = &b.w.p.files[file];
        let Some(n) = f.node_at(loc.line as usize, loc.col) else { return false };
        let Some(decl) = n.parent().filter(|p| p.kind() == "variable_declarator") else { return false };
        let Some(init) = decl.child_by_field_name("value").map(|v| f.lang.unwrap(v)) else { return false };
        let Expr::Call { callee, .. } = (f.lang.expr)(init, &f.text) else { return false };
        is_imported(f, callee, &|m| m == "next/navigation" || m == "next/router", "useRouter")
    }
}

impl Adapter for NextJs {
    fn name(&self) -> &'static str {
        "Next.js"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "next"
    }

    fn runs_on<'w>(&self, b: &Builder<'w>, decl: At<'w>, made: Option<At<'w>>) -> Option<(Group, &'static str)> {
        // a server action: a function of a file that starts with "use server", or one that starts with it; one a call
        // returned is where its name is stored
        let f = b.file(made.unwrap_or(decl));
        if f.lang.server != "typescript" {
            return None;
        }
        let directive = |n: Node| kids(n).into_iter().find(|s| s.kind() != "comment").is_some_and(|s| s.kind() == "expression_statement" && s.named_child(0).is_some_and(|x| x.kind() == "string" && matches!(f.text_of(x).trim_matches(['"', '\'']), "use server")));
        let inline = decl.node.child_by_field_name("body").is_some_and(|body| body.kind() == "statement_block" && directive(body));
        (directive(f.tree.root_node()) || inline).then_some((Group::Server, "a server action: the browser calls it, the server runs it"))
    }

    fn starts<'w>(&self, w: &World<'w>) -> Vec<Start<'w>> {
        let mut out = vec![];
        for (i, f) in w.p.files.iter().enumerate() {
            let server = |f: &SourceFile| if f.text.trim_start().starts_with("\"use client\"") || f.text.trim_start().starts_with("'use client'") { Group::Browser } else { Group::Server };
            // what runs before each request, where `config.matcher` says
            if MIDDLEWARE.is_match(&f.rel) {
                let matcher = kids(f.tree.root_node()).into_iter().filter(|st| st.kind() == "export_statement").find_map(|st| {
                    let d = st.child_by_field_name("declaration")?;
                    let v = kids(d).into_iter().find(|x| x.kind() == "variable_declarator" && x.child_by_field_name("name").is_some_and(|n| f.text_of(n) == "config"))?;
                    let pair = kids(v.child_by_field_name("value")?).into_iter().find(|p| p.kind() == "pair" && p.child_by_field_name("key").is_some_and(|k| f.text_of(k) == "matcher"))?;
                    // the value as written, without the comments inside it
                    static COMMENTS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/|//[^\n]*").unwrap());
                    Some(one_line(&COMMENTS.replace_all(f.text_of(pair.child_by_field_name("value")?), ""), 60))
                });
                let when = match matcher {
                    Some(m) => format!("before each request matching {}", one_line(&m, 40)),
                    None => "before every request".to_string(),
                };
                for (name, decl, default) in exported(f) {
                    if name == "middleware" || name == "proxy" || default {
                        out.push(Start { kind: "lifecycle", func: At { file: i, node: decl }, by: At { file: i, node: decl }, when: when.clone(), place: Some(Group::Server), optional: false, around: None });
                    }
                }
                // `export default withAuth(function middleware(req) { … }, options)`: the function a wrapper is given
                for st in kids(f.tree.root_node()).into_iter().filter(|st| st.kind() == "export_statement") {
                    if let Some(Expr::Call { args, .. }) = st.child_by_field_name("value").map(|v| (f.lang.expr)(v, &f.text))
                        && let Some(func) = args.into_iter().map(|a| f.lang.unwrap(a)).find(|a| f.lang.is_function(a.kind()))
                    {
                        out.push(Start { kind: "lifecycle", func: At { file: i, node: func }, by: At { file: i, node: func }, when: when.clone(), place: Some(Group::Server), optional: false, around: None });
                    }
                }
                continue;
            }
            let Some(m) = FILE.captures(&f.rel) else { continue };
            let dirs: Vec<&str> = m[1].split('/').filter(|s| !s.is_empty() && !(s.starts_with('(') && s.ends_with(')'))).collect();
            let path = format!("/{}", dirs.join("/"));
            for (name, decl, default) in exported(f) {
                let at = At { file: i, node: decl };
                // a layout is drawn around every page under its folder
                if &m[2] == "layout" && default {
                    out.push(Start { kind: "lifecycle", func: at, by: at, when: format!("around every page under {path}"), place: Some(server(f)), optional: true, around: Some(path.clone()) });
                }
                // the addresses a dynamic page is built for, worked out when the site is built
                if (&m[2] == "page" || &m[2] == "layout" || &m[2] == "route") && name == "generateStaticParams" {
                    out.push(Start { kind: "lifecycle", func: at, by: at, when: format!("when the site is built, for the addresses under {path}"), place: Some(Group::Server), optional: false, around: None });
                }
                // what a page or a layout says about itself in the document's head, worked out on the server first
                if (&m[2] == "page" || &m[2] == "layout") && name == "generateMetadata" {
                    out.push(Start { kind: "lifecycle", func: at, by: at, when: format!("before {path} is shown, for its title and tags"), place: Some(Group::Server), optional: false, around: None });
                }
            }
        }
        out
    }

    fn routes<'w>(&self, p: &'w Project, _res: &'w crate::resolve::Resolver<'w>) -> Vec<Route<'w>> {
        let mut out = vec![];
        for (i, f) in p.files.iter().enumerate() {
            let Some(m) = FILE.captures(&f.rel) else { continue };
            // route groups "(x)" don't change the URL
            let dirs: Vec<&str> = m[1].split('/').filter(|s| !s.is_empty() && !(s.starts_with('(') && s.ends_with(')'))).collect();
            let path = format!("/{}", dirs.join("/"));
            let client = f.text.trim_start().starts_with("\"use client\"") || f.text.trim_start().starts_with("'use client'");
            let group = if client { Group::Browser } else { Group::Server };
            for (name, decl, default) in exported(f) {
                let decl = At { file: i, node: decl };
                if &m[2] == "page" && default {
                    out.push(Route { kind: RouteKind::Page, path: path.clone(), segs: segs(&path), method: None, file: f.rel.clone(), decl, group, how: "Next.js routing", name: None, screen: true });
                }
                if &m[2] == "route" && METHODS.contains(&name.as_str()) {
                    out.push(Route { kind: RouteKind::Endpoint, path: path.clone(), segs: segs(&path), method: Some(name), file: f.rel.clone(), decl, group: Group::Server, how: "Next.js routing", name: None, screen: false });
                }
            }
        }
        out
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        // notFound() / redirect(…): the work ends here, as with a throw
        if chain.segs.is_empty() && self.ending(b, e).is_some() {
            return Some(b.throw(e, Some(e), ctx));
        }
        // cookies().get(…) / headers().get(…): reading the incoming request
        if let Expr::Call { callee, .. } = (l.expr)(chain.root, &f.text) {
            let callee = l.unwrap(callee);
            if l.is_name(callee.kind()) && ["cookies", "headers"].iter().any(|n| is_imported(f, callee, &|m| m == "next/headers", n)) {
                let label = format!("{}(){}", f.text_of(callee), chain.segs.iter().map(|s| b.seg_text(e.file, s)).collect::<String>());
                return Some(vec![Item::Own { id: b.id(), at: ctx.this.clone(), kind: SelfKind::Reads, label: one_line(&label, 48), detail: Some("reads the incoming request (next/headers)".into()), place: b.place(e) }]);
            }
        }
        // router.push("/…") / router.replace("/…")
        if chain.segs.len() == 1 && ["push", "replace"].contains(&chain.segs[0].name.as_str()) && l.is_name(chain.root.kind()) && self.is_router(b, b.at(e, chain.root)) {
            let seg = &chain.segs[0];
            let url = seg.args.as_ref().and_then(|a| a.first()).and_then(|a| b.url_text(b.at(e, *a)));
            return Some(vec![super::navigation(b, e, url, ctx)]);
        }
        None
    }

    fn thrown<'w>(&self, b: &mut Builder<'w>, e: At<'w>) -> Option<String> {
        self.ending(b, e)
    }

    fn response<'w>(&self, b: &mut Builder<'w>, e: At<'w>) -> Option<Response<'w>> {
        let f = b.file(e);
        let l = f.lang;
        let next = |n: Node| is_imported(f, n, &|m| m == "next/server", "NextResponse");
        let (args, redirect) = match e.node.kind() {
            "new_expression" if e.node.child_by_field_name("constructor").is_some_and(next) => (e.node.child_by_field_name("arguments").map(kids).unwrap_or_default(), false),
            _ => {
                let Expr::Call { callee, args } = (l.expr)(e.node, &f.text) else { return None };
                let c = l.unwrap(callee);
                let (obj, prop) = l.member_fields(c.kind())?;
                let fun = f.text_of(c.child_by_field_name(prop)?);
                if !next(c.child_by_field_name(obj)?) || !["json", "redirect"].contains(&fun) {
                    return None;
                }
                (args, fun == "redirect")
            }
        };
        Some(super::web::response_label(b, e, args, redirect))
    }

    fn check<'w>(&self, b: &mut Builder<'w>, decl: At<'w>) {
        // <Link href="…"> and router.push("…") that point at a page that does not exist
        let f = b.file(decl);
        let l = f.lang;
        let mut stack = vec![decl.node];
        while let Some(n) = stack.pop() {
            stack.extend(kids(n));
            let check = |b: &mut Builder<'w>, url: Option<String>, at: Node<'w>, how: String| {
                let Some(url) = url.filter(|u| u.starts_with('/')) else { return };
                if match_route(&b.w.routes, &url, RouteKind::Page, None).is_none() {
                    let place = b.place(b.at(decl, at));
                    b.signal(format!("{how} goes to {url}, but no page in app/ answers that URL"), place);
                }
            };
            if matches!(n.kind(), "jsx_self_closing_element" | "jsx_opening_element") {
                let Some(tag) = n.child_by_field_name("name") else { continue };
                if !super::import_of(f, tag).is_some_and(|i| i.module == "next/link") {
                    continue;
                }
                for a in kids(n).into_iter().filter(|a| a.kind() == "jsx_attribute") {
                    let k = kids(a);
                    if k.first().is_none_or(|x| f.text_of(*x) != "href") {
                        continue;
                    }
                    let Some(v) = k.get(1).map(|v| if v.kind() == "jsx_expression" { v.named_child(0).unwrap_or(*v) } else { *v }) else { continue };
                    let url = b.url_text(b.at(decl, v));
                    check(b, url, a, format!("<Link href={{{}}}>", one_line(f.text_of(v), 30)));
                }
            }
            if let Expr::Call { callee, args } = (l.expr)(n, &f.text) {
                let c = l.unwrap(callee);
                let Some((obj, prop)) = l.member_fields(c.kind()) else { continue };
                let (Some(o), Some(p)) = (c.child_by_field_name(obj), c.child_by_field_name(prop)) else { continue };
                if !["push", "replace"].contains(&f.text_of(p)) || !l.is_name(o.kind()) || !self.is_router(b, b.at(decl, o)) {
                    continue;
                }
                let url = args.first().and_then(|a| b.url_text(b.at(decl, *a)));
                check(b, url, n, one_line(f.text_of(n), 30));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FILE;

    #[test]
    fn only_files_named_page_route_or_layout() {
        let folder = |f: &str| FILE.captures(f).map(|m| (m[1].to_string(), m[2].to_string()));
        assert_eq!(folder("app/page.tsx"), Some(("".into(), "page".into())));
        assert_eq!(folder("src/app/docs/layout.tsx"), Some(("docs/".into(), "layout".into())));
        assert_eq!(folder("app/api/items/route.ts"), Some(("api/items/".into(), "route".into())));
        assert_eq!(folder("app/auth/login-page.tsx"), None);
        assert_eq!(folder("app/components/my-layout.tsx"), None);
    }
}
