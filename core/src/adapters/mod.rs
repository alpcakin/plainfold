// Framework and platform adapters. Each one gives meaning to a framework's conventions (which files are screens,
// what a state change or a database call is) and nothing else. The story builder asks every active adapter at a
// few fixed points; with no adapter active it still walks code, it just finds no entry points.
//
// Each adapter states what it handles at the top of its file. Adapters may use one language's syntax
// (a JSX element is TypeScript/JavaScript syntax), but they never name a project's files or functions.
use crate::resolve::{Resolver, ValueRules};
use crate::source::{At, Project, SourceFile};
use crate::story::routes::Route;
use crate::story::{Builder, Chain, Ctx, World};
use crate::model::{Group, Item};
use tree_sitter::Node;

mod apscheduler;
mod dart_http;
mod expo_router;
mod fastapi;
mod flutter;
mod flutter_state;
mod nextjs;
mod react;
mod react_hook_form;
mod react_native;
mod styling;
mod supabase;
mod swr;
mod tanstack_query;
pub mod tests;
mod web;

/// A user action found in a screen: the control, the event, the words on it, and the function it runs.
pub struct TriggerSite<'w> {
    /// the attribute (or registration) that attaches the handler
    pub attr: At<'w>,
    /// the element that holds it, shown as detail
    pub holder: At<'w>,
    /// "press", "submit"
    pub event: String,
    /// the event as the code names it, without its prefix ("press" for `onPress`)
    pub event_name: String,
    /// `<Button onPress={save}>`
    pub element: String,
    pub control: Option<String>,
    /// the words people see on the control
    pub text: String,
    pub handler: At<'w>,
    /// the handler is referred to by name (not written inline)
    pub named_handler: bool,
    /// the call of a project function that returned the handler (`save = guard(check, f)`), when it did
    pub via: Option<At<'w>>,
}

/// A returned value that is a response to a request: its label ("200 { books }") and the values it evaluates.
pub struct Response<'w> {
    pub label: String,
    pub args: Vec<At<'w>>,
}

/// Code registered to run later, at a moment the framework defines (after a render, when a screen comes into view).
pub struct Later<'w> {
    pub name: String,
    pub func: At<'w>,
    pub deps: Option<String>,
    pub when: String,
    /// what the code is called with when it runs (the key a fetcher is given), when known
    pub given: Vec<At<'w>>,
}

/// A function that a framework or a library starts by itself, where the code hands it over (a job given to a
/// scheduler, the function an app runs when it starts): a story of its own starts there.
pub struct Start<'w> {
    /// "schedule" | "lifecycle"
    pub kind: &'static str,
    pub func: At<'w>,
    /// the code that hands it over: the call, or the decorator
    pub by: At<'w>,
    /// when it runs, in the adapter's words around the code's values (`every 5 minutes`)
    pub when: String,
    /// where it runs, when the adapter knows; otherwise it is where the code that hands it over runs
    pub place: Option<Group>,
    /// it is a story only where it does more than draw (a layout that only renders its children is not)
    pub optional: bool,
    /// a layout: the address of the screens it is drawn around (`/docs`)
    pub around: Option<String>,
}

#[allow(unused_variables)]
pub trait Adapter {
    fn name(&self) -> &'static str;
    /// The languages whose code this adapter reads, by their language server ("typescript", "python", "dart"); none
    /// named: any. A package is matched only in code of these, so one name in two languages is two packages.
    fn langs(&self) -> &'static [&'static str] {
        &[]
    }
    /// The adapter reads code of this language.
    fn reads(&self, lang: &crate::lang::Lang) -> bool {
        self.langs().is_empty() || self.langs().contains(&lang.server)
    }
    /// An installed package this adapter gives meaning to: its calls are left to the adapter, never drawn as plain
    /// calls into a package.
    fn handles(&self, package: &str) -> bool {
        false
    }
    /// An installed package whose import turns this adapter on: those it handles, and any whose calls it only
    /// partly gives meaning to (the others stay plain calls into the package).
    fn applies(&self, package: &str) -> bool {
        self.handles(package)
    }
    /// Screens and endpoints this framework defines.
    fn routes<'w>(&self, p: &'w Project, res: &'w Resolver<'w>) -> Vec<Route<'w>> {
        vec![]
    }
    /// Functions the framework runs itself where the code hands them to it (a dependency given to `Depends`): they
    /// are walked where it runs them, so they are not entry points of their own.
    fn runs<'w>(&self, w: &World<'w>) -> Vec<At<'w>> {
        vec![]
    }
    /// The framework runs this function only where the code names it (a component, rendered where it is written as
    /// an element): one nothing names never runs, so it is no entry point.
    fn only_named(&self, file: &SourceFile, decl: Node) -> bool {
        false
    }
    /// The function sits where the runner runs code, but the runner never starts it by itself: support code a test
    /// asks for (a fixture) or a helper. It is no entry point.
    fn support(&self, file: &SourceFile, decl: Node) -> bool {
        false
    }
    /// Functions the framework or library starts by itself, each with when it runs.
    fn starts<'w>(&self, w: &World<'w>) -> Vec<Start<'w>> {
        vec![]
    }
    /// User actions inside a screen.
    fn triggers<'w>(&self, b: &mut Builder<'w>, decl: At<'w>) -> Vec<TriggerSite<'w>> {
        vec![]
    }
    /// Places that hold user actions and are no screen (a component with a button). Each user action in one is a
    /// story there, told once however many screens draw it.
    fn controls<'w>(&self, w: &World<'w>) -> Vec<Route<'w>> {
        vec![]
    }
    /// Where the interface this framework draws runs, when it says (a native app's, on a device).
    fn draws_on(&self) -> Option<Group> {
        None
    }
    /// Walks a function its own way (a component's render passes, for example). `entry`: the function is the story's start.
    fn walk_function<'w>(&self, b: &mut Builder<'w>, decl: At<'w>, ctx: &Ctx, entry: bool) -> Option<Vec<Item>> {
        None
    }
    /// An expression this framework gives meaning to.
    fn expr<'w>(&self, b: &mut Builder<'w>, e: At<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        None
    }
    /// A step for an element of a package's (`tag` is its name) that this framework gives meaning to: a tab of a tab
    /// bar opens a screen. Its attributes and what it holds are walked as any others.
    fn element<'w>(&self, b: &mut Builder<'w>, e: At<'w>, tag: At<'w>, ctx: &Ctx) -> Option<Item> {
        None
    }
    /// What runs because of this call, before the steps the call itself shows (reading a provider runs its code the
    /// first time).
    fn before<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Vec<Item> {
        vec![]
    }
    /// A call this framework gives meaning to.
    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        None
    }
    /// A value returned from an endpoint that is a response.
    fn response<'w>(&self, b: &mut Builder<'w>, e: At<'w>) -> Option<Response<'w>> {
        None
    }
    /// A value thrown that the framework answers a request with, wherever it is thrown: its label (`403 "…"`).
    fn thrown<'w>(&self, b: &mut Builder<'w>, e: At<'w>) -> Option<String> {
        None
    }
    /// Where the function always runs, whoever calls it, and what makes it so (a server action: the browser calls it,
    /// the server runs it).
    /// `made`: the call of a project function that returned it, when one did (the name it is stored under is where it
    /// is written).
    fn runs_on<'w>(&self, b: &Builder<'w>, decl: At<'w>, made: Option<At<'w>>) -> Option<(Group, &'static str)> {
        None
    }
    /// The function runs inside its caller, on the caller's lifeline.
    fn same_lane<'w>(&self, b: &Builder<'w>, decl: At<'w>, ctx: &Ctx) -> bool {
        false
    }
    /// A function that says nothing about the story unless it crosses a boundary.
    fn silent<'w>(&self, b: &Builder<'w>, decl: At<'w>) -> bool {
        false
    }
    /// A call that registers code to run later.
    fn later<'w>(&self, b: &Builder<'w>, call: At<'w>) -> Option<Later<'w>> {
        None
    }
    /// The file holds tests: stories that start in it are tests.
    fn test(&self, file: &SourceFile) -> bool {
        false
    }
    /// Checks over a function the story runs (links to pages that do not exist, for example).
    fn check<'w>(&self, b: &mut Builder<'w>, decl: At<'w>) {}
    /// `x = wrap(f)` where the value of `x` is `f` itself.
    fn wrapped<'p>(&self, file: &'p SourceFile, call: Node<'p>) -> Option<Node<'p>> {
        None
    }
    /// A callee whose target is found by resolving another expression instead, and how that one is reached.
    fn redirect<'p>(&self, res: &Resolver<'p>, file: usize, callee: Node<'p>) -> Option<(At<'p>, &'static str)> {
        None
    }
    /// The values a call gives, where the framework decides them, and how they are reached.
    fn gives<'p>(&self, res: &Resolver<'p>, file: usize, call: Node<'p>) -> Option<(Vec<At<'p>>, &'static str)> {
        None
    }
    /// The values at one place of the list a call gives, and how they are reached.
    fn gives_at<'p>(&self, res: &Resolver<'p>, file: usize, call: Node<'p>, place: usize) -> Option<(Vec<At<'p>>, &'static str)> {
        None
    }
}

pub struct Adapters {
    pub list: Vec<Box<dyn Adapter>>,
}

impl Adapters {
    /// The adapters that apply to a project, decided by what the project's code imports.
    pub fn for_project(p: &Project) -> Adapters {
        // a framework adapter applies when the project imports a package it handles
        let imports = |a: &dyn Adapter| p.files.iter().filter(|f| a.reads(f.lang)).any(|f| f.imports.iter().any(|i| a.applies(&crate::resolve::package_of(&i.module, f.lang.package_sep))));
        let mut list: Vec<Box<dyn Adapter>> = vec![];
        let candidates: Vec<Box<dyn Adapter>> =
            vec![
                Box::new(react::React),
                Box::new(react_native::ReactNative),
                Box::new(react_hook_form::ReactHookForm),
                Box::new(tanstack_query::TanStackQuery),
                Box::new(swr::Swr),
                Box::new(styling::Styling),
                Box::new(nextjs::NextJs),
                Box::new(expo_router::ExpoRouter),
                Box::new(fastapi::FastApi),
                Box::new(supabase::Supabase),
                Box::new(flutter::Flutter),
                Box::new(flutter_state::FlutterState),
                Box::new(dart_http::DartHttp),
                Box::new(apscheduler::ApScheduler),
            ];
        for a in candidates {
            if imports(a.as_ref()) {
                list.push(a);
            }
        }
        if p.files.iter().any(|f| tests::is_test(&f.rel)) {
            list.push(Box::new(tests::Tests));
        }
        if p.files.iter().any(|f| f.lang.server == "typescript") {
            list.push(Box::new(web::Web));
        }
        Adapters { list }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.list.iter().map(|a| a.name()).collect()
    }
}

impl ValueRules for Adapters {
    fn wrapped<'p>(&self, file: &'p SourceFile, call: Node<'p>) -> Option<Node<'p>> {
        self.list.iter().find_map(|a| a.wrapped(file, call))
    }
    fn redirect<'p>(&self, res: &Resolver<'p>, file: usize, callee: Node<'p>) -> Option<(At<'p>, &'static str)> {
        self.list.iter().find_map(|a| a.redirect(res, file, callee))
    }
    fn gives<'p>(&self, res: &Resolver<'p>, file: usize, call: Node<'p>) -> Option<(Vec<At<'p>>, &'static str)> {
        self.list.iter().find_map(|a| a.gives(res, file, call))
    }
    fn gives_at<'p>(&self, res: &Resolver<'p>, file: usize, call: Node<'p>, place: usize) -> Option<(Vec<At<'p>>, &'static str)> {
        self.list.iter().find_map(|a| a.gives_at(res, file, call, place))
    }
}

// ---------- helpers adapters share ----------

/// The import a name refers to in its file, if the name is not declared again locally.
pub fn import_of<'a>(f: &'a SourceFile, name: Node) -> Option<&'a crate::lang::Import> {
    f.import_at(f.text_of(name), name)
}

/// `callee` is `name` imported from `module` (or a member `name` of a namespace imported from `module`).
pub fn is_imported(f: &SourceFile, callee: Node, module: &dyn Fn(&str) -> bool, name: &str) -> bool {
    let l = f.lang;
    let c = l.unwrap(callee);
    if l.is_name(c.kind()) {
        return import_of(f, c).is_some_and(|i| module(&i.module) && i.name == name);
    }
    if let Some((obj, prop)) = l.member_fields(c.kind()) {
        if let (Some(o), Some(p)) = (c.child_by_field_name(obj), c.child_by_field_name(prop)) {
            return f.text_of(p) == name && l.is_name(o.kind()) && import_of(f, o).is_some_and(|i| module(&i.module) && (i.name == "default" || i.name == "*"));
        }
    }
    false
}

/// A navigation call (`router.push(url)`): a reference to the screen of the project that answers the URL, told in its
/// own story; a URL no screen answers is a plain row.
pub fn navigation<'w>(b: &mut Builder<'w>, e: At<'w>, url: Option<String>, ctx: &Ctx) -> Item {
    use crate::model::*;
    use crate::source::one_line;
    use crate::story::routes::{RouteKind, match_route};
    let route = url.as_ref().filter(|u| u.starts_with('/')).and_then(|u| match_route(&b.w.routes, u, RouteKind::Page, None)).cloned();
    let place = b.place(e);
    let Some(route) = route else {
        return Item::Own { id: b.id(), at: ctx.this.clone(), kind: SelfKind::Local, label: one_line(b.text(e), 40), detail: None, place };
    };
    let to = b.screen(&route);
    let target = b.place(route.decl);
    // a screen whose opening is a story of its own is a reference to it; one with nothing to tell is just the place
    let told = b.told(&route);
    Item::Call(Call {
        id: b.id(),
        from: ctx.this.clone(),
        to,
        label: one_line(b.text(e), 40),
        detail: Some(one_line(b.text(e), 0)),
        link: Link { kind: LinkKind::Exact, how: format!("{}: {} → {}", route.how, route.path, route.file) },
        effect: None,
        at: place,
        target: Some(target),
        body: None,
        note: None,
        opens: told.then(|| route.path.clone()),
        story: None,
        table: None,
        touches: None,
        unused: false,
        into: None,
        ends: vec![],
        changes: vec![],
    })
}
