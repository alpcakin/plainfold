// Expo Router adapter. Handles: which files are screens (every file in `app/` except layouts `_…` and special
// files `+…`; `(group)` folders and `index` do not show in the URL) and their default-exported component, written
// there or taken from another module (`export { Screen as default } from "…"`, `export default Screen`); screens
// run on the device, also when the file hands it to a package's wrapper (`export default Sentry.wrap(Screen)`); a
// `_layout` runs around every screen in its folder, and the tabs of a tab bar it draws (`<Tabs.Screen name>`) open
// their screens; `useFocusEffect`, code that runs each time a screen comes into view; and navigation with
// `router.push(url)` (`replace`, `navigate`), `router` imported from it or made by `useRouter()`: a reference to the
// screen that answers the URL, which may name `(group)` folders.
use super::{Adapter, Later, is_imported};
use crate::lang::Expr;
use crate::model::Group;
use crate::source::{At, Project, SourceFile, one_line};
use crate::story::{Builder, Chain, Ctx};
use crate::model::Item;
use crate::story::routes::{Route, RouteKind, Seg};
use regex::Regex;
use std::sync::LazyLock;
use tree_sitter::Node;

pub struct ExpoRouter;

static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:src/)?app/(.+)\.[tj]sx?$").unwrap());

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

/// What a file exports as its default: a function written there (`export default function X() {}`,
/// `export default () => …`), a name to follow (`export default X`, `export { X as default } from "…"`), or a name
/// handed to a wrapper (`export default wrap(X)`: the callee, then the name).
enum Default<'t> {
    Here(Node<'t>),
    Name(Node<'t>),
    Wrapped(Node<'t>, Node<'t>),
}

fn default_export<'t>(f: &'t SourceFile) -> Option<Default<'t>> {
    for st in kids(f.tree.root_node()).into_iter().filter(|s| s.kind() == "export_statement") {
        let default = (0..st.child_count()).filter_map(|i| st.child(i as u32)).any(|c| c.kind() == "default");
        if default {
            if let Some(d) = st.child_by_field_name("declaration").filter(|d| d.kind() == "function_declaration") {
                return Some(Default::Here(d));
            }
            let v = f.lang.unwrap(st.child_by_field_name("value")?);
            if f.lang.is_function(v.kind()) {
                return Some(Default::Here(v));
            }
            if let Expr::Call { callee, args } = (f.lang.expr)(v, &f.text)
                && let [arg] = args.as_slice()
                && f.lang.unwrap(*arg).kind() == "identifier"
            {
                return Some(Default::Wrapped(callee, f.lang.unwrap(*arg)));
            }
            return (v.kind() == "identifier").then_some(Default::Name(v));
        }
        // `export { X as default }`, from this module or another
        for clause in kids(st).into_iter().filter(|c| c.kind() == "export_clause") {
            for spec in kids(clause).into_iter().filter(|s| s.kind() == "export_specifier") {
                if spec.child_by_field_name("alias").is_some_and(|a| f.text_of(a) == "default") {
                    return spec.child_by_field_name("name").map(Default::Name);
                }
            }
        }
    }
    None
}

/// The function a file exports as its default. A wrapper from a package (`Sentry.wrap(X)`) runs `X` as it is; one of
/// the project's own may do more around it, which is not followed yet, so that file has none.
fn default_function<'w>(p: &'w Project, res: &crate::resolve::Resolver<'w>, file: usize) -> Option<At<'w>> {
    match default_export(&p.files[file])? {
        Default::Here(d) => Some(At { file, node: d }),
        Default::Name(n) => function_named(p, res, file, n),
        Default::Wrapped(callee, n) => {
            let f = &p.files[file];
            let name = crate::resolve::callee_name(callee, f.lang)?;
            let own = res.definition(file, name).into_iter().any(|loc| p.file_index(&loc.file).is_some());
            if own {
                return None;
            }
            function_named(p, res, file, n)
        }
    }
}

/// The function a name stands for, where the language server finds its definition: one declared with that name, or
/// one given to it (`const Screen = () => …`).
fn function_named<'w>(p: &'w Project, res: &crate::resolve::Resolver<'w>, file: usize, name: Node<'w>) -> Option<At<'w>> {
    let loc = res.definition(file, name).into_iter().next()?;
    let i = p.file_index(&loc.file)?;
    let f = &p.files[i];
    let d = f.node_at(loc.line as usize, loc.col)?.parent()?;
    match d.kind() {
        "function_declaration" => Some(At { file: i, node: d }),
        "variable_declarator" => {
            let v = f.lang.unwrap(d.child_by_field_name("value")?);
            f.lang.is_function(v.kind()).then_some(At { file: i, node: v })
        }
        _ => None,
    }
}

impl ExpoRouter {
    /// `router` made by `useRouter()` from expo-router.
    fn made_by_use_router(&self, b: &Builder, id: At) -> bool {
        let Some(loc) = b.w.res.definition(id.file, id.node).into_iter().next() else { return false };
        let Some(file) = b.w.p.file_index(&loc.file) else { return false };
        let f = &b.w.p.files[file];
        let Some(decl) = f.node_at(loc.line as usize, loc.col).and_then(|n| n.parent()).filter(|p| p.kind() == "variable_declarator") else { return false };
        let Some(init) = decl.child_by_field_name("value").map(|v| f.lang.unwrap(v)) else { return false };
        let Expr::Call { callee, .. } = (f.lang.expr)(init, &f.text) else { return false };
        is_imported(f, callee, &|m| m == "expo-router", "useRouter")
    }
}

impl Adapter for ExpoRouter {
    fn name(&self) -> &'static str {
        "Expo Router"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "expo-router"
    }

    fn starts<'w>(&self, w: &crate::story::World<'w>) -> Vec<super::Start<'w>> {
        // a `_layout` is drawn around every screen in its folder
        let mut out = vec![];
        for (i, f) in w.p.files.iter().enumerate() {
            let Some(m) = FILE.captures(&f.rel) else { continue };
            let parts: Vec<&str> = m[1].split('/').collect();
            if parts.last() != Some(&"_layout") || parts.iter().any(|x| x.starts_with('+')) {
                continue;
            }
            let dirs: Vec<&str> = parts[..parts.len() - 1].iter().copied().filter(|x| !(x.starts_with('(') && x.ends_with(')'))).collect();
            let Some(decl) = default_function(w.p, w.res, i) else { continue };
            out.push(super::Start { kind: "lifecycle", func: decl, by: decl, when: format!("around every screen under /{}", dirs.join("/")), place: Some(Group::Device), optional: true, around: Some(format!("/{}", dirs.join("/"))) });
        }
        out
    }

    fn routes<'w>(&self, p: &'w Project, res: &'w crate::resolve::Resolver<'w>) -> Vec<Route<'w>> {
        let mut out = vec![];
        for (i, f) in p.files.iter().enumerate() {
            let Some(m) = FILE.captures(&f.rel) else { continue };
            let parts: Vec<&str> = m[1].split('/').collect();
            if parts.iter().any(|x| x.starts_with('_') || x.starts_with('+')) {
                continue;
            }
            let segs: Vec<&str> = parts.into_iter().filter(|x| *x != "index" && !(x.starts_with('(') && x.ends_with(')'))).collect();
            let Some(decl) = default_function(p, res, i) else { continue };
            let path = format!("/{}", segs.join("/"));
            let segs = segs
                .iter()
                .map(|s| if s.starts_with('[') && s.ends_with(']') { Seg::Dynamic(s.trim_matches(['[', ']']).to_string()) } else { Seg::Literal(s.to_string()) })
                .collect();
            out.push(Route { kind: RouteKind::Page, path, segs, method: None, file: f.rel.clone(), decl, group: Group::Device, how: "Expo Router", name: None, screen: true });
        }
        out
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        let seg = chain.segs.first().filter(|s| chain.segs.len() == 1 && ["push", "replace", "navigate"].contains(&s.name.as_str()))?;
        if !l.is_name(chain.root.kind()) || !(is_imported(f, chain.root, &|m| m == "expo-router", "router") || self.made_by_use_router(b, b.at(e, chain.root))) {
            return None;
        }
        // a link may name `(group)` folders, which do not show in the URL (`/(auth)/verify` is `/verify`)
        let url = seg.args.as_ref().and_then(|a| a.first()).and_then(|a| b.url_text(b.at(e, *a))).map(|u| {
            let (path, rest) = u.split_at(u.find(['?', '#']).unwrap_or(u.len()));
            let path = path.split('/').filter(|s| !(s.starts_with('(') && s.ends_with(')'))).collect::<Vec<_>>().join("/");
            format!("{}{rest}", if path.is_empty() && u.starts_with('/') { "/" } else { &path })
        });
        Some(vec![super::navigation(b, e, url, ctx)])
    }

    fn element<'w>(&self, b: &mut Builder<'w>, e: At<'w>, tag: At<'w>, ctx: &Ctx) -> Option<Item> {
        // a tab of a tab bar a layout draws (`<Tabs.Screen name="settings">`): pressing it opens the screen it names,
        // in the layout's folder. A stack's screens (`Stack.Screen`) only say how they look, so they open nothing.
        let f = b.file(e);
        let m = FILE.captures(&f.rel)?;
        let parts: Vec<&str> = m[1].split('/').collect();
        if parts.last() != Some(&"_layout") {
            return None;
        }
        let (obj, prop) = f.lang.member_fields(tag.node.kind())?;
        let bar = tag.node.child_by_field_name(obj)?;
        if tag.node.child_by_field_name(prop).is_none_or(|p| f.text_of(p) != "Screen") {
            return None;
        }
        let tabs = |m: &str| m == "expo-router" || m == "expo-router/drawer";
        let named = |n: &str| super::import_of(f, bar).is_some_and(|i| tabs(&i.module) && i.name == n);
        if !named("Tabs") && !named("Drawer") {
            return None;
        }
        let open = if e.node.kind() == "jsx_element" { e.node.child_by_field_name("open_tag")? } else { e.node };
        let name = kids(open)
            .into_iter()
            .filter(|a| a.kind() == "jsx_attribute")
            .find(|a| a.named_child(0).is_some_and(|n| f.text_of(n) == "name"))
            .and_then(|a| a.named_child(1))
            .and_then(|v| b.url_text(b.at(e, v)))?;
        let segs: Vec<&str> = parts[..parts.len() - 1]
            .iter()
            .copied()
            .chain(name.split('/'))
            .filter(|x| !x.is_empty() && *x != "index" && !(x.starts_with('(') && x.ends_with(')')))
            .collect();
        Some(super::navigation(b, e, Some(format!("/{}", segs.join("/"))), ctx))
    }

    fn later<'w>(&self, b: &Builder<'w>, call: At<'w>) -> Option<Later<'w>> {
        let f = b.file(call);
        let l = f.lang;
        let Expr::Call { callee, args } = (l.expr)(call.node, &f.text) else { return None };
        let from_nav = |m: &str| m == "expo-router" || m.starts_with("@react-navigation/");
        if !is_imported(f, callee, &from_nav, "useFocusEffect") {
            return None;
        }
        // the callback usually comes wrapped in a memoizing wrapper, which carries the dependencies
        let mut func = l.unwrap(*args.first()?);
        let mut deps = args.get(1).copied();
        if let Expr::Call { callee: inner, args: inner_args } = (l.expr)(func, &f.text) {
            if b.w.adapters.list.iter().any(|a| a.wrapped(f, func).is_some()) {
                let _ = inner;
                deps = inner_args.get(1).copied();
                func = l.unwrap(*inner_args.first()?);
            }
        }
        let func = b.at(call, func);
        if !b.is_function(func) {
            return None;
        }
        Some(Later { name: "useFocusEffect".into(), func, deps: deps.map(|d| one_line(f.text_of(d), 0)), when: "each time the screen comes into view".into(), given: vec![] })
    }
}
