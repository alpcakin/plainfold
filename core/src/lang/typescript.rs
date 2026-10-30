// TypeScript / JavaScript (and their JSX forms).
use super::*;

const FUNCTIONS: &[&str] = &["function_declaration", "generator_function_declaration", "function_expression", "function", "arrow_function", "method_definition"];
const NAMES: &[&str] = &["identifier", "property_identifier", "type_identifier", "shorthand_property_identifier"];

fn body<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    if n.kind() == "statement_block" { named_children(n).into_iter().filter(|c| c.kind() != "comment").collect() } else { vec![n] }
}

fn stmt<'t>(n: Node<'t>, src: &str) -> Stmt<'t> {
    match n.kind() {
        "return_statement" => Stmt::Return(named_children(n).into_iter().find(|c| c.kind() != "comment")),
        "throw_statement" => Stmt::Throw(named_children(n).into_iter().find(|c| c.kind() != "comment")),
        // `export const x = …`, `export function f…`, `export default value`: what is exported is the statement
        "export_statement" => match (n.child_by_field_name("declaration"), n.child_by_field_name("value")) {
            (Some(d), _) => stmt(d, src),
            (None, Some(v)) => Stmt::Expr(v),
            _ => Stmt::Other,
        },
        "if_statement" => {
            let cond = n.child_by_field_name("condition").unwrap();
            let then = n.child_by_field_name("consequence").map(body).unwrap_or_default();
            let els = n.child_by_field_name("alternative").and_then(|e| e.named_child(0)).map(body).unwrap_or_default();
            Stmt::If { cond, then, els }
        }
        "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => match n.child_by_field_name("body") {
            Some(b) => Stmt::Loop { body_node: b, body: body(b) },
            None => Stmt::Other,
        },
        "try_statement" => Stmt::Try {
            body: n.child_by_field_name("body").map(body).unwrap_or_default(),
            catches: n.child_by_field_name("handler").map(|h| vec![(h, h.child_by_field_name("body").map(body).unwrap_or_default())]).unwrap_or_default(),
            finally: n.child_by_field_name("finalizer").and_then(|f| f.child_by_field_name("body")).map(body).unwrap_or_default(),
        },
        "statement_block" => Stmt::Block(body(n)),
        "break_statement" | "continue_statement" => jump(n, &["for_statement", "for_in_statement", "while_statement", "do_statement"], &["switch_statement"], FUNCTIONS),
        "lexical_declaration" | "variable_declaration" => {
            let keyword = n.child(0).map(|k| text(k, src).to_string()).unwrap_or_default();
            let mut names = vec![];
            let mut values = vec![];
            for d in named_children(n).into_iter().filter(|d| d.kind() == "variable_declarator") {
                if let Some(name) = d.child_by_field_name("name") {
                    bound_names(name, src, NAMES, &mut names);
                }
                values.extend(d.child_by_field_name("value"));
            }
            Stmt::Decl { keyword, names, values }
        }
        "expression_statement" => match n.named_child(0) {
            Some(e) => Stmt::Expr(e),
            None => Stmt::Other,
        },
        "function_declaration" | "generator_function_declaration" | "class_declaration" | "abstract_class_declaration" | "interface_declaration" | "type_alias_declaration" | "enum_declaration" => Stmt::Definition,
        _ => Stmt::Other,
    }
}

fn expr<'t>(n: Node<'t>, src: &str) -> Expr<'t> {
    match n.kind() {
        "call_expression" => match n.child_by_field_name("function") {
            Some(callee) => Expr::Call { callee, args: n.child_by_field_name("arguments").map(|a| named_children(a).into_iter().filter(|c| c.kind() != "comment").collect()).unwrap_or_default() },
            None => Expr::Other,
        },
        "ternary_expression" => Expr::Cond {
            cond: n.child_by_field_name("condition").unwrap(),
            then: n.child_by_field_name("consequence").unwrap(),
            els: n.child_by_field_name("alternative"),
            negated: false,
        },
        "binary_expression" => {
            let op = n.child_by_field_name("operator").map(|o| text(o, src)).unwrap_or("");
            match (op, n.child_by_field_name("left"), n.child_by_field_name("right")) {
                ("&&" | "||" | "??", Some(l), Some(r)) => Expr::Cond { cond: l, then: r, els: None, negated: op != "&&" },
                _ => Expr::Other,
            }
        }
        k if FUNCTIONS.contains(&k) => Expr::Function,
        _ => Expr::Other,
    }
}

fn imports(root: Node, src: &str) -> Vec<Import> {
    let mut out = vec![];
    for st in named_children(root) {
        if st.kind() != "import_statement" {
            continue;
        }
        let Some(source) = st.child_by_field_name("source") else { continue };
        let module = text(source, src).trim_matches(|c| c == '"' || c == '\'').to_string();
        // inside the quotes
        let module_at = (source.start_position().row, source.start_position().column + 1);
        let Some(clause) = named_children(st).into_iter().find(|c| c.kind() == "import_clause") else { continue };
        let mut add = |local: Node, name: &str| {
            out.push(Import { local: text(local, src).to_string(), module: module.clone(), name: name.to_string(), line: local.start_position().row, col: local.start_position().column, module_at, scope: None })
        };
        for c in named_children(clause) {
            match c.kind() {
                "identifier" => add(c, "default"),
                "namespace_import" => {
                    if let Some(id) = c.named_child(0) {
                        add(id, "*")
                    }
                }
                "named_imports" => {
                    for s in named_children(c).into_iter().filter(|s| s.kind() == "import_specifier") {
                        let name = s.child_by_field_name("name").unwrap();
                        let local = s.child_by_field_name("alias").unwrap_or(name);
                        add(local, text(name, src));
                    }
                }
                _ => {}
            }
        }
    }
    loaded(root, src, &mut out);
    out
}

/// Modules loaded by a call, anywhere: `const m = require("m")`, `const { a, b: c } = await import("m")`, or a bare
/// `require("m")`. Inside a function, the names it gives are known only there.
fn loaded(root: Node, src: &str, out: &mut Vec<Import>) {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        stack.extend(named_children(n));
        if n.kind() != "call_expression" {
            continue;
        }
        let Some(f) = n.child_by_field_name("function") else { continue };
        if !matches!(f.kind(), "import") && !(f.kind() == "identifier" && text(f, src) == "require") {
            continue;
        }
        let Some(arg) = n.child_by_field_name("arguments").and_then(|a| a.named_child(0)).filter(|a| a.kind() == "string") else { continue };
        let module = text(arg, src).trim_matches(|c| c == '"' || c == '\'').to_string();
        let module_at = (arg.start_position().row, arg.start_position().column + 1);
        let scope = std::iter::successors(n.parent(), |p| p.parent()).find(|p| FUNCTIONS.contains(&p.kind())).map(|p| (p.start_byte(), p.end_byte()));
        let mut v = n;
        while let Some(p) = v.parent().filter(|p| matches!(p.kind(), "await_expression" | "parenthesized_expression")) {
            v = p;
        }
        let target = v.parent().filter(|p| p.kind() == "variable_declarator" && p.child_by_field_name("value") == Some(v)).and_then(|d| d.child_by_field_name("name"));
        let mut add = |local: &str, name: &str, at: Node| {
            out.push(Import { local: local.to_string(), module: module.clone(), name: name.to_string(), line: at.start_position().row, col: at.start_position().column, module_at, scope })
        };
        match target {
            Some(t) if t.kind() == "identifier" => add(text(t, src), "*", t),
            Some(t) if t.kind() == "object_pattern" => {
                for p in named_children(t) {
                    match p.kind() {
                        "shorthand_property_identifier_pattern" => add(text(p, src), text(p, src), p),
                        "pair_pattern" => {
                            if let (Some(k), Some(v)) = (p.child_by_field_name("key"), p.child_by_field_name("value").filter(|v| v.kind() == "identifier")) {
                                add(text(v, src), text(k, src), v)
                            }
                        }
                        _ => {}
                    }
                }
            }
            // loaded for what it does, or kept another way: the module is still one the code needs
            _ => add("", "*", arg),
        }
    }
}

fn function_name<'t>(n: Node<'t>) -> Option<Node<'t>> {
    if let Some(name) = n.child_by_field_name("name") {
        return Some(name);
    }
    // `const f = () => …`, `{ f: () => … }`, `const f = wrap(() => …)`, a class's `f = () => …`: the nearest name it
    // is assigned to
    let mut p = n.parent();
    while let Some(x) = p {
        match x.kind() {
            "variable_declarator" => return x.child_by_field_name("name").filter(|n| n.kind() == "identifier"),
            "public_field_definition" => return x.child_by_field_name("name").filter(|n| n.kind() == "property_identifier"),
            "pair" => return x.child_by_field_name("key"),
            "parenthesized_expression" | "as_expression" | "satisfies_expression" | "arguments" | "call_expression" => p = x.parent(),
            _ => return None,
        }
    }
    None
}

fn function_body<'t>(n: Node<'t>) -> Option<Node<'t>> {
    n.child_by_field_name("body")
}

fn not_operand<'t>(n: Node<'t>, src: &str) -> Option<Node<'t>> {
    if n.kind() == "unary_expression" && n.child_by_field_name("operator").is_some_and(|o| text(o, src) == "!") { n.child_by_field_name("argument") } else { None }
}

fn discard<'t>(n: Node<'t>, src: &str) -> Option<Node<'t>> {
    if n.kind() == "unary_expression" && n.child_by_field_name("operator").is_some_and(|o| text(o, src) == "void") { n.child_by_field_name("argument") } else { None }
}

fn params(f: Node, src: &str) -> Vec<String> {
    if let Some(p) = f.child_by_field_name("parameter") {
        return vec![text(p, src).to_string()];
    }
    let Some(list) = f.child_by_field_name("parameters") else { return vec![] };
    named_children(list).into_iter().filter(|c| c.kind() != "comment").map(|c| text(c.child_by_field_name("pattern").unwrap_or(c), src).to_string()).collect()
}

/// The name to resolve for a type written as a base: `Base`, `Base<T>`, `ns.Base`.
fn type_name<'t>(t: Node<'t>) -> Option<Node<'t>> {
    match t.kind() {
        "generic_type" | "nested_type_identifier" => t.child_by_field_name("name"),
        "identifier" | "type_identifier" | "member_expression" => Some(t),
        _ => None,
    }
}

fn bases<'t>(class: Node<'t>) -> Vec<Node<'t>> {
    let mut out = vec![];
    for c in named_children(class) {
        // a class: `extends A implements B, C`; an interface: `extends A, B`
        let clauses = if c.kind() == "class_heritage" { named_children(c) } else { vec![c] };
        for clause in clauses {
            match clause.kind() {
                "extends_clause" => out.extend(field_children(clause, "value").into_iter().filter_map(type_name)),
                "implements_clause" | "extends_type_clause" => out.extend(named_children(clause).into_iter().filter_map(type_name)),
                _ => {}
            }
        }
    }
    out
}

fn members<'t>(class: Node<'t>) -> Vec<Node<'t>> {
    let Some(body) = class.child_by_field_name("body") else { return vec![] };
    named_children(body)
        .into_iter()
        .filter(|m| matches!(m.kind(), "method_definition" | "method_signature" | "abstract_method_signature" | "property_signature" | "public_field_definition"))
        .filter_map(|m| m.child_by_field_name("name"))
        .collect()
}

/// A member declared without a body, from its name: an interface's method (`save(x): void`, `save: (x) => void`) or an
/// abstract one. Calls to it run the versions the classes implementing it write.
fn abstract_member<'t>(name: Node<'t>) -> Option<Node<'t>> {
    let m = name.parent().filter(|m| m.child_by_field_name("name") == Some(name))?;
    match m.kind() {
        "method_signature" | "abstract_method_signature" => Some(m),
        "property_signature" => m.child_by_field_name("type").and_then(|t| t.named_child(0)).filter(|t| t.kind() == "function_type").map(|_| m),
        _ => None,
    }
}

/// `get size() { … }`, `set size(v) { … }`: the method runs when the property is read or set.
fn accessor(f: Node, _src: &str) -> bool {
    if f.kind() != "method_definition" {
        return false;
    }
    let mut c = f.walk();
    let found = f.children(&mut c).any(|k| !k.is_named() && (k.kind() == "get" || k.kind() == "set"));
    found
}

/// An installed package (`node_modules/<name>/…`, `node_modules/@scope/name/…`). TypeScript's own `lib.*.d.ts` files
/// and Node's types are the language and its platform.
fn package(p: &std::path::Path) -> Option<String> {
    let parts = parts(p);
    let i = parts.iter().rposition(|c| *c == "node_modules")?;
    let first = *parts.get(i + 1)?;
    let name = if first.starts_with('@') { format!("{first}/{}", parts.get(i + 2)?) } else { first.to_string() };
    if name == "typescript" && parts.last().is_some_and(|f| f.starts_with("lib.")) {
        return None;
    }
    match name.strip_prefix("@types/") {
        Some("node") => None,
        Some(t) => Some(t.to_string()),
        None => Some(name),
    }
}

macro_rules! ts_lang {
    ($name:ident, $id:literal, $exts:expr, $grammar:expr, $lsp:literal, $elements:expr) => {
        pub static $name: Lang = Lang {
            id: $id,
            exts: $exts,
            grammar: $grammar,
            server: "typescript",
            lsp_id: $lsp,
            line_comment: "//",
            package_sep: '/',
            functions: FUNCTIONS,
            calls: &[("call_expression", "function"), ("new_expression", "constructor")],
            elements: $elements,
            unwrap: &["parenthesized_expression", "non_null_expression", "await_expression", "as_expression", "satisfies_expression", "type_assertion"],
            unwrap_one: &[],
            member: &[("member_expression", "object", "property")],
            names: NAMES,
            strings: Strings { nodes: &["string", "template_string"], fragments: &["string_fragment", "escape_sequence"], interpolations: &["template_substitution"] },
            numbers: &["number"],
            markup_text: &["jsx_text"],
            truthy: &["true"],
            falsy: &["false", "null", "undefined", "\"\"", "''", "0"],
            iterate: &["map", "forEach", "flatMap", "filter", "find", "findIndex", "some", "every", "reduce"],
            promise: &["then", "finally", "catch"],
            follow: Follow {
                declarators: &[("variable_declarator", "name", "value"), ("public_field_definition", "name", "value")],
                destructure: Some(Destructure { pattern: "object_pattern", pair: ("pair_pattern", "key", "value"), shorthand: "shorthand_property_identifier_pattern" }),
                list: Some((&["array_pattern"], &["array"])),
                returns: "return_statement",
                object: Some(("object", "pair", "key", "value", "shorthand_property_identifier")),
                parameter: &["required_parameter", "optional_parameter", "formal_parameters"],
            },
            blocks: &["statement_block"],
            awaited: Some(("Promise<", ">")),
            not_operand,
            discard,
            params,
            stmt,
            expr,
            body,
            imports,
            function_name,
            function_body,
            reach: Reach {
                decorators: &["decorator"],
                classes: &["class_declaration", "abstract_class_declaration", "class", "interface_declaration"],
                bases,
                members,
                implicit: |name| name == "constructor",
                accessor,
                not_uses: &["import_statement", "export_clause"],
                exposes: &["export_statement"],
            },
            package,
            callable: &["=>", "Function"],
            named_arg: |_, _| None,
            abstract_member,
            signature: signature_after,
            // a `catch` handles every value
            caught: |_, _| None,
            caught_as: |clause, src| clause.child_by_field_name("parameter").filter(|p| p.kind() == "identifier").map(|p| text(p, src).to_string()),
            assigns: &[("assignment_expression", "left"), ("augmented_assignment_expression", "left")],
            own: Own::Keyword("this"),
            dynamic_import: Some("import"),
            typed,
        };
    };
}

/// `(c: Client)`, `const c: Client<T> = …`: the annotation's type name; `type Client = …`: that of what it stands for.
fn typed<'t>(name: Node<'t>) -> Option<Node<'t>> {
    let p = name.parent()?;
    let (field, annotated) = match p.kind() {
        "required_parameter" | "optional_parameter" => ("pattern", true),
        "variable_declarator" => ("name", true),
        "type_alias_declaration" => ("name", false),
        _ => return None,
    };
    if p.child_by_field_name(field) != Some(name) {
        return None;
    }
    let t = if annotated { p.child_by_field_name("type")?.named_child(0)? } else { p.child_by_field_name("value")? };
    let t = if t.kind() == "generic_type" { t.child_by_field_name("name")? } else { t };
    (t.kind() == "type_identifier").then_some(t)
}

ts_lang!(TYPESCRIPT, "typescript", &[".ts", ".mts", ".cts"], || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "typescript", &[]);
const JSX_ELEMENTS: &[(&str, &str)] = &[("jsx_self_closing_element", "name"), ("jsx_opening_element", "name")];
ts_lang!(TSX, "tsx", &[".tsx"], || tree_sitter_typescript::LANGUAGE_TSX.into(), "typescriptreact", JSX_ELEMENTS);
ts_lang!(JAVASCRIPT, "javascript", &[".js", ".jsx", ".mjs", ".cjs"], || tree_sitter_typescript::LANGUAGE_TSX.into(), "javascriptreact", JSX_ELEMENTS);
