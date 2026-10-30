// Python.
use super::*;

const NAMES: &[&str] = &["identifier"];

fn body<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    if n.kind() == "block" { named_children(n).into_iter().filter(|c| c.kind() != "comment").collect() } else { vec![n] }
}

fn block_of<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    n.child_by_field_name("body")
        .or_else(|| n.child_by_field_name("consequence"))
        .or_else(|| named_children(n).into_iter().rev().find(|c| c.kind() == "block"))
        .map(body)
        .unwrap_or_default()
}

fn stmt<'t>(n: Node<'t>, src: &str) -> Stmt<'t> {
    match n.kind() {
        "return_statement" => Stmt::Return(n.named_child(0)),
        // `raise E(…) from cause`: the value is the first part
        "raise_statement" => Stmt::Throw(n.named_child(0).filter(|v| Some(*v) != n.child_by_field_name("cause"))),
        // `elif` is an `if` of its own whose else part is the next clause
        "if_statement" | "elif_clause" => {
            let cond = n.child_by_field_name("condition").unwrap();
            let then = n.child_by_field_name("consequence").map(body).unwrap_or_default();
            let next = if n.kind() == "if_statement" { field_children(n, "alternative").into_iter().next() } else { n.next_named_sibling() };
            let els = match next {
                Some(e) if e.kind() == "elif_clause" => vec![e],
                Some(e) if e.kind() == "else_clause" => block_of(e),
                _ => vec![],
            };
            Stmt::If { cond, then, els }
        }
        "else_clause" => Stmt::Block(block_of(n)),
        "for_statement" | "while_statement" => match n.child_by_field_name("body") {
            Some(b) => Stmt::Loop { body_node: b, body: body(b) },
            None => Stmt::Other,
        },
        "try_statement" => {
            let kids = named_children(n);
            Stmt::Try {
                body: n.child_by_field_name("body").map(body).unwrap_or_default(),
                catches: kids.iter().filter(|c| c.kind() == "except_clause" || c.kind() == "except_group_clause").map(|c| (*c, block_of(*c))).collect(),
                finally: kids.iter().filter(|c| c.kind() == "finally_clause").flat_map(|c| block_of(*c)).collect(),
            }
        }
        "with_statement" | "block" => Stmt::Block(if n.kind() == "block" { body(n) } else { block_of(n) }),
        "expression_statement" => {
            let Some(e) = n.named_child(0) else { return Stmt::Other };
            if e.kind() == "assignment" || e.kind() == "augmented_assignment" {
                let mut names = vec![];
                if let Some(l) = e.child_by_field_name("left") {
                    bound_names(l, src, NAMES, &mut names);
                }
                return Stmt::Decl { keyword: String::new(), names, values: e.child_by_field_name("right").into_iter().collect() };
            }
            Stmt::Expr(e)
        }
        "break_statement" | "continue_statement" => jump(n, &["for_statement", "while_statement"], &[], &["function_definition", "lambda"]),
        "function_definition" | "class_definition" | "decorated_definition" => Stmt::Definition,
        "assert_statement" => Stmt::Eval(named_children(n)),
        _ => Stmt::Other,
    }
}

fn expr<'t>(n: Node<'t>, src: &str) -> Expr<'t> {
    match n.kind() {
        "call" => match n.child_by_field_name("function") {
            Some(callee) => Expr::Call {
                callee,
                args: n.child_by_field_name("arguments").map(|a| named_children(a).into_iter().filter(|c| c.kind() != "comment").collect()).unwrap_or_default(),
            },
            None => Expr::Other,
        },
        // `a if cond else b`
        "conditional_expression" => {
            let k = named_children(n);
            if k.len() == 3 { Expr::Cond { cond: k[1], then: k[0], els: Some(k[2]), negated: false } } else { Expr::Other }
        }
        "boolean_operator" => {
            let op = n.child_by_field_name("operator").map(|o| text(o, src)).unwrap_or("");
            match (n.child_by_field_name("left"), n.child_by_field_name("right")) {
                (Some(l), Some(r)) => Expr::Cond { cond: l, then: r, els: None, negated: op == "or" },
                _ => Expr::Other,
            }
        }
        "lambda" => Expr::Function,
        _ => Expr::Other,
    }
}

/// Every import in the file, in source order: those inside a block (`try: import x`), a function or a class too,
/// since a module imported where it is used is still imported. The name one inside a function or a class gives is
/// known only there.
fn imports(root: Node, src: &str) -> Vec<Import> {
    let mut out = vec![];
    let mut stack: Vec<(Node, Option<(usize, usize)>)> = vec![(root, None)];
    while let Some((st, scope)) = stack.pop() {
        let from = st.kind() == "import_from_statement";
        if !from && st.kind() != "import_statement" {
            let inner = if matches!(st.kind(), "function_definition" | "class_definition") { Some((st.start_byte(), st.end_byte())) } else { scope };
            stack.extend(named_children(st).into_iter().rev().map(|k| (k, inner)));
            continue;
        }
        let module_name = st.child_by_field_name("module_name");
        let module = module_name.map(|m| text(m, src).to_string());
        for n in field_children(st, "name") {
            let (name, local) = if n.kind() == "aliased_import" { (n.child_by_field_name("name").unwrap(), n.child_by_field_name("alias").unwrap()) } else { (n, n) };
            // `from module import name`: the module is written after `from`; `import module`: it is the name
            let at = module_name.unwrap_or(name).start_position();
            let (module, name) = match &module {
                Some(m) => (m.clone(), text(name, src).to_string()),
                None => (text(name, src).to_string(), "*".to_string()),
            };
            out.push(Import { local: text(local, src).to_string(), module, name, line: local.start_position().row, col: local.start_position().column, module_at: (at.row, at.column), scope });
        }
    }
    out
}

fn function_name<'t>(n: Node<'t>) -> Option<Node<'t>> {
    if let Some(name) = n.child_by_field_name("name") {
        return Some(name);
    }
    let p = n.parent()?;
    match p.kind() {
        "assignment" => p.child_by_field_name("left").filter(|l| l.kind() == "identifier"),
        "keyword_argument" | "pair" => p.child_by_field_name("name").or_else(|| p.child_by_field_name("key")),
        _ => None,
    }
}

fn function_body<'t>(n: Node<'t>) -> Option<Node<'t>> {
    n.child_by_field_name("body")
}

fn bases<'t>(class: Node<'t>) -> Vec<Node<'t>> {
    // `class A(B, C, metaclass=M)`: the keyword arguments are not bases
    class.child_by_field_name("superclasses").map(|a| named_children(a).into_iter().filter(|b| b.kind() != "keyword_argument" && b.kind() != "comment").collect()).unwrap_or_default()
}

fn members<'t>(class: Node<'t>) -> Vec<Node<'t>> {
    let Some(body) = class.child_by_field_name("body") else { return vec![] };
    named_children(body)
        .into_iter()
        .filter_map(|m| if m.kind() == "decorated_definition" { m.child_by_field_name("definition") } else { Some(m) })
        .filter(|m| m.kind() == "function_definition")
        .filter_map(|m| m.child_by_field_name("name"))
        .collect()
}

/// `@property`, `@cached_property`, `@name.setter`: the function runs when the attribute is read or set.
fn accessor(f: Node, src: &str) -> bool {
    let Some(d) = f.parent().filter(|p| p.kind() == "decorated_definition") else { return false };
    named_children(d).into_iter().filter(|c| c.kind() == "decorator").any(|c| {
        let t = text(c, src).trim_start_matches('@').trim();
        matches!(t.rsplit('.').next(), Some("property" | "cached_property" | "setter" | "getter" | "deleter"))
    })
}

/// An installed distribution (`site-packages/<name>/…`), or the stubs the server brings for one
/// (typeshed's `stubs/<distribution>/<package>/…`). Anything else outside the project is Python's own library.
fn package(p: &std::path::Path) -> Option<String> {
    let parts = parts(p);
    let module = |c: &str| c.trim_end_matches(".pyi").trim_end_matches(".py").trim_end_matches("-stubs").to_string();
    if let Some(i) = parts.iter().rposition(|c| *c == "site-packages" || *c == "dist-packages") {
        let top = module(parts.get(i + 1)?);
        // a namespace package (a folder with no `__init__`) holds several distributions: name the one inside
        let dir: std::path::PathBuf = parts[..=i + 1].iter().collect();
        let namespace = dir.is_dir() && !dir.join("__init__.py").exists() && !dir.join("__init__.pyi").exists();
        return Some(match parts.get(i + 2).filter(|_| namespace && parts.len() > i + 3) {
            Some(sub) => format!("{top}.{}", module(sub)),
            None => top,
        });
    }
    let i = parts.iter().rposition(|c| *c == "stubs")?;
    if i > 0 && parts[i - 1].contains("typeshed") { parts.get(i + 2).map(|m| module(m)) } else { None }
}

pub static PYTHON: Lang = Lang {
    id: "python",
    exts: &[".py"],
    grammar: || tree_sitter_python::LANGUAGE.into(),
    server: "python",
    lsp_id: "python",
    line_comment: "#",
    package_sep: '.',
    functions: &["function_definition", "lambda"],
    calls: &[("call", "function")],
    elements: &[],
    unwrap: &["parenthesized_expression", "await"],
    unwrap_one: &[],
    member: &[("attribute", "object", "attribute")],
    names: NAMES,
    strings: Strings { nodes: &["string", "concatenated_string"], fragments: &["string_content", "escape_sequence"], interpolations: &["interpolation"] },
    numbers: &["integer", "float"],
    markup_text: &[],
    truthy: &["True"],
    falsy: &["False", "None", "\"\"", "''", "0"],
    iterate: &[],
    promise: &[],
    follow: Follow {
        declarators: &[("assignment", "left", "right")],
        destructure: None,
        list: Some((&["pattern_list", "tuple_pattern", "list_pattern"], &["expression_list", "tuple", "list"])),
        returns: "return_statement",
        object: None,
        parameter: &["parameters", "typed_parameter", "default_parameter", "typed_default_parameter", "lambda_parameters"],
    },
    blocks: &["block"],
    awaited: None,
    not_operand: |n, _| if n.kind() == "not_operator" { n.child_by_field_name("argument") } else { None },
    discard: |_, _| None,
    params: |f, src| {
        f.child_by_field_name("parameters")
            .map(|l| named_children(l).into_iter().map(|c| text(c.child_by_field_name("name").or_else(|| c.named_child(0).filter(|_| c.kind() != "identifier")).unwrap_or(c), src).to_string()).collect())
            .unwrap_or_default()
    },
    stmt,
    expr,
    body,
    imports,
    function_name,
    function_body,
    reach: Reach {
        decorators: &["decorator"],
        classes: &["class_definition"],
        bases,
        members,
        implicit: |name| name.len() > 4 && name.starts_with("__") && name.ends_with("__"),
        accessor,
        not_uses: &["import_statement", "import_from_statement", "future_import_statement"],
        exposes: &[],
    },
    package,
    callable: &["->", "Callable"],
    named_arg: |a, _| if a.kind() == "keyword_argument" { Some((a.child_by_field_name("name")?, a.child_by_field_name("value")?)) } else { None },
    // an abstract method has a body of its own (`...`, `pass`, a docstring)
    abstract_member: |_| None,
    signature: signature_after,
    // `except E:`, `except (A, B) as e:`, `except A, B:`; a bare `except:`, or the base of every error, handles anything
    caught: |clause, src| {
        let mut types = vec![];
        for mut v in field_children(clause, "value") {
            if v.kind() == "as_pattern" {
                v = v.named_child(0)?;
            }
            types.extend(if v.kind() == "tuple" { named_children(v) } else { vec![v] }.into_iter().map(|t| last_part(text(t, src))));
        }
        if types.is_empty() || types.iter().any(|t| t == "Exception" || t == "BaseException") { None } else { Some(types) }
    },
    caught_as: |clause, src| {
        let alias = field_children(clause, "value").into_iter().find(|v| v.kind() == "as_pattern")?.child_by_field_name("alias")?;
        Some(text(alias, src).to_string())
    },
    assigns: &[("assignment", "left"), ("augmented_assignment", "left")],
    own: Own::FirstParam,
    // `def f(c: Client)`, `c: Client = None`
    dynamic_import: None,
    typed: |name| {
        let p = name.parent().filter(|p| matches!(p.kind(), "typed_parameter" | "typed_default_parameter"))?;
        let t = p.child_by_field_name("type").filter(|_| p.child_by_field_name("name").or_else(|| p.named_child(0)) == Some(name))?;
        t.named_child(0).filter(|x| x.kind() == "identifier")
    },
};
