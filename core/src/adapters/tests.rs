// Test runners adapter. Handles: which files hold tests, by the naming rules of the runners themselves
// (pytest: `test_*.py`, `*_test.py`, `conftest.py`; Jest, Vitest and Mocha: `*.test.*`, `*.spec.*`, `__tests__/`;
// Dart's `test` and `flutter_test`: `*_test.dart` and the package's `test/` and `integration_test/` folders; and a
// `tests/` folder). A story that starts in one of them is a test, so the view keeps tests apart.
// In those files, the calls a JavaScript or Dart runner registers code with (`describe`, `it`, `test`, `group`,
// `testWidgets`, `beforeEach`, `setUp`…): the function given to one runs when the runner runs it, so it is walked in
// place as a frame named by the call. The runners' own calls (`expect(…)`, `jest.fn()`) are how a test is written, not
// something it uses: not drawn.
// pytest fixtures: the value a test's parameter is given by the fixture it names (`fixture_value`), for adapters that
// need to know what a parameter holds (a framework's test client).
use super::{Adapter, import_of, is_imported};
use crate::lang::Expr;
use crate::model::{FrameKind, Item};
use crate::source::{At, Project, SourceFile, one_line};
use crate::story::{Builder, Chain, Ctx};
use tree_sitter::Node;

/// The names a runner registers code with.
const REGISTER: &[&str] = &[
    "describe", "context", "suite", "it", "test", "specify", "beforeEach", "afterEach", "beforeAll", "afterAll", "before", "after", "group", "testWidgets", "setUp", "tearDown",
    "setUpAll", "tearDownAll",
];
/// What may follow one: `it.only(…)`, `describe.skip(…)`, `test.each(table)(…)`.
const MODIFIERS: &[&str] = &["only", "skip", "todo", "concurrent", "sequential", "failing", "each"];
/// The runners' packages.
const RUNNERS: &[&str] = &["jest", "@jest/globals", "vitest", "mocha", "jasmine", "@types/jest", "@types/mocha", "@types/jasmine", "test", "flutter_test", "test_api", "matcher"];

pub struct Tests;

/// The file is a test or test support, by its path relative to the project.
pub fn is_test(rel: &str) -> bool {
    let mut parts: Vec<&str> = rel.split('/').collect();
    let name = parts.pop().unwrap_or("");
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    if parts.iter().any(|d| *d == "__tests__" || *d == "tests") {
        return true;
    }
    if name.ends_with(".py") {
        return stem.starts_with("test_") || stem.ends_with("_test") || stem == "conftest";
    }
    if name.ends_with(".dart") {
        return stem.ends_with("_test") || parts.iter().any(|d| *d == "test" || *d == "integration_test");
    }
    let mut dots = stem.split('.').skip(1);
    dots.any(|d| d == "test" || d == "spec")
}

/// pytest collects `test*` functions of `test_*.py` and `*_test.py` files, and the `test*` methods of classes named
/// `Test*` or written on a base class (unittest's `TestCase`). `class`: the enclosing class's name and whether it has
/// a base other than `object`. Everything else (a fixture, a helper class, `conftest.py`) is run only when a test asks.
fn pytest_collects(rel: &str, class: Option<(&str, bool)>, name: &str) -> bool {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    let stem = file.strip_suffix(".py").unwrap_or("");
    if !(stem.starts_with("test_") || stem.ends_with("_test")) || !name.starts_with("test") {
        return false;
    }
    class.is_none_or(|(class, based)| class.starts_with("Test") || based)
}

/// The decorators written on a function: its own wrapper's.
fn decorators(func: Node) -> Vec<Node> {
    func.parent().filter(|p| p.kind() == "decorated_definition").map(|p| kids(p).into_iter().filter(|d| d.kind() == "decorator").collect()).unwrap_or_default()
}

/// A decorator that is pytest's own (`@pytest.fixture`, `@pytest.mark.slow`, `@fixture(autouse=True)`): its first name
/// is imported from pytest.
fn from_pytest(f: &SourceFile, decorator: Node) -> bool {
    let Some(mut e) = decorator.named_child(0) else { return false };
    loop {
        let c = f.lang.unwrap(e);
        if f.lang.is_name(c.kind()) {
            return import_of(f, c).is_some_and(|i| i.module == "pytest");
        }
        let next = match (f.lang.expr)(c, &f.text) {
            Expr::Call { callee, .. } => Some(callee),
            _ => f.lang.member_fields(c.kind()).and_then(|(obj, _)| c.child_by_field_name(obj)),
        };
        match next {
            Some(n) => e = n,
            None => return false,
        }
    }
}

fn kids(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

/// pytest: the value a test's parameter `name` is given when it names a fixture (a function of that name decorated
/// with `pytest.fixture`, in the test's own file or in a `conftest.py` of its folder or one above, nearest first):
/// what the fixture returns or yields.
pub fn fixture_value<'w>(p: &'w Project, file: usize, name: &str) -> Option<At<'w>> {
    let dir = |rel: &str| rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
    let here = dir(&p.files[file].rel);
    let mut conftests: Vec<(usize, usize)> = p
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.rel.rsplit('/').next() == Some("conftest.py"))
        .filter_map(|(i, f)| {
            let d = dir(&f.rel);
            (d.is_empty() || here == d || here.starts_with(&format!("{d}/"))).then_some((d.len(), i))
        })
        .collect();
    conftests.sort_by(|a, b| b.0.cmp(&a.0));
    for i in std::iter::once(file).chain(conftests.into_iter().map(|(_, i)| i)) {
        let f = &p.files[i];
        for top in kids(f.tree.root_node()).into_iter().filter(|t| t.kind() == "decorated_definition") {
            let Some(func) = top.child_by_field_name("definition").filter(|d| f.lang.is_function(d.kind())) else { continue };
            if func.child_by_field_name("name").map(|n| f.text_of(n)) != Some(name) {
                continue;
            }
            let fixture = kids(top).into_iter().filter(|d| d.kind() == "decorator").filter_map(|d| d.named_child(0)).any(|d| {
                let callee = match (f.lang.expr)(d, &f.text) {
                    Expr::Call { callee, .. } => callee,
                    _ => d,
                };
                is_imported(f, callee, &|m| m == "pytest", "fixture")
            });
            if fixture {
                return given(f, func).map(|n| At { file: i, node: n });
            }
        }
    }
    None
}

/// What a fixture gives: the value of its first `return` or `yield`, in the order written; a name it yields that a
/// `with … as name` opened is the value it was opened with.
fn given<'t>(f: &SourceFile, func: Node<'t>) -> Option<Node<'t>> {
    fn first<'t>(f: &SourceFile, n: Node<'t>) -> Option<Node<'t>> {
        for k in kids(n) {
            if f.lang.is_function(k.kind()) {
                continue;
            }
            if k.kind() == "yield" || k.kind() == f.lang.follow.returns {
                if let Some(v) = k.named_child(0) {
                    return Some(v);
                }
            }
            if let Some(v) = first(f, k) {
                return Some(v);
            }
        }
        None
    }
    let v = first(f, func.child_by_field_name("body")?)?;
    if !f.lang.is_name(v.kind()) {
        return Some(v);
    }
    let mut stack = vec![func];
    while let Some(n) = stack.pop() {
        if n.kind() == "as_pattern" && n.child_by_field_name("alias").is_some_and(|a| f.text_of(a) == f.text_of(v)) {
            return n.named_child(0);
        }
        stack.extend(kids(n));
    }
    Some(v)
}

/// `it.only` → ["it", "only"]; `test.each(table)` → ["test", "each"]: the names a callee is made of.
fn names<'w>(b: &Builder<'w>, e: At<'w>, callee: Node<'w>) -> Option<Vec<String>> {
    let l = b.lang(e);
    let c = l.unwrap(callee);
    if l.is_name(c.kind()) {
        return Some(vec![b.text(b.at(e, c)).to_string()]);
    }
    if let Some((obj, prop)) = l.member_fields(c.kind()) {
        let mut out = names(b, e, c.child_by_field_name(obj)?)?;
        out.push(b.text(b.at(e, c.child_by_field_name(prop)?)).to_string());
        return Some(out);
    }
    match (l.expr)(c, &b.file(e).text) {
        Expr::Call { callee, .. } => names(b, e, callee),
        _ => None,
    }
}

impl Adapter for Tests {
    fn name(&self) -> &'static str {
        "test runners"
    }
    fn test(&self, file: &SourceFile) -> bool {
        is_test(&file.rel)
    }
    fn handles(&self, package: &str) -> bool {
        RUNNERS.contains(&package)
    }
    fn support(&self, file: &SourceFile, decl: Node) -> bool {
        if file.lang.server != "python" || !is_test(&file.rel) {
            return false;
        }
        let Some(name) = (file.lang.function_name)(decl) else { return false };
        // the class it is written in, if any
        let mut class = None;
        let mut up = decl.parent();
        while let Some(x) = up {
            if x.kind() == "class_definition" {
                let based = x.child_by_field_name("superclasses").is_some_and(|b| kids(b).iter().any(|k| file.text_of(*k) != "object"));
                class = x.child_by_field_name("name").map(|n| (file.text_of(n), based));
                break;
            }
            up = x.parent();
        }
        // a function another framework registers by a decorator (a route of an app written for a test) is started by it
        let registered = decorators(decl).into_iter().any(|d| !from_pytest(file, d));
        !registered && !pytest_collects(&file.rel, class, file.text_of(name))
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, _chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        let l = f.lang;
        if !is_test(&f.rel) {
            return None;
        }
        let Expr::Call { callee, args } = (l.expr)(l.unwrap(e.node), &f.text) else { return None };
        let words = names(b, e, callee)?;
        if !REGISTER.contains(&words[0].as_str()) || !words[1..].iter().all(|w| MODIFIERS.contains(&w.as_str())) {
            return None;
        }
        // a function of the project's own that happens to be called `it` is not the runner's
        if b.project_decl(b.at(e, callee)).is_some() {
            return None;
        }
        let func = args.iter().rev().map(|a| l.unwrap(*a)).find(|a| l.is_function(a.kind()))?;
        let mut out = vec![];
        for a in &args {
            if l.unwrap(*a) != func {
                out.extend(b.expr(b.at(e, *a), ctx));
            }
        }
        let items = b.callback(b.at(e, func), ctx);
        // `it('adds two')`: the call, with the words it names the test by
        let words = args.iter().find(|a| l.strings.nodes.contains(&a.kind())).map(|a| one_line(f.text_of(*a), 60));
        let label = format!("{}({})", one_line(f.text_of(callee), 40), words.unwrap_or_else(|| "…".into()));
        out.push(Item::Frame { id: b.id(), kind: FrameKind::Effect, label, note: Some("when the test runner runs it".into()), at: b.place(e), items, els: None });
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_test, pytest_collects};

    #[test]
    fn pytest_collects_by_its_naming_rules() {
        assert!(pytest_collects("tests/test_a.py", None, "test_save"));
        assert!(pytest_collects("a_test.py", Some(("TestSave", false)), "test_it"));
        assert!(pytest_collects("test_a.py", Some(("SaveTests", true)), "test_it"));
        assert!(!pytest_collects("tests/conftest.py", None, "test_x"));
        assert!(!pytest_collects("tests/test_a.py", None, "client"));
        assert!(!pytest_collects("tests/test_a.py", Some(("Actions", false)), "test_login"));
        assert!(!pytest_collects("tests/helpers.py", None, "test_x"));
    }

    #[test]
    fn test_files_by_the_runners_naming_rules() {
        for yes in ["tests/test_orders.py", "pkg/test_x.py", "pkg/x_test.py", "conftest.py", "src/a.test.ts", "src/a.spec.tsx", "lib/__tests__/a.ts", "tests/helpers.py", "app/test/a_test.dart", "test/fakes/repo.dart", "x/integration_test/app.dart"] {
            assert!(is_test(yes), "{yes}");
        }
        for no in ["app/main.py", "src/testing.ts", "src/latest.ts", "contest.py", "src/a.ts", "testdata.py", "lib/testing.dart", "lib/latest.dart"] {
            assert!(!is_test(no), "{no}");
        }
    }
}
