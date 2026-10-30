// Screens and endpoints, as routing adapters report them, and matching a URL written in the code to one of them.
use crate::model::Group;
use crate::resolve::{Resolver, callee_name};
use crate::source::{At, Project};
use crate::story::string_value;
use tree_sitter::Node;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RouteKind {
    Page,
    Endpoint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Literal(String),
    /// a segment filled in at runtime (`[id]`, `{id}`, `:id`, depending on the framework)
    Dynamic(String),
}

#[derive(Clone)]
pub struct Route<'w> {
    pub kind: RouteKind,
    /// the path as the framework writes it, for display ("/books/[id]")
    pub path: String,
    pub segs: Vec<Seg>,
    pub method: Option<String>,
    pub file: String,
    pub decl: At<'w>,
    /// where the route's code runs
    pub group: Group,
    /// "Next.js routing", "Expo Router": how the route was found, for the link record
    pub how: &'static str,
    /// what the place is called when its code is named otherwise (a Flutter `State` class draws the widget that
    /// makes it)
    pub name: Option<String>,
    /// a screen a person opens by its address (a router's page): its opening is a story whatever it reaches, and
    /// whatever opens or draws it refers to that story. A place known only by what it draws (a Flutter widget, a
    /// component with controls) is a story where it crosses a boundary.
    pub screen: bool,
}

/// Matches a URL written in the code ("/api/books?page=1", "/books/{…}") to a route.
/// Literal segments must match exactly; "{…}" (a value only known at runtime) matches a dynamic segment.
pub fn match_route<'a, 'w>(all: &'a [Route<'w>], url: &str, kind: RouteKind, method: Option<&str>) -> Option<&'a Route<'w>> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let score = |r: &Route| -> Option<usize> {
        if r.segs.len() != parts.len() {
            return None;
        }
        let mut s = 0;
        for (seg, part) in r.segs.iter().zip(&parts) {
            match seg {
                Seg::Dynamic(_) => {}
                Seg::Literal(_) if *part == "{…}" => return None,
                Seg::Literal(l) if l == part => s += 1,
                Seg::Literal(_) => return None,
            }
        }
        Some(s)
    };
    let mut best: Option<(&Route, usize)> = None;
    for r in all.iter().filter(|r| r.kind == kind && (method.is_none() || r.method.as_deref() == method)) {
        if let Some(s) = score(r) {
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((r, s));
            }
        }
    }
    best.map(|(r, _)| r)
}

/// The string a name is defined as (`settings.API_PREFIX`, defined as `API_PREFIX: str = "/api"`).
pub fn constant(p: &Project, res: &Resolver, file: usize, n: Node) -> Option<String> {
    let f = &p.files[file];
    let name = callee_name(n, f.lang)?;
    let loc = res.definition(file, name).into_iter().next()?;
    let i = p.file_index(&loc.file)?;
    let df = &p.files[i];
    let at = df.node_at(loc.line as usize, loc.col)?;
    // a parameter stands for what the call being walked gave it
    if at.parent().is_some_and(|x| df.lang.follow.parameter.contains(&x.kind())) {
        let func = std::iter::successors(at.parent(), |x| x.parent()).find(|x| df.lang.is_function(x.kind()))?;
        let value = res.bound_to(crate::source::At { file: i, node: func }.key(), df.text_of(at))?;
        return url_of(p, res, value.file, value.node);
    }
    string_value(df.lang, &df.text, df.lang.declared(at)?.1?)
}

/// What a name stands for: what the call being walked gave a parameter, or the value a name is defined as.
pub fn value_of<'p>(p: &'p Project, res: &Resolver<'p>, file: usize, n: Node) -> Option<At<'p>> {
    let f = &p.files[file];
    let name = callee_name(n, f.lang)?;
    let loc = res.definition(file, name).into_iter().next()?;
    let i = p.file_index(&loc.file)?;
    let df = &p.files[i];
    let at = df.node_at(loc.line as usize, loc.col)?;
    if at.parent().is_some_and(|x| df.lang.follow.parameter.contains(&x.kind())) {
        let func = std::iter::successors(at.parent(), |x| x.parent()).find(|x| df.lang.is_function(x.kind()))?;
        return res.bound_to(At { file: i, node: func }.key(), df.text_of(at));
    }
    df.lang.declared(at)?.1.map(|v| At { file: i, node: v })
}

/// A URL written in the code: a string whose parts written as names are followed to the strings they are defined
/// as (`f"{settings.API_V1_STR}/items/"`), or a name defined as one. Parts only known at runtime are "{…}".
pub fn url_of(p: &Project, res: &Resolver, file: usize, n: Node) -> Option<String> {
    let f = &p.files[file];
    let l = f.lang;
    let n = l.unwrap(n);
    if !l.strings.nodes.contains(&n.kind()) {
        return constant(p, res, file, n);
    }
    let mut out = String::new();
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        if l.strings.fragments.contains(&ch.kind()) {
            out.push_str(f.text_of(ch));
        } else if l.strings.interpolations.contains(&ch.kind()) {
            let value = ch.named_child(0).and_then(|x| constant(p, res, file, x));
            out.push_str(value.as_deref().unwrap_or("{…}"));
        } else if l.strings.nodes.contains(&ch.kind()) {
            out.push_str(&url_of(p, res, file, ch)?);
        }
    }
    Some(out)
}

/// The route a request reaches, by its URL and method, and whether that is known for certain. A URL that starts with
/// a path (`/items`) is the project's own; one that starts with an address only known at runtime (`{…}/items`, a base
/// address read from settings) reaches the route with that path only if the address is the project's server, which
/// the code does not show: that link is inferred.
pub fn match_request<'a, 'w>(all: &'a [Route<'w>], url: &str, method: &str) -> Option<(&'a Route<'w>, bool)> {
    if url.starts_with('/') {
        return match_route(all, url, RouteKind::Endpoint, Some(method)).map(|r| (r, true));
    }
    let path = url.strip_prefix("{…}").filter(|p| p.starts_with('/'))?;
    match_route(all, path, RouteKind::Endpoint, Some(method)).map(|r| (r, false))
}
