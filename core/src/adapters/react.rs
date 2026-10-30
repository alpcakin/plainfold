// React adapter. Handles: components (a function used as a JSX element, also wrapped in `forwardRef` or `memo`),
// their render passes, state (`useState`) and its setters, effects (`useEffect`, `useLayoutEffect`), custom hooks
// (functions named `use…`, React's own rule), memoized callbacks (`useCallback`) and refs (`useRef(f).current`),
// shared values (`useContext` to where its provider gives it, what `useMemo` returns), and user actions written as
// JSX event attributes (`onPress`, `onSubmit`, `action`): in a screen, and in any other component, which is then a
// place of its own. JSX is TypeScript/JavaScript syntax, so the node types below are that grammar's.
use super::{Adapter, Later, TriggerSite, is_imported};
use crate::lang::{Expr, Stmt};
use crate::model::*;
use crate::resolve::{Resolver, ValueRules};
use crate::source::{At, NodeKey, SourceFile, one_line};
use crate::story::routes::{Route, RouteKind, Seg};
use crate::story::{Builder, Chain, Ctx, World, boring};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use tree_sitter::Node;

pub struct React;

const REACT: &dyn Fn(&str) -> bool = &|m: &str| m == "react";

fn hook_like(name: &str) -> bool {
    let mut c = name.chars();
    name.starts_with("use") && c.nth(3).is_some_and(|ch| ch.is_ascii_uppercase())
}

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

/// Attribute name and value expression of a JSX attribute (`onPress={save}` → ("onPress", save)).
fn attribute<'t>(a: Node<'t>, src: &str) -> Option<(String, Option<Node<'t>>)> {
    if a.kind() != "jsx_attribute" {
        return None;
    }
    let k = kids(a);
    let name = src[k.first()?.byte_range()].to_string();
    let value = k.get(1).and_then(|v| if v.kind() == "jsx_expression" { v.named_child(0) } else { None });
    Some((name, value))
}

/// Events the platform fires by itself, not the user: a view laid out, something loaded or failed to, an animation or
/// a transition ended, media played to its end.
const NOT_USER: &[&str] = &["layout", "load", "loadstart", "loadend", "error", "contentsizechange", "animationstart", "animationend", "animationiteration", "transitionend", "ended"];

/// The event a JSX attribute attaches a handler to (`onPress` → "press"; `action` and `onSubmit` → "submit"), or
/// none when it is no user action. `onChange` is left out.
fn event_of(name: &str) -> Option<String> {
    let event = if name == "action" || name == "onSubmit" {
        "submit".to_string()
    } else if name.starts_with("on") && name[2..].starts_with(|c: char| c.is_ascii_uppercase()) {
        name[2..].to_lowercase()
    } else {
        return None;
    };
    (event != "change" && !NOT_USER.contains(&event.as_str())).then_some(event)
}

/// The start or end of a press, a touch or a click. On a control that also handles the whole gesture (`onPress`,
/// `onClick`) it is a phase of that gesture (most often what the control looks like while held), not an action.
const PHASES: &[&str] = &["pressin", "pressout", "mousedown", "mouseup", "touchstart", "touchend", "pointerdown", "pointerup"];

fn handles_whole(el: Node, src: &str) -> bool {
    kids(el).into_iter().any(|a| attribute(a, src).is_some_and(|(n, _)| n == "onPress" || n == "onClick"))
}

/// Where code inside `root` uses the name `name`: as a value, or as a member of another (`props.name`). Not where it is
/// declared, nor an attribute that only bears the same name.
fn uses_of<'t>(root: Node<'t>, name: &str, src: &str) -> Vec<Node<'t>> {
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let p = n.parent();
        let declares = |p: Node| match p.kind() {
            "formal_parameters" => true,
            "variable_declarator" | "function_declaration" => p.child_by_field_name("name") == Some(n),
            "required_parameter" | "optional_parameter" => p.child_by_field_name("pattern") == Some(n),
            _ => false,
        };
        let used = match n.kind() {
            "identifier" => !p.is_some_and(declares),
            "property_identifier" => p.is_some_and(|p| p.kind() == "member_expression"),
            "shorthand_property_identifier" => true,
            _ => false,
        };
        if used && &src[n.byte_range()] == name {
            out.push(n);
        }
        stack.extend(kids(n));
    }
    out
}

/// Whether code inside `root` spreads values onto a JSX element (`<Item {...props} />`).
fn spreads(root: Node) -> bool {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "jsx_expression" && n.named_child(0).is_some_and(|x| x.kind() == "spread_element") {
            return true;
        }
        stack.extend(kids(n));
    }
    false
}

/// Whether code holds a JSX attribute that attaches a handler to a user action.
fn has_action(n: Node, src: &str) -> bool {
    let mut stack = vec![n];
    while let Some(x) = stack.pop() {
        if let Some((name, Some(_))) = attribute(x, src)
            && event_of(&name).is_some()
        {
            return true;
        }
        stack.extend(kids(x));
    }
    false
}

fn opening<'t>(e: Node<'t>) -> Option<Node<'t>> {
    match e.kind() {
        "jsx_element" => e.child_by_field_name("open_tag"),
        "jsx_self_closing_element" => Some(e),
        _ => None,
    }
}

impl React {
    fn is_react(&self, f: &SourceFile, callee: Node, name: &str) -> bool {
        is_imported(f, callee, REACT, name)
    }

    /// Whether the user can set off the prop `prop` given to the project component `comp`: a control in it attaches
    /// the prop to a user action, directly, through a function of its own, or through another component of the
    /// project. A prop that is not named but spread onto an element is not followed and counts as yes; one only
    /// called from code (an effect, the end of an animation), or never used, counts as no.
    fn user_prop<'w>(&self, b: &Builder<'w>, comp: At<'w>, prop: &str, seen: &mut HashSet<(NodeKey, String)>) -> bool {
        if seen.len() > 32 || !seen.insert((comp.key(), prop.to_string())) {
            return false;
        }
        let f = b.file(comp);
        let uses = uses_of(comp.node, prop, &f.text);
        if uses.is_empty() {
            return spreads(comp.node);
        }
        uses.into_iter().any(|u| self.used_by_user(b, comp, u, seen))
    }

    /// Whether one use of a prop inside a component is set off by the user (see `user_prop`).
    fn used_by_user<'w>(&self, b: &Builder<'w>, comp: At<'w>, u: Node<'w>, seen: &mut HashSet<(NodeKey, String)>) -> bool {
        let f = b.file(comp);
        let l = f.lang;
        let mut n = u;
        while let Some(p) = n.parent() {
            if p == comp.node {
                return false;
            }
            if p.kind() == "jsx_attribute" {
                let Some((name, _)) = attribute(p, &f.text) else { return false };
                let tag = p.parent().and_then(|el| el.child_by_field_name("name")).filter(|t| f.text_of(*t).starts_with(|c: char| c.is_ascii_uppercase()));
                return match tag.and_then(|t| b.project_decl(b.at(comp, t))) {
                    Some(inner) => self.user_prop(b, inner, &name, seen),
                    None => event_of(&name).is_some(),
                };
            }
            // inside a function of the component's own: where that function is used
            if l.is_function(p.kind())
                && let Some(fname) = (l.function_name)(p)
            {
                return self.user_prop(b, comp, f.text_of(fname), seen);
            }
            n = p;
        }
        false
    }

    /// The words directly inside an element: `<Button>Save</Button>` → "Save".
    fn element_text(&self, b: &Builder, e: At) -> String {
        let words: Vec<String> = kids(e.node).into_iter().filter(|c| c.kind() == "jsx_text").map(|c| b.text(b.at(e, c)).trim().to_string()).filter(|t| !t.is_empty()).collect();
        one_line(&words.join(" "), 32)
    }

    /// The words on a control, as a person reads them on it: the first written between tags inside it, at any depth,
    /// as text or as a string constant (`<Text>{labels.save}</Text>`); else an attribute that carries them, either one
    /// its component in the project writes between tags (`<Button label={…}>` whose code has `{label}` there) or one
    /// React and React Native read out for it (`aria-label`, `accessibilityLabel`, `title`). Words need a letter or a
    /// digit: a lone "·" is no name.
    fn control_words<'w>(&self, b: &Builder<'w>, element: At<'w>, opening: At<'w>) -> String {
        let mut stack = vec![element.node];
        while let Some(n) = stack.pop() {
            if n.kind() == "jsx_element" {
                let words: Vec<String> = kids(n)
                    .into_iter()
                    .filter_map(|c| match c.kind() {
                        "jsx_text" => Some(b.text(b.at(element, c)).trim().to_string()),
                        "jsx_expression" => c.named_child(0).and_then(|x| b.words_of(b.at(element, x))),
                        _ => None,
                    })
                    .filter(|t| t.chars().any(char::is_alphanumeric))
                    .collect();
                if !words.is_empty() {
                    return one_line(&words.join(" "), 32);
                }
            }
            stack.extend(kids(n).into_iter().rev());
        }
        let f = b.file(opening);
        let written = opening.node.child_by_field_name("name").map(|t| self.written_props(b, b.at(opening, t))).unwrap_or_default();
        for a in kids(opening.node) {
            let Some((name, _)) = attribute(a, &f.text) else { continue };
            if !written.contains(&name) && !["aria-label", "accessibilityLabel", "title"].contains(&name.as_str()) {
                continue;
            }
            let v = kids(a).get(1).map(|v| if v.kind() == "jsx_expression" { v.named_child(0).unwrap_or(*v) } else { *v });
            if let Some(words) = v.and_then(|v| b.words_of(b.at(opening, v))).filter(|t| t.chars().any(char::is_alphanumeric)) {
                return one_line(&words, 32);
            }
        }
        String::new()
    }

    /// The props a component of the project writes between tags (`<Text>{label}</Text>`, `{props.title}`).
    fn written_props<'w>(&self, b: &Builder<'w>, tag: At<'w>) -> Vec<String> {
        let Some(decl) = b.project_decl(tag).filter(|d| b.is_function(*d)) else { return vec![] };
        let f = b.file(decl);
        let mut out = vec![];
        let mut stack = vec![decl.node];
        while let Some(n) = stack.pop() {
            if n.kind() == "jsx_expression" && n.parent().is_some_and(|p| p.kind() == "jsx_element") {
                let x = n.named_child(0).map(|x| f.lang.unwrap(x));
                let name = x.and_then(|x| match f.lang.member_fields(x.kind()) {
                    Some((_, prop)) => x.child_by_field_name(prop),
                    None => Some(x).filter(|x| f.lang.is_name(x.kind())),
                });
                if let Some(name) = name.map(|n| f.text_of(n).to_string()).filter(|n| n != "children" && !out.contains(n)) {
                    out.push(name);
                }
            }
            stack.extend(kids(n).into_iter().rev());
        }
        out
    }

    fn label(&self, b: &Builder, e: At) -> String {
        let Some(open) = opening(e.node) else { return one_line(b.text(e), 40) };
        let Some(tag) = open.child_by_field_name("name") else { return one_line(b.text(e), 40) };
        let tag = b.text(b.at(e, tag));
        let text = if e.node.kind() == "jsx_element" { self.element_text(b, e) } else { String::new() };
        if text.is_empty() { format!("<{tag}>") } else { format!("<{tag}>{text}</{tag}>") }
    }

    /// `const [x, setX] = useState(…)`: for a name bound there, the state's name and whether it is the setter. A name
    /// given it from elsewhere (a hook's result, a shared value) is followed there.
    fn state_of<'w>(&self, b: &Builder<'w>, id: At<'w>) -> Option<(String, bool)> {
        let loc = b.w.res.definition(id.file, id.node).into_iter().next()?;
        let file = b.w.p.file_index(&loc.file)?;
        let f = &b.w.p.files[file];
        let n = f.node_at(loc.line as usize, loc.col)?;
        self.state_at(b, At { file, node: n }).or_else(|| match b.w.res.target(id.file, id.node, b.w.adapters) {
            crate::resolve::Target::Declared(at) => self.state_at(b, at),
            _ => None,
        })
    }

    fn state_at<'w>(&self, b: &Builder<'w>, at: At<'w>) -> Option<(String, bool)> {
        let f = b.file(at);
        let n = at.node;
        let pattern = n.parent().filter(|p| p.kind() == "array_pattern")?;
        let decl = pattern.parent().filter(|p| p.kind() == "variable_declarator")?;
        let init = f.lang.unwrap(decl.child_by_field_name("value")?);
        let Expr::Call { callee, .. } = (f.lang.expr)(init, &f.text) else { return None };
        if !self.is_react(f, callee, "useState") {
            return None;
        }
        let elems = kids(pattern);
        Some((f.text_of(*elems.first()?).to_string(), elems.iter().position(|x| *x == n) == Some(1)))
    }

    fn set_state<'w>(&self, b: &mut Builder<'w>, e: At<'w>, args: &[Node<'w>], state: &str, ctx: &Ctx) -> Item {
        let arg = args.first().map(|a| b.unwrap(b.at(e, *a)));
        let literal = arg.is_some_and(|a| matches!(a.node.kind(), "true" | "false" | "null" | "number" | "string" | "undefined"));
        ctx.set_known(state, if literal { b.text(arg.unwrap()).to_string() } else { "?".into() });
        Item::Own {
            id: b.id(),
            at: ctx.this.clone(),
            kind: SelfKind::State,
            label: one_line(b.text(e), 48),
            detail: Some(format!("changes the state \"{state}\", which renders the component again")),
            place: b.place(e),
        }
    }

    /// A component: state, the first render, effects, then the render that follows the state changes.
    pub fn component<'w>(&self, b: &mut Builder<'w>, decl: At<'w>, ctx: &Ctx, hook: bool) -> Vec<Item> {
        let l = b.lang(decl);
        let f = b.file(decl);
        let Some(body) = (l.function_body)(decl.node).filter(|x| l.blocks.contains(&x.kind())) else { return vec![] };
        let adapters = b.w.adapters;
        for a in &adapters.list {
            a.check(b, decl);
        }
        let mut states: Vec<(Node, String, String)> = vec![];
        let mut effects: Vec<(At, Later)> = vec![];
        let mut rest = vec![];
        for st in (l.body)(body) {
            if let Stmt::Decl { values, .. } = (l.stmt)(st, &f.text) {
                let d = kids(st).into_iter().find(|x| x.kind() == "variable_declarator");
                let pattern = d.and_then(|d| d.child_by_field_name("name")).filter(|n| n.kind() == "array_pattern");
                if let (Some(pattern), Some(v)) = (pattern, values.first()) {
                    if let Expr::Call { callee, args } = (l.expr)(l.unwrap(*v), &f.text) {
                        if self.is_react(f, callee, "useState") {
                            let name = kids(pattern).first().map(|x| f.text_of(*x).to_string()).unwrap_or_default();
                            let init = args.first().map(|a| one_line(f.text_of(*a), 0)).unwrap_or("undefined".into());
                            states.push((st, name, init));
                            continue;
                        }
                    }
                }
            }
            // code registered to run later, by a call on its own or one whose result is kept (`const { data } = …`)
            let registers = match (l.stmt)(st, &f.text) {
                Stmt::Expr(e) => Some(e),
                Stmt::Decl { values, .. } if values.len() == 1 => Some(values[0]),
                _ => None,
            };
            if let Some(e) = registers {
                let call = b.unwrap(b.at(decl, e));
                if let Some(later) = adapters.list.iter().find_map(|a| a.later(b, call)) {
                    effects.push((call, later));
                    continue;
                }
            }
            rest.push(st);
        }
        for (_, name, init) in &states {
            ctx.set_known(name, init.clone());
        }
        let mut out = vec![];
        if !states.is_empty() {
            let label = format!("useState: {}", states.iter().map(|(_, n, i)| format!("{n} = {i}")).collect::<Vec<_>>().join(", "));
            let (first, last) = (b.at(decl, states[0].0), b.at(decl, states[states.len() - 1].0));
            out.push(Item::Own { id: b.id(), at: ctx.this.clone(), kind: SelfKind::State, label: one_line(&label, 56), detail: None, place: b.span(first, last) });
        }
        let first = self.render(b, decl.file, &rest, ctx, 1, hook);
        out.extend(first);
        let before = ctx.known.borrow().clone();
        for (call, later) in effects {
            let inner = Ctx { return_to: None, ..ctx.clone() };
            let bound = b.w.res.bindings();
            let given: Vec<Node> = later.given.iter().map(|g| g.node).collect();
            if let Some(first) = later.given.first() {
                b.bind_args(later.func, first.file, &given);
            }
            let items = b.callback(later.func, &inner);
            b.w.res.unbind(bound);
            // effects that touch nothing outside the screen are not part of the story
            if items.is_empty() || boring(&items) {
                continue;
            }
            let label = format!("{}(…{})", later.name, later.deps.as_ref().map(|d| format!(", {d}")).unwrap_or_default());
            out.push(Item::Frame { id: b.id(), kind: FrameKind::Effect, label, note: Some(later.when), at: b.place(call), items, els: None });
        }
        if *ctx.known.borrow() != before {
            let mut seen = std::collections::HashSet::new();
            fn mark(list: &[Item], seen: &mut std::collections::HashSet<String>) {
                for i in list {
                    let r = i.place();
                    let t = match i {
                        Item::Call(_) => "call",
                        Item::Return { .. } => "return",
                        Item::Own { .. } => "self",
                        Item::Frame { .. } => "frame",
                    };
                    seen.insert(format!("{t}:{}:{}:{}", r.file, r.from, r.to));
                    match i {
                        Item::Frame { items, els, .. } => {
                            mark(items, seen);
                            mark(els.as_deref().unwrap_or(&[]), seen);
                        }
                        Item::Call(c) => mark(c.body.as_deref().unwrap_or(&[]), seen),
                        _ => {}
                    }
                }
            }
            mark(&out, &mut seen);
            let again: Vec<Item> = self
                .render(b, decl.file, &rest, ctx, 2, hook)
                .into_iter()
                .filter(|i| {
                    let r = i.place();
                    let t = match i {
                        Item::Call(_) => "call",
                        Item::Return { .. } => "return",
                        Item::Own { .. } => "self",
                        Item::Frame { .. } => "frame",
                    };
                    !seen.contains(&format!("{t}:{}:{}:{}", r.file, r.from, r.to))
                })
                .collect();
            if again.iter().any(|i| !matches!(i, Item::Own { .. })) {
                out.extend(again);
            }
        }
        out
    }

    /// One render pass. Conditions on values known as code text are decided; the rest become alt frames.
    fn render<'w>(&self, b: &mut Builder<'w>, file: usize, list: &[Node<'w>], ctx: &Ctx, pass: u8, hook: bool) -> Vec<Item> {
        let mut out = vec![];
        let mut decided: Vec<String> = vec![];
        self.render_walk(b, file, list, ctx, pass, hook, &mut out, &mut decided);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render_walk<'w>(&self, b: &mut Builder<'w>, file: usize, list: &[Node<'w>], ctx: &Ctx, pass: u8, hook: bool, out: &mut Vec<Item>, decided: &mut Vec<String>) -> bool {
        let l = b.w.p.files[file].lang;
        let src = &b.w.p.files[file].text;
        // plain statements in a row are walked together, so that they are one step here as anywhere else
        let mut run: Vec<Node<'w>> = vec![];
        for &st in list {
            let at = At { file, node: st };
            let stmt = (l.stmt)(st, src);
            if matches!(stmt, Stmt::If { .. } | Stmt::Return(_)) && !run.is_empty() {
                out.extend(b.stmts(file, &std::mem::take(&mut run), ctx));
            }
            match stmt {
                Stmt::If { cond, then, els } => {
                    let cond = At { file, node: cond };
                    match b.decide(cond, ctx) {
                        None => {
                            let keep = out.len();
                            self.render_walk(b, file, &then, ctx, pass, hook, out, decided);
                            let then_items = out.split_off(keep);
                            let mut els_items = vec![];
                            if !els.is_empty() {
                                self.render_walk(b, file, &els, ctx, pass, hook, out, decided);
                                els_items = out.split_off(keep);
                            }
                            out.push(Item::Frame { id: b.id(), kind: FrameKind::Alt, label: b.cond_text(cond), note: None, at: b.place(at), items: then_items, els: if els_items.is_empty() { None } else { Some(els_items) } });
                        }
                        Some(v) => {
                            decided.push(format!("{} is {v}", b.cond_text(cond)));
                            let branch = if v { then } else { els };
                            if !branch.is_empty() && self.render_walk(b, file, &branch, ctx, pass, hook, out, decided) {
                                return true;
                            }
                        }
                    }
                }
                Stmt::Return(e) => {
                    if hook {
                        return true;
                    }
                    let e = e.map(|e| b.unwrap(At { file, node: e }));
                    let label = e.map(|e| self.label(b, e)).unwrap_or("nothing".into());
                    let detail = format!("render {pass}{}", if decided.is_empty() { String::new() } else { format!(" · {}", decided.join(", ")) });
                    out.push(Item::Own { id: b.id(), at: ctx.this.clone(), kind: SelfKind::Render, label, detail: Some(detail), place: b.place(at) });
                    if let Some(e) = e {
                        let quiet = Ctx { quiet: true, ..ctx.clone() };
                        let items = b.expr(e, &quiet);
                        out.extend(items);
                    }
                    return true;
                }
                _ => {
                    if pass == 1 {
                        run.push(st);
                    }
                }
            }
        }
        if !run.is_empty() {
            out.extend(b.stmts(file, &run, ctx));
        }
        false
    }

    fn jsx<'w>(&self, b: &mut Builder<'w>, e: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let mut out = vec![];
        let f = b.file(e);
        if let Some(open) = opening(e.node) {
            if let Some(tag) = open.child_by_field_name("name") {
                let tag_at = b.at(e, tag);
                let tag_text = b.text(tag_at);
                let decl = if tag_text.starts_with(|c: char| c.is_ascii_uppercase()) { b.project_decl(tag_at) } else { None };
                if decl.is_none() {
                    let adapters = b.w.adapters;
                    if let Some(step) = adapters.list.iter().find_map(|a| a.element(b, e, tag_at, ctx)) {
                        out.push(step);
                    }
                }
                let attrs: Vec<Node> = kids(open).into_iter().filter(|a| a.kind() == "jsx_attribute" || a.kind() == "jsx_expression").collect();
                for a in &attrs {
                    let Some((name, Some(value))) = attribute(*a, &f.text) else { continue };
                    // a user action is a story of its own
                    if (name.starts_with("on") && name[2..].starts_with(|c: char| c.is_ascii_uppercase())) || name == "action" {
                        continue;
                    }
                    let value = b.at(e, value);
                    if decl.is_none() && tag_text == "img" && name == "src" && b.url_text(value).is_none() {
                        let text = format!("<img src={{{}}}>: the browser loads a picture from an address only known at runtime", one_line(b.text(value), 0));
                        let at = b.place(b.at(e, *a));
                        b.gap(text, at);
                    }
                    if decl.is_none() {
                        let items = b.expr(value, ctx);
                        out.extend(items);
                    }
                }
                if let Some(decl) = decl {
                    let name = b.decl_name(decl);
                    let sub = b.file(decl).rel.clone();
                    let added = !b.has_part(&b.pid(decl));
                    let to = b.part(Participant { id: b.pid(decl), name: name.clone(), kind: PartKind::Component, group: ctx.group, sub: Some(sub), at: Some(b.place(decl)) });
                    // `key` is React bookkeeping, not something the component receives
                    let props: Vec<&str> = attrs.iter().filter(|a| attribute(**a, &f.text).is_none_or(|(n, _)| n != "key")).map(|a| f.text_of(*a)).collect();
                    let close = if open.kind() == "jsx_self_closing_element" { " />" } else { ">" };
                    let label = format!("<{tag_text}{}{close}", if props.is_empty() { String::new() } else { format!(" {}", props.join(" ")) });
                    let mut call = Call {
                        id: b.id(),
                        from: ctx.this.clone(),
                        to: to.clone(),
                        label: one_line(&label, 40),
                        detail: Some(one_line(b.text(e), 0)),
                        link: b.resolved_link(e, decl, "JSX element", &[]),
                        effect: None,
                        at: b.place(e),
                        target: Some(b.place(decl)),
                        body: None,
                        note: None,
                        opens: None,
                        story: None,
                        table: None,
                        touches: None,
                        unused: false,
                        into: None,
                        ends: vec![],
                        changes: vec![],
                    };
                    b.reached(decl, e, ctx, Some(&call.id));
                    b.drew(decl, e, ctx, Some(&call.id));
                    // a screen whose opening is a story of its own is a reference to it, told there once
                    if let Some(route) = b.page_at(decl).filter(|r| b.told(r)) {
                        call.opens = Some(route.path.clone());
                        call.unused = true;
                    } else if !ctx.stack.contains(&decl.key()) {
                        let mut stack = ctx.stack.clone();
                        stack.push(decl.key());
                        let inner = Ctx { this: to.clone(), group: ctx.group, return_to: None, endpoint: false, known: Rc::default(), stack, lane: false, in_loop: false, quiet: false, catching: None };
                        let id = call.id.clone();
                        if let Some(items) = b.inside(&id, |b| b.trial(|b| self.component(b, decl, &inner, false))) {
                            b.walked.push((name, decl));
                            call.touches = Some(crate::story::touched(&items));
                            call.body = Some(items);
                        }
                    }
                    let inside = self.children(b, e, ctx);
                    if call.body.is_none() && call.opens.is_none() && !ctx.in_loop {
                        if added {
                            b.drop_part(&to);
                        }
                    } else {
                        out.push(Item::Call(call));
                    }
                    out.extend(inside);
                    return out;
                }
            }
        }
        out.extend(self.children(b, e, ctx));
        out
    }

    /// What is written between an element's tags: drawn too, a component's `children` included, by the code that
    /// writes them.
    fn children<'w>(&self, b: &mut Builder<'w>, e: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let mut out = vec![];
        if e.node.kind() == "jsx_element" {
            for c in kids(e.node) {
                match c.kind() {
                    "jsx_opening_element" | "jsx_closing_element" | "jsx_text" => {}
                    "jsx_expression" => {
                        if let Some(x) = c.named_child(0) {
                            let items = b.expr(b.at(e, x), ctx);
                            out.extend(items);
                        }
                    }
                    _ => {
                        let items = b.expr(b.at(e, c), ctx);
                        out.extend(items);
                    }
                }
            }
        }
        out
    }
}

impl Adapter for React {
    fn name(&self) -> &'static str {
        "React"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "react"
    }

    fn triggers<'w>(&self, b: &mut Builder<'w>, decl: At<'w>) -> Vec<TriggerSite<'w>> {
        let f = b.file(decl);
        let mut out = vec![];
        let mut stack = vec![decl.node];
        let mut found = vec![];
        while let Some(n) = stack.pop() {
            if n.kind() == "jsx_attribute" {
                found.push(n);
            }
            let k = kids(n);
            stack.extend(k.into_iter().rev());
        }
        for a in found {
            let Some((name, Some(value))) = attribute(a, &f.text) else { continue };
            let Some(el) = a.parent() else { continue };
            let Some(event) = event_of(&name) else { continue };
            if PHASES.contains(&event.as_str()) && handles_whole(el, &f.text) {
                continue;
            }
            let mut v = b.unwrap(b.at(decl, value));
            // a handler a call makes from a function it is given (`handleSubmit(save)`): the function given, when an
            // adapter says so
            while let Some(inner) = b.w.adapters.wrapped(f, v.node) {
                v = b.unwrap(b.at(v, inner));
            }
            let named = f.lang.is_name(v.node.kind());
            // where a handler a project function returned was made (`save = guard(check, f)`)
            let handlers: Vec<(At, Option<At>)> = if b.is_function(v) {
                vec![(v, None)]
            } else if named {
                // a name made by a framework from a choice (`useActionState(ok ? save : send)`) is each of them
                match b.project_target_via(v) {
                    Some(h) => vec![h],
                    None => b.w.res.choices(v.file, v.node, b.w.adapters),
                }
            } else {
                vec![]
            };
            if handlers.is_empty() {
                continue;
            }
            // a prop given to a component of the project runs when that component calls it
            let tag = el.child_by_field_name("name").filter(|t| f.text_of(*t).starts_with(|c: char| c.is_ascii_uppercase()));
            if let Some(comp) = tag.and_then(|t| b.project_decl(b.at(decl, t)))
                && !self.user_prop(b, comp, &name, &mut HashSet::new())
            {
                continue;
            }
            let holder = if el.kind() == "jsx_opening_element" { el.parent().unwrap_or(el) } else { el };
            // a form is sent by its control of type "submit", whatever its tag, else by its first `<button>`
            let button = if event == "submit" {
                let mut st = vec![holder];
                let (mut submit, mut first) = (None, None);
                while let Some(x) = st.pop() {
                    if let Some(open) = x.child_by_field_name("open_tag").filter(|_| x.kind() == "jsx_element") {
                        let typed = kids(open).into_iter().any(|a| attribute(a, &f.text).is_some_and(|(n, _)| n == "type") && kids(a).get(1).is_some_and(|v| f.text_of(*v).trim_matches(['"', '\'']) == "submit"));
                        if typed {
                            submit = Some(x);
                            break;
                        }
                        if first.is_none() && open.child_by_field_name("name").is_some_and(|t| f.text_of(t) == "button") {
                            first = Some(x);
                        }
                    }
                    st.extend(kids(x).into_iter().rev());
                }
                submit.or(first)
            } else {
                Some(holder)
            };
            let text = match button.filter(|x| x.kind() == "jsx_element") {
                Some(x) => self.control_words(b, b.at(decl, x), b.at(decl, if x == holder { el } else { x.child_by_field_name("open_tag").unwrap_or(x) })),
                None => self.control_words(b, b.at(decl, el), b.at(decl, el)),
            };
            let tag = el.child_by_field_name("name").map(|t| f.text_of(t).to_string()).unwrap_or_default();
            let event_name = if name.starts_with("on") { format!("{}{}", name[2..3].to_lowercase(), &name[3..]) } else { event.clone() };
            for (handler, via) in handlers {
                out.push(TriggerSite {
                    via,
                    attr: b.at(decl, a),
                    holder: b.at(decl, el),
                    event: event.clone(),
                    event_name: event_name.clone(),
                    element: format!("<{tag} {}>", one_line(f.text_of(a), 0)),
                    control: Some(tag.clone()),
                    text: text.clone(),
                    handler,
                    named_handler: named,
                });
            }
        }
        out
    }

    fn controls<'w>(&self, w: &World<'w>) -> Vec<Route<'w>> {
        // what it draws runs in a browser, unless an adapter says where else
        let group = w.adapters.list.iter().find_map(|a| a.draws_on()).unwrap_or(Group::Browser);
        let screens: HashSet<NodeKey> = w.routes.iter().map(|r| r.decl.key()).collect();
        let mut found: Vec<(At<'w>, String)> = vec![];
        for (i, f) in w.p.files.iter().enumerate().filter(|(_, f)| f.lang.server == "typescript") {
            let l = f.lang;
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                // the outermost function or class: what is written inside it is part of it
                if l.is_function(n.kind()) || l.reach.classes.contains(&n.kind()) {
                    let at = At { file: i, node: n };
                    if !screens.contains(&at.key()) && has_action(n, &f.text) {
                        let stem = || std::path::Path::new(&f.rel).file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        found.push((at, (l.function_name)(n).map(|x| f.text_of(x).to_string()).unwrap_or_else(stem)));
                    }
                    continue;
                }
                stack.extend(kids(n));
            }
        }
        let mut count: HashMap<&str, usize> = HashMap::new();
        for (_, n) in &found {
            *count.entry(n).or_default() += 1;
        }
        found
            .iter()
            .map(|(at, n)| {
                // a component is known by its name; two of one name, by their files too
                let file = w.p.files[at.file].rel.clone();
                let path = if count[n.as_str()] > 1 { format!("{n} · {file}") } else { n.clone() };
                Route { kind: RouteKind::Page, segs: vec![Seg::Literal(path.clone())], path, method: None, file, decl: *at, group, how: "React component", name: Some(n.clone()), screen: false }
            })
            .collect()
    }

    fn only_named(&self, file: &SourceFile, decl: Node) -> bool {
        // a component: a function named with a capital that makes JSX
        let named = (file.lang.function_name)(decl).is_some_and(|n| file.text_of(n).starts_with(|c: char| c.is_ascii_uppercase()));
        let mut stack = vec![decl];
        while let Some(x) = stack.pop() {
            if matches!(x.kind(), "jsx_element" | "jsx_self_closing_element" | "jsx_fragment") {
                return named;
            }
            stack.extend(kids(x));
        }
        false
    }

    fn walk_function<'w>(&self, b: &mut Builder<'w>, decl: At<'w>, ctx: &Ctx, entry: bool) -> Option<Vec<Item>> {
        let l = b.lang(decl);
        let block = (l.function_body)(decl.node).is_some_and(|x| l.blocks.contains(&x.kind()));
        // an endpoint answers a request: it renders nothing
        if entry && !ctx.endpoint {
            return Some(self.component(b, decl, ctx, false));
        }
        if block && hook_like(&b.decl_name(decl)) {
            return Some(self.component(b, decl, ctx, true));
        }
        None
    }

    fn expr<'w>(&self, b: &mut Builder<'w>, e: At<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        match e.node.kind() {
            "jsx_element" | "jsx_self_closing_element" => Some(self.jsx(b, e, ctx)),
            _ => None,
        }
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        if !chain.segs.is_empty() {
            return None;
        }
        let f = b.file(e);
        let Expr::Call { callee, args } = (f.lang.expr)(f.lang.unwrap(e.node), &f.text) else { return None };
        let callee = f.lang.unwrap(callee);
        if !f.lang.is_name(callee.kind()) {
            return None;
        }
        let (state, setter) = self.state_of(b, b.at(e, callee))?;
        if !setter {
            return None;
        }
        Some(vec![self.set_state(b, e, &args, &state, ctx)])
    }

    fn same_lane<'w>(&self, b: &Builder<'w>, decl: At<'w>, ctx: &Ctx) -> bool {
        // a hook, or a function made inside one, runs inside the component that uses it
        if ctx.lane || hook_like(&b.decl_name(decl)) {
            return true;
        }
        let l = b.lang(decl);
        let mut n = decl.node.parent();
        while let Some(p) = n {
            if l.is_function(p.kind()) && (l.function_name)(p).is_some_and(|name| hook_like(b.text(b.at(decl, name)))) {
                return true;
            }
            n = p.parent();
        }
        false
    }

    fn silent<'w>(&self, b: &Builder<'w>, decl: At<'w>) -> bool {
        hook_like(&b.decl_name(decl))
    }

    fn later<'w>(&self, b: &Builder<'w>, call: At<'w>) -> Option<Later<'w>> {
        let f = b.file(call);
        let Expr::Call { callee, args } = (f.lang.expr)(call.node, &f.text) else { return None };
        let name = ["useEffect", "useLayoutEffect"].into_iter().find(|n| self.is_react(f, callee, n))?;
        let func = b.unwrap(b.at(call, *args.first()?));
        if !b.is_function(func) {
            return None;
        }
        let deps = args.get(1).map(|d| one_line(f.text_of(*d), 0));
        let when = match deps.as_deref() {
            Some("[]") => "after first render".to_string(),
            Some(d) => format!("after first render, again when {d} changes"),
            None => "after every render".to_string(),
        };
        Some(Later { name: name.to_string(), func, deps, when, given: vec![] })
    }

    fn wrapped<'p>(&self, file: &'p SourceFile, call: Node<'p>) -> Option<Node<'p>> {
        let Expr::Call { callee, args } = (file.lang.expr)(call, &file.text) else { return None };
        // a memoized callback, and a component wrapped to take a ref or to skip renders: the function given runs
        if ["useCallback", "forwardRef", "memo"].iter().any(|n| self.is_react(file, callee, n)) { args.first().copied() } else { None }
    }

    fn redirect<'p>(&self, res: &Resolver<'p>, file: usize, callee: Node<'p>) -> Option<(At<'p>, &'static str)> {
        // `ref.current(…)` where `ref = useRef(f)`: the function the ref was made with
        let f = &res.project.files[file];
        let l = f.lang;
        let c = l.unwrap(callee);
        let (obj, prop) = l.member_fields(c.kind())?;
        if f.text_of(c.child_by_field_name(prop)?) != "current" {
            return None;
        }
        let o = c.child_by_field_name(obj)?;
        if !l.is_name(o.kind()) {
            return None;
        }
        let loc = res.definition(file, o).into_iter().next()?;
        let df = res.project.file_index(&loc.file)?;
        let dfile = &res.project.files[df];
        let n = dfile.node_at(loc.line as usize, loc.col)?;
        let decl = n.parent().filter(|p| p.kind() == "variable_declarator")?;
        let init = dfile.lang.unwrap(decl.child_by_field_name("value")?);
        let Expr::Call { callee, args } = (dfile.lang.expr)(init, &dfile.text) else { return None };
        if !self.is_react(dfile, callee, "useRef") {
            return None;
        }
        Some((At { file: df, node: *args.first()? }, "the function useRef was made with"))
    }

    fn gives_at<'p>(&self, res: &Resolver<'p>, file: usize, call: Node<'p>, place: usize) -> Option<(Vec<At<'p>>, &'static str)> {
        // `[state, dispatch] = useReducer(reducer, init)`, `[state, formAction, pending] = useActionState(action, init)`:
        // what the second one runs is the function given first
        let f = &res.project.files[file];
        let Expr::Call { callee, args } = (f.lang.expr)(call, &f.text) else { return None };
        let given = self.is_react(f, callee, "useReducer") || self.is_react(f, callee, "useActionState") || is_imported(f, callee, &|m| m == "react-dom", "useFormState");
        if place != 1 || !given {
            return None;
        }
        Some((vec![At { file, node: f.lang.unwrap(*args.first()?) }], "the function it runs, given first"))
    }

    fn gives<'p>(&self, res: &Resolver<'p>, file: usize, call: Node<'p>) -> Option<(Vec<At<'p>>, &'static str)> {
        let f = &res.project.files[file];
        let Expr::Call { callee, args } = (f.lang.expr)(call, &f.text) else { return None };
        // `useMemo(() => value, deps)`: what the function returns
        if self.is_react(f, callee, "useMemo") {
            let func = f.lang.unwrap(*args.first()?);
            let values = res.returned_objects(At { file, node: func }).into_iter().map(|(file, node)| At { file, node }).collect();
            return Some((values, "what useMemo's function returns"));
        }
        // `useContext(Shared)`, `use(Shared)`: the value given where it is provided, `<Shared.Provider value={…}>` or
        // `<Shared value={…}>`, anywhere in the project
        if !self.is_react(f, callee, "useContext") && !self.is_react(f, callee, "use") {
            return None;
        }
        let shared = f.lang.unwrap(*args.first()?);
        if !f.lang.is_name(shared.kind()) {
            return None;
        }
        let name = f.text_of(shared);
        let made = res.definition(file, shared).into_iter().next()?;
        let mut out = vec![];
        for (i, pf) in res.project.files.iter().enumerate().filter(|(_, pf)| pf.lang.server == "typescript" && pf.text.contains(name)) {
            let mut stack = vec![pf.tree.root_node()];
            while let Some(n) = stack.pop() {
                stack.extend(kids(n));
                if !matches!(n.kind(), "jsx_opening_element" | "jsx_self_closing_element") {
                    continue;
                }
                let Some(tag) = n.child_by_field_name("name") else { continue };
                let at = match tag.kind() {
                    "member_expression" if tag.child_by_field_name("property").is_some_and(|p| pf.text_of(p) == "Provider") => tag.child_by_field_name("object"),
                    "identifier" => Some(tag),
                    _ => None,
                };
                let Some(at) = at.filter(|x| pf.text_of(*x) == name) else { continue };
                if res.definition(i, at).into_iter().next().as_ref() != Some(&made) {
                    continue;
                }
                for a in kids(n) {
                    if let Some((_, Some(v))) = attribute(a, &pf.text).filter(|(attr, _)| attr == "value") {
                        out.push(At { file: i, node: v });
                    }
                }
            }
        }
        out.sort_by_key(|a| (a.file, a.node.start_byte()));
        Some((out, "the value given where it is provided"))
    }
}
