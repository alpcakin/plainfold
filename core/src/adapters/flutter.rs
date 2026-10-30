// Flutter adapter. Handles: widgets, the places of a Flutter app. A widget is a class of the project that draws: it
// has a `build(BuildContext …)` method, or it is the `State` a widget makes in `createState()`, which draws for it.
// Each one is a place of its own, named by its widget: what runs when it is first shown (`initState`,
// `didChangeDependencies`), what it builds, what runs when it goes away (`dispose`), and the user actions written in
// it (an argument named `on…` given a function: `onPressed: save`, `onTap: () => …`, also on a widget of the
// project). A widget of the project made while another one builds is drawn as that one rendering it. `setState(…)`
// changes the widget's state and runs its function at once. A function handed to Flutter that it runs later (a
// callback after the frame is drawn, a builder) is a frame of its own, shown when it does something; a builder makes
// what is drawn, so it is walked quietly, as `build` is. `Navigator.push…`
// and `show…` (dialogs, sheets) given a builder that makes a widget of the project open that widget. A widget whose
// opening is a story of its own is a reference to that story wherever it is rendered or opened. The `flutter`
// package is the platform the app runs on, as is `dart:ui`, which Flutter ships as `sky_engine`: their calls are not
// drawn as calls into a package. The function that calls `runApp(…)` (an app's `main`) is what runs when the app
// starts, on the device.
use super::{Adapter, Start, TriggerSite};
use crate::lang::{Expr, Stmt};
use crate::model::*;
use crate::resolve::Resolver;
use crate::source::{At, Project, SourceFile, one_line};
use crate::story::routes::{Route, RouteKind, Seg};
use crate::story::{Builder, Chain, Ctx, World, boring};
use std::collections::HashMap;
use std::rc::Rc;
use tree_sitter::Node;

pub struct Flutter;

/// Flutter's widgets that scroll a list: a control that wraps one (`RefreshIndicator`) is a gesture on the list, and
/// the words in the list are not its own.
const SCROLLS: &[&str] = &["ListView", "GridView", "CustomScrollView", "SingleChildScrollView", "PageView", "NestedScrollView", "ReorderableListView", "ListWheelScrollView"];
/// Methods Flutter calls on a widget or its `State` by itself.
const LIFECYCLE: &[&str] = &["createState", "initState", "didChangeDependencies", "didUpdateWidget", "build", "activate", "deactivate", "dispose", "reassemble"];
/// What runs when a widget is first shown, in the order Flutter runs it, and when.
const FIRST: &[(&str, &str)] = &[("initState", "when the widget is first shown, before it builds"), ("didChangeDependencies", "after initState, and again when something it reads from above it changes")];

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).filter(|k| k.kind() != "comment").collect()
}

/// `onPressed`, `onTap`, `onFieldSubmitted`: an argument that gives the code to run when the user does something.
fn event(label: &str) -> Option<String> {
    let rest = label.strip_prefix("on")?;
    rest.starts_with(|c: char| c.is_ascii_uppercase()).then(|| rest.to_string())
}

/// A function given by name to Flutter that it runs when something happens (`confirmDismiss`, `validator`), not
/// while it builds: anything but a builder (`builder`, `itemBuilder`).
fn callback(label: &str) -> bool {
    !label.to_lowercase().ends_with("builder")
}

/// A method a class writes itself, with a body.
fn method<'t>(f: &SourceFile, class: Node<'t>, name: &str) -> Option<Node<'t>> {
    let l = f.lang;
    (l.reach.members)(class).into_iter().filter(|m| f.text_of(*m) == name).find_map(|m| l.function_of(m)).filter(|m| (l.function_body)(*m).is_some())
}

/// The class draws: it has a `build` method whose first parameter is Flutter's `BuildContext`.
fn draws(f: &SourceFile, class: Node) -> bool {
    let Some(build) = method(f, class, "build") else { return false };
    // method → its head → the function's head → its parameters → the first one's type
    let mut stack = vec![build];
    while let Some(n) = stack.pop() {
        if n.kind() == "formal_parameter_list" {
            return kids(n).into_iter().find(|p| p.kind() == "formal_parameter").and_then(|p| kids(p).into_iter().find(|t| t.kind() == "type")).is_some_and(|t| f.text_of(t) == "BuildContext");
        }
        if n == build || n.kind().ends_with("signature") {
            stack.extend(kids(n));
        }
    }
    false
}

/// The class a definition location is written in.
fn class_at<'p>(p: &'p Project, loc: &crate::lsp::Loc) -> Option<At<'p>> {
    let i = p.file_index(&loc.file)?;
    let f = &p.files[i];
    let mut n = f.node_at(loc.line as usize, loc.col);
    while let Some(x) = n {
        if f.lang.reach.classes.contains(&x.kind()) {
            return Some(At { file: i, node: x });
        }
        n = x.parent();
    }
    None
}

/// The class a call makes, when it makes an instance of a class of the project (`LoginScreen()`, `const Card(…)`).
fn made<'p>(res: &Resolver<'p>, file: usize, call: Node<'p>) -> Option<At<'p>> {
    let p = res.project;
    let f = &p.files[file];
    let l = f.lang;
    let Expr::Call { callee, .. } = (l.expr)(call, &f.text) else { return None };
    let name = crate::resolve::callee_name(callee, l)?;
    let loc = res.definition(file, name).into_iter().next().filter(|loc| p.is_own(&loc.file))?;
    let class = class_at(p, &loc)?;
    // the name a call reaches is the class's own, or that of one of its constructors
    let cf = &p.files[class.file];
    let at = cf.node_at(loc.line as usize, loc.col);
    let named = at.is_some() && class.node.child_by_field_name("name") == at;
    let constructor = at.is_some_and(|n| {
        let mut x = n.parent();
        while let Some(y) = x.filter(|y| y.kind() != "class_body") {
            if y.kind().contains("constructor_signature") {
                return true;
            }
            x = y.parent();
        }
        false
    });
    (named || constructor).then_some(class)
}

/// The `State` a widget makes in `createState()`.
fn state_of<'p>(res: &Resolver<'p>, widget: At<'p>) -> Option<At<'p>> {
    let f = &res.project.files[widget.file];
    let l = f.lang;
    let m = method(f, widget.node, "createState")?;
    let body = (l.function_body)(m)?;
    let value = if l.blocks.contains(&body.kind()) {
        (l.body)(body).into_iter().find_map(|st| match (l.stmt)(st, &f.text) {
            Stmt::Return(v) => v,
            _ => None,
        })?
    } else {
        body
    };
    made(res, widget.file, l.unwrap(value))
}

impl Flutter {
    /// The place a widget class draws in: its `State`, else itself, when it draws.
    fn place<'w>(&self, res: &Resolver<'w>, class: At<'w>) -> Option<At<'w>> {
        let f = &res.project.files[class.file];
        if draws(f, class.node) {
            return Some(class);
        }
        state_of(res, class).filter(|s| draws(&res.project.files[s.file], s.node))
    }

    /// A widget being shown: what runs when it is first shown, what it builds, and what runs when it goes away.
    fn widget<'w>(&self, b: &mut Builder<'w>, class: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let f = b.file(class);
        let mut out = vec![];
        for (name, when) in FIRST {
            let Some(m) = method(f, class.node, name) else { continue };
            let items = b.callback(b.at(class, m), &Ctx { return_to: None, ..ctx.clone() });
            b.walked.push((name.to_string(), b.at(class, m)));
            if !boring(&items) {
                out.push(Item::Frame { id: b.id(), kind: FrameKind::Effect, label: format!("{name}()"), note: Some(when.to_string()), at: b.place(b.at(class, m)), items, els: None });
            }
        }
        if let Some(m) = method(f, class.node, "build") {
            out.extend(self.build(b, b.at(class, m), ctx));
            b.walked.push(("build".into(), b.at(class, m)));
        }
        if let Some(m) = method(f, class.node, "dispose") {
            let items = b.callback(b.at(class, m), &Ctx { return_to: None, ..ctx.clone() });
            if !boring(&items) {
                out.push(Item::Frame { id: b.id(), kind: FrameKind::Effect, label: "dispose()".into(), note: Some("when the widget is removed for good".into()), at: b.place(b.at(class, m)), items, els: None });
            }
        }
        out
    }

    /// `build`: its statements as they are written, then the widget it returns. All of it is drawing, so it is walked
    /// quietly, as a render: plain helpers and packages that draw are not part of the story, only what crosses a
    /// boundary shows.
    fn build<'w>(&self, b: &mut Builder<'w>, m: At<'w>, ctx: &Ctx) -> Vec<Item> {
        let l = b.lang(m);
        let Some(body) = (l.function_body)(m.node) else { return vec![] };
        let quiet = Ctx { quiet: true, ..ctx.clone() };
        let mut out = vec![];
        if l.blocks.contains(&body.kind()) {
            self.render(b, m.file, &(l.body)(body), &quiet, &mut out);
        } else {
            self.returns(b, b.at(m, body), body, &quiet, &mut out);
        }
        out
    }

    /// Whether a statement returns from the function it is in, somewhere inside it.
    fn returns_in(&self, b: &Builder, file: usize, st: Node) -> bool {
        let l = b.w.p.files[file].lang;
        let mut stack = vec![st];
        while let Some(n) = stack.pop() {
            if n.kind() == l.follow.returns {
                return true;
            }
            if !l.is_function(n.kind()) {
                stack.extend(kids(n));
            }
        }
        false
    }

    /// The statements of a build, up to the widget it returns; a condition whose branches return is an alternative.
    fn render<'w>(&self, b: &mut Builder<'w>, file: usize, list: &[Node<'w>], ctx: &Ctx, out: &mut Vec<Item>) -> bool {
        let l = b.w.p.files[file].lang;
        let src = &b.w.p.files[file].text;
        let mut run: Vec<Node<'w>> = vec![];
        for &st in list {
            let at = At { file, node: st };
            match (l.stmt)(st, src) {
                // a condition that chooses what is built; one that only does something is a plain statement
                Stmt::If { cond, then, els } if self.returns_in(b, file, st) => {
                    out.extend(b.stmts(file, &std::mem::take(&mut run), ctx));
                    let cond = At { file, node: cond };
                    out.extend(b.expr(cond, ctx));
                    let keep = out.len();
                    let then_returns = self.render(b, file, &then, ctx, out);
                    let then_items = out.split_off(keep);
                    let els_returns = self.render(b, file, &els, ctx, out);
                    let els_items = out.split_off(keep);
                    if !then_items.is_empty() || !els_items.is_empty() {
                        out.push(Item::Frame { id: b.id(), kind: FrameKind::Alt, label: b.cond_text(cond), note: None, at: b.place(at), items: then_items, els: if els_items.is_empty() { None } else { Some(els_items) } });
                    }
                    if then_returns && els_returns {
                        return true;
                    }
                }
                Stmt::Return(e) => {
                    out.extend(b.stmts(file, &std::mem::take(&mut run), ctx));
                    if let Some(e) = e {
                        self.returns(b, at, e, ctx, out);
                    }
                    return true;
                }
                _ => run.push(st),
            }
        }
        out.extend(b.stmts(file, &run, ctx));
        false
    }

    /// The widget a build returns: one row for it, then what crosses a boundary while it is made.
    fn returns<'w>(&self, b: &mut Builder<'w>, at: At<'w>, e: Node<'w>, ctx: &Ctx, out: &mut Vec<Item>) {
        let e = b.unwrap(b.at(at, e));
        let f = b.file(e);
        let label = match (f.lang.expr)(e.node, &f.text) {
            Expr::Call { callee, .. } => format!("{}(…)", one_line(f.text_of(callee), 32)),
            _ => one_line(b.text(e), 40),
        };
        out.push(Item::Own { id: b.id(), at: ctx.this.clone(), kind: SelfKind::Render, label, detail: Some("build".into()), place: b.place(at) });
        out.extend(b.expr(e, ctx));
    }

    /// A widget of the project made here: the widget rendering it, with what it does when shown, when that crosses a
    /// boundary. None when it does not: its arguments are then walked as any others.
    fn render_widget<'w>(&self, b: &mut Builder<'w>, e: At<'w>, class: At<'w>, place: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        let name = b.decl_name(class);
        let sub = b.file(place).rel.clone();
        let added = !b.has_part(&b.pid(place));
        let to = b.part(Participant { id: b.pid(place), name: name.clone(), kind: PartKind::Component, group: ctx.group, sub: Some(sub), at: Some(b.place(place)) });
        let mut call = Call {
            id: b.id(),
            from: ctx.this.clone(),
            to: to.clone(),
            label: one_line(b.text(e), 40),
            detail: Some(one_line(b.text(e), 0)),
            link: b.resolved_link(e, class, "widget", &[]),
            effect: None,
            at: b.place(e),
            target: Some(b.place(place)),
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
        b.drew(place, e, ctx, Some(&call.id));
        // a widget whose opening is a story of its own is a reference to it, told there once
        if let Some(route) = b.page_at(place).filter(|r| b.told(r)) {
            call.opens = Some(route.path.clone());
            call.unused = true;
        } else if !ctx.stack.contains(&place.key()) {
            let mut stack = ctx.stack.clone();
            stack.push(place.key());
            let inner = Ctx { this: to.clone(), group: ctx.group, return_to: None, endpoint: false, known: Rc::default(), stack, lane: false, in_loop: ctx.in_loop, quiet: false, catching: None };
            let id = call.id.clone();
            if let Some(items) = b.inside(&id, |b| b.trial(|b| self.widget(b, place, &inner))) {
                call.touches = Some(crate::story::touched(&items));
                call.body = Some(items);
            }
        }
        if call.body.is_none() && call.opens.is_none() {
            if added {
                b.drop_part(&to);
            }
            return None;
        }
        b.reached(class, e, ctx, Some(&call.id));
        // what it is given is worked out first, in the widget building it; what the user does is a story of its own
        let mut out = vec![];
        for a in args {
            if (l.named_arg)(*a, &f.text).is_some_and(|(n, _)| event(f.text_of(n)).is_some()) {
                continue;
            }
            out.extend(b.expr(b.at(e, l.arg_value(*a, &f.text)), ctx));
        }
        out.push(Item::Call(call));
        Some(out)
    }

    /// The first widget of the project a function makes, for a call that shows what a builder makes.
    fn shown<'w>(&self, b: &Builder<'w>, file: usize, n: Node<'w>) -> Option<(At<'w>, At<'w>)> {
        let l = b.w.p.files[file].lang;
        let mut stack = vec![n];
        while let Some(x) = stack.pop() {
            if l.call_field(x.kind()).is_some() {
                if let Some(class) = made(b.w.res, file, x) {
                    if let Some(place) = self.place(b.w.res, class) {
                        return Some((class, place));
                    }
                }
            }
            stack.extend(kids(x).into_iter().rev());
        }
        None
    }

    /// `Navigator.push(context, MaterialPageRoute(builder: (_) => const Next()))`, `showDialog(builder: …)`: the call
    /// opens the widget of the project its builder makes.
    fn opens<'w>(&self, b: &mut Builder<'w>, e: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Option<Vec<Item>> {
        let l = b.lang(e);
        let src = &b.file(e).text;
        let mut funcs = vec![];
        for a in args {
            let mut stack = vec![l.arg_value(*a, src)];
            while let Some(x) = stack.pop() {
                if l.is_function(x.kind()) {
                    funcs.push(x);
                } else {
                    stack.extend(kids(x));
                }
            }
        }
        let (class, place) = funcs.into_iter().find_map(|func| self.shown(b, e.file, func))?;
        let route = b.w.routes.iter().find(|r| r.decl.key() == place.key()).cloned()?;
        let to = b.screen(&route);
        let name = b.decl_name(class);
        b.reached(class, e, ctx, None);
        Some(vec![Item::Call(Call {
            id: b.id(),
            from: ctx.this.clone(),
            to,
            label: one_line(b.text(e), 40),
            detail: Some(one_line(b.text(e), 0)),
            link: Link { kind: LinkKind::Exact, how: format!("Flutter: the route's builder makes {name} ({})", route.file) },
            effect: None,
            at: b.place(e),
            target: Some(b.place(place)),
            body: None,
            note: None,
            opens: Some(route.path.clone()),
            story: None,
            table: None,
            touches: None,
            unused: false,
            into: None,
            ends: vec![],
            changes: vec![],
        })])
    }

    /// `setState(() { … })`: the state changes, which builds the widget again; the function runs at once.
    fn set_state<'w>(&self, b: &mut Builder<'w>, e: At<'w>, args: &[Node<'w>], ctx: &Ctx) -> Vec<Item> {
        let l = b.lang(e);
        let mut out = vec![Item::Own {
            id: b.id(),
            at: ctx.this.clone(),
            kind: SelfKind::State,
            label: one_line(b.text(e), 48),
            detail: Some("changes the widget's state, which builds it again".into()),
            place: b.place(e),
        }];
        for a in args {
            let v = b.unwrap(b.at(e, *a));
            if l.is_function(v.node.kind()) {
                out.extend(b.callback(v, ctx).into_iter().filter(|i| !i.is_local()));
            }
        }
        out
    }

    /// A call into Flutter given functions, written there or the project's own named there: each one Flutter runs
    /// later (or while it builds) is a frame named by the call, shown when what it does is more than plain statements.
    /// What the user does is a story of its own.
    fn handed<'w>(&self, b: &mut Builder<'w>, e: At<'w>, callee: Node<'w>, args: &[Node<'w>], ctx: &Ctx) -> Option<Vec<Item>> {
        let l = b.lang(e);
        let src = &b.file(e).text;
        let given: Vec<(Option<String>, At<'w>, Option<At<'w>>)> = args
            .iter()
            .map(|a| {
                let named = (l.named_arg)(*a, src).map(|(n, _)| b.text(b.at(e, n)).to_string());
                let v = b.unwrap(b.at(e, l.arg_value(*a, src)));
                // what runs when something happens is a story of its own
                let func = if named.as_deref().is_some_and(callback) {
                    None
                } else if b.is_function(v) {
                    Some(v)
                } else if l.is_name(v.node.kind()) || l.member_fields(v.node.kind()).is_some() {
                    b.project_decl(v).filter(|d| b.is_function(*d))
                } else {
                    None
                };
                (named, v, func)
            })
            .collect();
        if given.iter().all(|(_, _, f)| f.is_none()) {
            return None;
        }
        let mut out = vec![];
        let name = one_line(b.text(b.at(e, callee)), 40);
        for (named, v, func) in given {
            let Some(func) = func else {
                out.extend(b.expr(v, ctx));
                continue;
            };
            // a builder makes what is drawn: walked quietly, as `build` is
            let builds = named.as_deref().is_some_and(|n| !callback(n));
            let inner = Ctx { return_to: None, quiet: ctx.quiet || builds, ..ctx.clone() };
            let items = if func == v { b.callback(func, &inner) } else { b.project_call(v, func, &[], &inner) };
            if !boring(&items) {
                let label = match &named {
                    Some(n) => format!("{name}({n}: …)"),
                    None => format!("{name}(…)"),
                };
                out.push(Item::Frame { id: b.id(), kind: FrameKind::Effect, label, note: Some(format!("when {name} runs it")), at: b.place(v), items, els: None });
            }
        }
        Some(out)
    }

    /// The words a person reads on a control: the first text it shows (Flutter's `Text` and `SelectableText`), as
    /// written or as a constant, at any depth inside what it is made of; else what it gives the project's own widget
    /// that shows it as text; else what it says to accessibility (`tooltip`, `semanticLabel`).
    fn words<'w>(&self, b: &Builder<'w>, holder: At<'w>, skip: Node<'w>) -> String {
        let f = b.file(holder);
        let l = f.lang;
        let found = |b: &Builder<'w>, n: Node<'w>| -> Option<String> {
            let Expr::Call { callee, args } = (l.expr)(n, &f.text) else { return None };
            let name = crate::resolve::callee_name(callee, l).map(|c| f.text_of(c))?;
            let shows = ["Text", "SelectableText"].contains(&name) || (name == "rich" && l.member_fields(l.unwrap(callee).kind()).is_some() && f.text_of(callee).starts_with("Text."));
            if !shows || b.project_decl(b.at(holder, callee)).is_some() {
                return None;
            }
            let first = args.into_iter().find(|a| (l.named_arg)(*a, &f.text).is_none())?;
            b.words_of(b.at(holder, first))
        };
        let scrolls = |n: Node| match (l.expr)(n, &f.text) {
            Expr::Call { callee, .. } => SCROLLS.contains(&f.text_of(callee).split(['.', '<']).next().unwrap_or("")),
            _ => false,
        };
        let mut stack: Vec<Node> = kids(holder.node).into_iter().rev().collect();
        while let Some(n) = stack.pop() {
            if n == skip || scrolls(n) {
                continue;
            }
            if let Some(w) = found(b, n).filter(|w| w.chars().any(char::is_alphanumeric)) {
                return one_line(&w, 32);
            }
            stack.extend(kids(n).into_iter().rev());
        }
        let Expr::Call { args, .. } = (l.expr)(holder.node, &f.text) else { return String::new() };
        let shown = made(b.w.res, holder.file, holder.node).and_then(|c| self.place(b.w.res, c)).map(|p| self.shown_as_text(b, p)).unwrap_or_default();
        for a in args {
            let Some((n, v)) = (l.named_arg)(a, &f.text) else { continue };
            let n = f.text_of(n);
            if !shown.iter().any(|s| s == n) && !["tooltip", "semanticLabel", "semanticsLabel"].contains(&n) {
                continue;
            }
            if let Some(w) = b.words_of(b.at(holder, v)).filter(|w| w.chars().any(char::is_alphanumeric)) {
                return one_line(&w, 32);
            }
        }
        String::new()
    }

    /// The fields a widget of the project shows as text (`Text(title)`, `Text(widget.title)`).
    fn shown_as_text<'w>(&self, b: &Builder<'w>, place: At<'w>) -> Vec<String> {
        let f = b.file(place);
        let l = f.lang;
        let mut out = vec![];
        let mut stack = vec![place.node];
        while let Some(n) = stack.pop() {
            if let Expr::Call { callee, args } = (l.expr)(n, &f.text) {
                if crate::resolve::callee_name(callee, l).is_some_and(|c| f.text_of(c) == "Text") {
                    if let Some(first) = args.into_iter().find(|a| (l.named_arg)(*a, &f.text).is_none()) {
                        let x = l.unwrap(first);
                        let name = match l.member_fields(x.kind()) {
                            Some((_, prop)) => x.child_by_field_name(prop),
                            None => Some(x).filter(|x| l.is_name(x.kind())),
                        };
                        out.extend(name.map(|n| f.text_of(n).to_string()));
                    }
                }
            }
            stack.extend(kids(n));
        }
        out
    }

    /// The function a user action runs: one written there, or one of the project's named there, also when it is given
    /// only while a condition holds (`loading ? null : submit`).
    fn handler<'w>(&self, b: &Builder<'w>, v: At<'w>) -> Option<(At<'w>, bool)> {
        let l = b.lang(v);
        let v = b.unwrap(v);
        if b.is_function(v) {
            return Some((v, false));
        }
        if l.is_name(v.node.kind()) || l.member_fields(v.node.kind()).is_some() {
            return b.project_decl(v).filter(|d| b.is_function(*d)).map(|d| (d, true));
        }
        if let Expr::Cond { then, els: Some(els), .. } = (l.expr)(v.node, &b.file(v).text) {
            let null = |n: Node| n.kind() == "null_literal";
            return match (null(then), null(els)) {
                (true, false) => self.handler(b, b.at(v, els)),
                (false, true) => self.handler(b, b.at(v, then)),
                _ => None,
            };
        }
        None
    }
}

impl Adapter for Flutter {
    fn name(&self) -> &'static str {
        "Flutter"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["dart"]
    }
    fn handles(&self, package: &str) -> bool {
        package == "flutter" || package == "sky_engine"
    }

    fn starts<'w>(&self, w: &World<'w>) -> Vec<Start<'w>> {
        let p = w.p;
        let mut calls: Vec<(usize, Node<'w>, Node<'w>)> = vec![];
        for (i, f) in p.files.iter().enumerate().filter(|(i, f)| f.lang.id == "dart" && w.res.ready(*i)) {
            let l = f.lang;
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                stack.extend(kids(n));
                if let Expr::Call { callee, .. } = (l.expr)(n, &f.text)
                    && let Some(name) = crate::resolve::callee_name(callee, l).filter(|c| f.text_of(*c) == "runApp")
                {
                    calls.push((i, n, name));
                }
            }
        }
        w.res.prefetch(&calls.iter().map(|&(i, _, name)| (i, name)).collect::<Vec<_>>());
        let mut out = vec![];
        for (i, call, name) in calls {
            let f = &p.files[i];
            let Some(loc) = w.res.definition(i, name).into_iter().next() else { continue };
            if p.is_own(&loc.file) || !(f.lang.package)(&loc.file).is_some_and(|k| self.handles(&k)) {
                continue;
            }
            let mut n = call.parent();
            while let Some(x) = n.filter(|x| !f.lang.is_function(x.kind())) {
                n = x.parent();
            }
            if let Some(func) = n {
                out.push(Start { kind: "lifecycle", func: At { file: i, node: func }, by: At { file: i, node: call }, when: "when the app starts".into(), place: Some(Group::Device), optional: false, around: None });
            }
        }
        out.sort_by_key(|s| (s.by.file, s.by.node.start_byte()));
        out
    }

    fn routes<'w>(&self, p: &'w Project, res: &'w Resolver<'w>) -> Vec<Route<'w>> {
        let mut classes = vec![];
        for (i, f) in p.files.iter().enumerate().filter(|(_, f)| f.lang.id == "dart") {
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                if f.lang.reach.classes.contains(&n.kind()) {
                    classes.push(At { file: i, node: n });
                    continue;
                }
                stack.extend(kids(n));
            }
        }
        // a `State` draws for the widget that makes it
        let mut made_by: HashMap<crate::source::NodeKey, At<'w>> = HashMap::new();
        for c in &classes {
            if let Some(s) = state_of(res, *c) {
                made_by.insert(s.key(), *c);
            }
        }
        let mut places: Vec<(At<'w>, At<'w>)> = vec![];
        for c in &classes {
            if draws(&p.files[c.file], c.node) {
                places.push((*c, made_by.get(&c.key()).copied().unwrap_or(*c)));
            }
        }
        let name = |w: At<'w>| w.node.child_by_field_name("name").map(|n| p.files[w.file].text_of(n).to_string()).unwrap_or_default();
        let mut count: HashMap<String, usize> = HashMap::new();
        for (_, w) in &places {
            *count.entry(name(*w)).or_default() += 1;
        }
        places
            .into_iter()
            .map(|(place, widget)| {
                let n = name(widget);
                // a widget is known by its class; two of one name, by their files too
                let path = if count[&n] > 1 { format!("{n} · {}", p.files[widget.file].rel) } else { n.clone() };
                Route {
                    kind: RouteKind::Page,
                    segs: vec![Seg::Literal(path.clone())],
                    path,
                    method: None,
                    file: p.files[place.file].rel.clone(),
                    decl: place,
                    group: Group::Device,
                    how: "Flutter widget",
                    name: Some(n),
                    screen: false,
                }
            })
            .collect()
    }

    fn runs<'w>(&self, w: &World<'w>) -> Vec<At<'w>> {
        let mut out = vec![];
        for r in w.routes.iter().filter(|r| r.how == "Flutter widget") {
            let f = &w.p.files[r.decl.file];
            out.extend(LIFECYCLE.iter().filter_map(|m| method(f, r.decl.node, m)).map(|m| At { file: r.decl.file, node: m }));
        }
        // the widgets that make a `State`
        for (i, f) in w.p.files.iter().enumerate().filter(|(_, f)| f.lang.id == "dart") {
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                if f.lang.reach.classes.contains(&n.kind()) {
                    out.extend(method(f, n, "createState").map(|m| At { file: i, node: m }));
                    continue;
                }
                stack.extend(kids(n));
            }
        }
        out
    }

    fn triggers<'w>(&self, b: &mut Builder<'w>, decl: At<'w>) -> Vec<TriggerSite<'w>> {
        let f = b.file(decl);
        let l = f.lang;
        if !l.reach.classes.contains(&decl.node.kind()) {
            return vec![];
        }
        let mut found = vec![];
        let mut stack = vec![decl.node];
        while let Some(n) = stack.pop() {
            if (l.named_arg)(n, &f.text).is_some() {
                found.push(n);
            }
            stack.extend(kids(n).into_iter().rev());
        }
        let mut out = vec![];
        for a in found {
            let Some((label, value)) = (l.named_arg)(a, &f.text) else { continue };
            let label = f.text_of(label);
            // the widget the argument is given to
            let Some(holder) = a.parent().and_then(|args| args.parent()).filter(|c| l.call_field(c.kind()).is_some()) else { continue };
            // `on…` given to any widget, and what else Flutter runs when something happens to one of its own
            let ev = match event(label) {
                Some(ev) => ev,
                None if callback(label) && b.package_of(b.at(decl, holder), &b.chain(b.at(decl, holder))).is_some_and(|(p, _)| self.handles(&p)) => {
                    let mut c = label.chars();
                    c.next().map(|first| first.to_uppercase().chain(c).collect()).unwrap_or_default()
                }
                None => continue,
            };
            let Some((handler, named)) = self.handler(b, b.at(decl, value)) else { continue };
            let Expr::Call { callee, .. } = (l.expr)(holder, &f.text) else { continue };
            let control = one_line(f.text_of(callee), 40);
            let text = self.words(b, b.at(decl, holder), a);
            let mut name = ev.clone();
            name[..1].make_ascii_lowercase();
            out.push(TriggerSite {
                via: None,
                attr: b.at(decl, a),
                holder: b.at(decl, holder),
                event: ev.to_lowercase(),
                event_name: name,
                element: format!("{control}({})", one_line(f.text_of(a), 0)),
                control: Some(control),
                text,
                handler,
                named_handler: named,
            });
        }
        out
    }

    fn walk_function<'w>(&self, b: &mut Builder<'w>, decl: At<'w>, ctx: &Ctx, _entry: bool) -> Option<Vec<Item>> {
        let f = b.file(decl);
        if !f.lang.reach.classes.contains(&decl.node.kind()) || !draws(f, decl.node) {
            return None;
        }
        Some(self.widget(b, decl, ctx))
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        if l.id != "dart" {
            return None;
        }
        let call = l.unwrap(e.node);
        let Expr::Call { callee, args } = (l.expr)(call, &f.text) else { return None };
        // a widget of the project made here
        if let Some(class) = made(b.w.res, e.file, call) {
            let place = self.place(b.w.res, class)?;
            return self.render_widget(b, b.at(e, call), class, place, &args, ctx);
        }
        let (package, _) = b.package_of(e, chain)?;
        if !self.handles(&package) {
            return None;
        }
        let name = chain.segs.last().map(|s| s.name.clone()).or_else(|| crate::resolve::callee_name(callee, l).map(|n| f.text_of(n).to_string())).unwrap_or_default();
        if name == "setState" {
            return Some(self.set_state(b, e, &args, ctx));
        }
        if name.starts_with("push") || name.starts_with("show") {
            if let Some(items) = self.opens(b, e, &args, ctx) {
                return Some(items);
            }
        }
        self.handed(b, e, callee, &args, ctx)
    }
}
