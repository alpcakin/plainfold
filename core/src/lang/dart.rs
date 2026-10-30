// Dart (Flutter apps, Dart servers and command-line programs).
use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const NAMES: &[&str] = &["identifier", "type_identifier", "identifier_dollar_escaped"];
const FUNCTIONS: &[&str] = &["function_declaration", "method_declaration", "local_function_declaration", "function_expression", "getter_declaration", "setter_declaration"];
const CLASSES: &[&str] = &["class_declaration", "mixin_declaration", "enum_declaration", "extension_declaration", "extension_type_declaration"];
/// The heads of functions and members, which hold their names.
const SIGNATURES: &[&str] = &["function_signature", "getter_signature", "setter_signature", "operator_signature"];
const CONSTRUCTORS: &[&str] = &["constructor_signature", "factory_constructor_signature", "constant_constructor_signature", "redirecting_factory_constructor_signature"];

/// Statements, as a `case` of a `switch` holds them after its pattern.
const STATEMENTS: &[&str] = &[
    "block", "local_variable_declaration", "local_function_declaration", "for_statement", "while_statement", "do_statement", "switch_statement", "if_statement", "try_statement",
    "break_statement", "continue_statement", "return_statement", "yield_statement", "yield_each_statement", "expression_statement", "assert_statement", "labeled_statement",
    "rethrow_statement", "empty_statement", "pattern_assignment",
];

fn is_comment(n: &Node) -> bool {
    n.kind() == "comment"
}

fn kids<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    named_children(n).into_iter().filter(|c| !is_comment(c)).collect()
}

fn body<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    if n.kind() == "block" { kids(n) } else { vec![n] }
}

/// `final a = 1, b = f();` / `var (x, y) = pair();`: the names declared, the values given, and the keyword written.
fn declaration<'t>(n: Node<'t>, src: &str) -> Stmt<'t> {
    let mut keyword = vec![];
    let mut names = vec![];
    let mut values = vec![];
    for d in kids(n) {
        match d.kind() {
            "initialized_variable_definition" => {
                let mut c = d.walk();
                for k in d.children(&mut c).take_while(|k| k.is_named() || !matches!(k.kind(), "=" | ",")) {
                    if !k.is_named() && matches!(k.kind(), "final" | "var" | "const" | "late") {
                        keyword.push(text(k, src));
                    }
                }
                names.extend(d.child_by_field_name("name").map(|x| text(x, src).to_string()));
                // the value, then each cascade on it (`Command(f)..execute()`)
                values.extend(field_children(d, "value"));
                for more in kids(d).into_iter().filter(|k| k.kind() == "initialized_identifier") {
                    names.extend(more.child_by_field_name("name").map(|x| text(x, src).to_string()));
                    values.extend(field_children(more, "value"));
                }
            }
            "pattern_variable_declaration" => {
                let parts = kids(d);
                if let Some((value, pattern)) = parts.split_last() {
                    for p in pattern {
                        bound_names(*p, src, NAMES, &mut names);
                    }
                    values.push(*value);
                }
            }
            _ => {}
        }
    }
    Stmt::Decl { keyword: keyword.join(" "), names, values }
}

fn stmt<'t>(n: Node<'t>, src: &str) -> Stmt<'t> {
    match n.kind() {
        "return_statement" => Stmt::Return(kids(n).into_iter().next()),
        "rethrow_statement" => Stmt::Throw(None),
        "if_statement" => {
            let then = n.child_by_field_name("consequence");
            let alt = n.child_by_field_name("alternative");
            let Some(cond) = kids(n).into_iter().find(|c| Some(*c) != then && Some(*c) != alt) else { return Stmt::Other };
            // `else if` is an `if` statement of its own
            let els = match alt {
                Some(e) if e.kind() == "if_statement" => vec![e],
                Some(e) => body(e),
                None => vec![],
            };
            Stmt::If { cond, then: then.map(body).unwrap_or_default(), els }
        }
        "for_statement" | "while_statement" | "do_statement" => match n.child_by_field_name("body") {
            Some(b) => Stmt::Loop { body_node: b, body: body(b) },
            None => Stmt::Other,
        },
        // `try { … } on E catch (e) { … } catch (e) { … } finally { … }`: each clause is an `on` type and/or a
        // `catch`, then its block
        "try_statement" => {
            let main = n.child_by_field_name("body");
            let (mut catches, mut finally, mut clause) = (vec![], vec![], None);
            for c in kids(n).into_iter().filter(|c| Some(*c) != main) {
                match c.kind() {
                    "type" | "catch_clause" => clause = clause.or(Some(c)),
                    "block" => catches.push((clause.take().unwrap_or(c), body(c))),
                    "finally_clause" => finally = kids(c).into_iter().flat_map(body).collect(),
                    _ => {}
                }
            }
            Stmt::Try { body: main.map(body).unwrap_or_default(), catches, finally }
        }
        "block" => Stmt::Block(body(n)),
        // `switch` picks one case: each case is an alternative to the ones after it, as `if … else if …` is
        "switch_statement" => match n.child_by_field_name("body").and_then(|b| kids(b).into_iter().next()) {
            Some(first) => stmt(first, src),
            None => Stmt::Other,
        },
        "switch_statement_case" => {
            let parts = kids(n);
            let Some((pattern, rest)) = parts.split_first() else { return Stmt::Other };
            let statements = |c: Node<'t>| kids(c).into_iter().filter(|s| STATEMENTS.contains(&s.kind())).collect::<Vec<_>>();
            let mut next = n.next_named_sibling();
            while let Some(c) = next.filter(is_comment) {
                next = c.next_named_sibling();
            }
            let els = match next {
                Some(c) if c.kind() == "switch_statement_case" => vec![c],
                Some(c) if c.kind() == "switch_statement_default" => statements(c),
                _ => vec![],
            };
            Stmt::If { cond: *pattern, then: rest.iter().copied().filter(|s| STATEMENTS.contains(&s.kind())).collect(), els }
        }
        "switch_statement_default" => Stmt::Block(kids(n).into_iter().filter(|s| STATEMENTS.contains(&s.kind())).collect()),
        "labeled_statement" => Stmt::Block(kids(n).into_iter().filter(|c| c.kind() != "identifier").collect()),
        "local_variable_declaration" => declaration(n, src),
        "expression_statement" => {
            let parts = kids(n);
            match parts.as_slice() {
                [e] if e.kind() == "assignment_expression" => {
                    let mut names = vec![];
                    if let Some(l) = e.child_by_field_name("left") {
                        bound_names(l, src, NAMES, &mut names);
                    }
                    Stmt::Decl { keyword: String::new(), names, values: field_children(*e, "right") }
                }
                [e] if e.kind() == "throw_expression" => Stmt::Throw(e.child_by_field_name("value")),
                [e] => Stmt::Expr(*e),
                // `a..b()..c()`: the object, then each cascade on it
                [] => Stmt::Other,
                _ => Stmt::Eval(parts),
            }
        }
        "break_statement" | "continue_statement" => jump(n, &["for_statement", "while_statement", "do_statement"], &["switch_statement"], FUNCTIONS),
        "local_function_declaration" => Stmt::Definition,
        "assert_statement" => Stmt::Eval(kids(n).into_iter().flat_map(kids).collect()),
        "yield_statement" | "yield_each_statement" => Stmt::Eval(kids(n)),
        _ => Stmt::Other,
    }
}

fn arguments<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    n.child_by_field_name("arguments").map(kids).unwrap_or_default()
}

fn expr<'t>(n: Node<'t>, _src: &str) -> Expr<'t> {
    let call = |callee: Option<Node<'t>>| match callee {
        Some(callee) => Expr::Call { callee, args: arguments(n) },
        None => Expr::Other,
    };
    let two = |negated: bool| match kids(n).as_slice() {
        [a, b] => Expr::Cond { cond: *a, then: *b, els: None, negated },
        _ => Expr::Other,
    };
    match n.kind() {
        "call_expression" => call(n.child_by_field_name("function")),
        // `Widget(…)` without `new` is a call; `new X(…)`, `const X.named(…)` name the constructor apart
        "new_expression" | "const_object_expression" | "constructor_invocation" => call(n.child_by_field_name("constructor").or_else(|| n.child_by_field_name("type"))),
        // `..add(x)` on the object a cascade is written on
        "cascade_call_expression" => call(n.child_by_field_name("property").or_else(|| n.child_by_field_name("function"))),
        "conditional_expression" | "if_element" => {
            let (then, els) = (n.child_by_field_name("consequence"), n.child_by_field_name("alternative"));
            let cond = n.child_by_field_name("condition").or_else(|| kids(n).into_iter().find(|c| Some(*c) != then && Some(*c) != els));
            match (cond, then) {
                (Some(cond), Some(then)) => Expr::Cond { cond, then, els, negated: false },
                _ => Expr::Other,
            }
        }
        "logical_and_expression" => two(false),
        // the right side runs when the left is false, or null
        "logical_or_expression" | "if_null_expression" => two(true),
        "function_expression" => Expr::Function,
        _ => Expr::Other,
    }
}

/// `import 'package:http/http.dart' as http;`, `import '../x.dart' show A, B;`. A relative path is written with `./`
/// in front, as other languages write it.
fn imports(root: Node, src: &str) -> Vec<Import> {
    let mut out = vec![];
    for st in named_children(root).into_iter().filter(|s| s.kind() == "import_or_export") {
        let Some(spec) = named_children(st).into_iter().filter(|s| s.kind() == "library_import").flat_map(named_children).find(|s| s.kind() == "import_specification") else { continue };
        let Some(uri) = spec.child_by_field_name("uri") else { continue };
        let written = text(uri, src).trim_matches(|c| c == '\'' || c == '"');
        let module = if written.contains(':') || written.starts_with('.') || written.starts_with('/') { written.to_string() } else { format!("./{written}") };
        // where the path is written: asking there for a definition reaches the imported file
        let (line, col) = (uri.start_position().row, uri.start_position().column + 1);
        let mut add = |local: &str, name: &str| out.push(Import { local: local.to_string(), module: module.clone(), name: name.to_string(), line, col, module_at: (line, col), scope: None });
        let mut named = false;
        if let Some(alias) = spec.child_by_field_name("alias") {
            add(text(alias, src), "*");
            named = true;
        }
        for c in named_children(spec).into_iter().filter(|c| c.kind() == "combinator" && text(*c, src).starts_with("show")) {
            for id in named_children(c).into_iter().filter(|i| i.kind() == "identifier") {
                add(text(id, src), text(id, src));
                named = true;
            }
        }
        if !named {
            add("", "*");
        }
    }
    out
}

/// The name a function's or member's head gives it; a named constructor is known by its own name (`Repo.dev` → `dev`).
fn signature_name<'t>(sig: Node<'t>) -> Option<Node<'t>> {
    match sig.kind() {
        "method_signature" => named_children(sig).into_iter().find_map(signature_name),
        k if SIGNATURES.contains(&k) => sig.child_by_field_name("name"),
        k if CONSTRUCTORS.contains(&k) => field_children(sig, "name").into_iter().filter(|n| n.kind() == "identifier").last(),
        _ => None,
    }
}

fn function_name<'t>(n: Node<'t>) -> Option<Node<'t>> {
    match n.kind() {
        "function_declaration" | "method_declaration" | "getter_declaration" | "setter_declaration" => n.child_by_field_name("signature").and_then(signature_name),
        "local_function_declaration" => named_children(n).into_iter().find_map(signature_name),
        k if SIGNATURES.contains(&k) => signature_name(n),
        // `onPressed: () { … }`, `final f = () => …`, `f = wrap(() => …)`: the nearest name it is given
        "function_expression" => {
            let mut p = n.parent();
            while let Some(x) = p {
                match x.kind() {
                    "named_argument" => return named_children(x).into_iter().find(|c| c.kind() == "label").and_then(|l| l.named_child(0)),
                    "initialized_variable_definition" | "static_final_declaration" | "initialized_identifier" => return x.child_by_field_name("name"),
                    "assignment_expression" => return x.child_by_field_name("left").and_then(|l| l.named_child(0)).filter(|l| l.kind() == "identifier"),
                    "parenthesized_expression" | "arguments" | "call_expression" => p = x.parent(),
                    _ => return None,
                }
            }
            None
        }
        _ => n.child_by_field_name("name"),
    }
}

fn function_body<'t>(n: Node<'t>) -> Option<Node<'t>> {
    let b = match n.kind() {
        "local_function_declaration" => named_children(n).into_iter().find(|c| c.kind() == "function_body")?,
        _ => n.child_by_field_name("body").filter(|b| b.kind().ends_with("body"))?,
    };
    // a block, or the expression after `=>`; `async` and the arrow are not named
    kids(b).into_iter().next()
}

fn parameter_list<'t>(f: Node<'t>) -> Option<Node<'t>> {
    if let Some(p) = f.child_by_field_name("parameters") {
        return Some(p);
    }
    let sig = f.child_by_field_name("signature").or_else(|| named_children(f).into_iter().find(|c| SIGNATURES.contains(&c.kind())))?;
    let sig = if sig.kind() == "method_signature" { named_children(sig).into_iter().next()? } else { sig };
    sig.child_by_field_name("parameters")
}

/// The parameters in the order written: the positional ones, then those in `[…]` or `{…}`.
fn params(f: Node, src: &str) -> Vec<String> {
    fn walk(n: Node, src: &str, out: &mut Vec<String>) {
        for c in kids(n) {
            match c.kind() {
                "optional_formal_parameters" => walk(c, src, out),
                "formal_parameter" => {
                    // `String token`, `this.name`, `super.key`
                    let name = c.child_by_field_name("name").or_else(|| named_children(c).into_iter().flat_map(named_children).find(|x| x.kind() == "identifier"));
                    out.push(text(name.unwrap_or(c), src).to_string());
                }
                _ => {}
            }
        }
    }
    let mut out = vec![];
    if let Some(list) = parameter_list(f) {
        walk(list, src, &mut out);
    }
    out
}

fn bases<'t>(class: Node<'t>) -> Vec<Node<'t>> {
    let mut out = vec![];
    // `extends A with M`: the class, then the mixins; `implements B, C`; a mixin's `on A`
    if let Some(s) = class.child_by_field_name("superclass") {
        out.extend(s.child_by_field_name("type"));
        out.extend(kids(s).into_iter().filter(|m| m.kind() == "mixins").flat_map(kids).filter(|t| t.kind() == "type"));
    }
    if let Some(i) = class.child_by_field_name("interfaces") {
        out.extend(kids(i).into_iter().filter(|t| t.kind() == "type"));
    }
    if class.kind() == "mixin_declaration" {
        out.extend(kids(class).into_iter().filter(|t| t.kind() == "type"));
    }
    out
}

fn members<'t>(class: Node<'t>) -> Vec<Node<'t>> {
    let Some(body) = class.child_by_field_name("body") else { return vec![] };
    kids(body)
        .into_iter()
        .filter(|m| m.kind() == "class_member")
        .flat_map(kids)
        .filter_map(|m| match m.kind() {
            "method_declaration" => function_name(m),
            // declared without a body: an abstract method, getter or setter
            "declaration" => kids(m).into_iter().find(|s| SIGNATURES.contains(&s.kind())).and_then(signature_name),
            _ => None,
        })
        .collect()
}

/// A getter or a setter runs when the property is read or set.
fn accessor(f: Node, _src: &str) -> bool {
    match f.kind() {
        "getter_declaration" | "setter_declaration" => true,
        "method_declaration" => f.child_by_field_name("signature").is_some_and(|s| kids(s).iter().any(|k| k.kind() == "getter_signature" || k.kind() == "setter_signature")),
        _ => false,
    }
}

/// `Future<void> login();` in an abstract class: the head of a member with no body.
fn abstract_member<'t>(name: Node<'t>) -> Option<Node<'t>> {
    let sig = name.parent().filter(|s| SIGNATURES.contains(&s.kind()))?;
    sig.parent().filter(|d| d.kind() == "declaration").map(|_| sig)
}

/// The package a file outside the project belongs to: the nearest folder above its `lib` that holds a
/// `pubspec.yaml`, by the name written there (`.pub-cache/hosted/pub.dev/http-1.2.2/lib/…` → `http`, Flutter's own
/// `packages/flutter/lib/…` → `flutter`). Dart's own library (`dart-sdk/lib/…`) has none.
fn package(p: &Path) -> Option<String> {
    static NAMES: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> = LazyLock::new(Mutex::default);
    let mut dir = p.parent();
    while let Some(d) = dir {
        if d.file_name().is_some_and(|n| n == "lib")
            && let Some(root) = d.parent()
        {
            let name = NAMES
                .lock()
                .unwrap()
                .entry(root.to_path_buf())
                .or_insert_with(|| {
                    let pubspec = std::fs::read_to_string(root.join("pubspec.yaml")).ok()?;
                    pubspec.lines().find_map(|l| l.strip_prefix("name:")).map(|n| n.trim().trim_matches(|c| c == '\'' || c == '"').to_string())
                })
                .clone();
            if name.is_some() {
                return name;
            }
        }
        dir = d.parent();
    }
    None
}

/// A signature as the Dart analysis server shows it: types before names (`Future<Page> load(String token,
/// {int limit = 20})`, `void Function(int) callback`).
fn signature(sig: &str) -> (Vec<Param>, Option<String>) {
    let sig = sig.trim();
    // the parameter list: the first `(` outside brackets
    let mut depth = 0i32;
    let mut open = None;
    for (i, c) in sig.char_indices() {
        match c {
            '<' | '[' | '{' => depth += 1,
            '>' | ']' | '}' => depth -= 1,
            '(' if depth == 0 => {
                open = Some(i);
                break;
            }
            _ => {}
        }
    }
    let Some(open) = open else { return (vec![], None) };
    // the name is the word before it; what comes before the name is the return type
    let head = sig[..open].trim_end();
    let name_start = head.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '.')).map(|i| i + 1).unwrap_or(0);
    let before = head[..name_start].trim();
    let ret = before.split_whitespace().filter(|w| !matches!(*w, "static" | "external" | "factory" | "const" | "abstract")).collect::<Vec<_>>().join(" ");
    let ret = if ret.is_empty() { None } else { Some(ret) };
    let mut depth = 0i32;
    let mut close = sig.len();
    for (i, c) in sig[open..].char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                if depth == 0 {
                    close = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &sig[open + 1..close.max(open + 1)];
    // named (`{…}`) and optional (`[…]`) parameters are written in brackets of their own
    let params = top_level(inner, ',')
        .into_iter()
        .flat_map(|p| {
            let p = p.trim();
            match p.strip_prefix(['{', '[']) {
                Some(group) => top_level(group.trim_end().trim_end_matches(['}', ']']), ','),
                None => vec![p],
            }
        })
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            let p = top_level(p, '=').first().map(|x| x.trim()).unwrap_or(p);
            let p = p.strip_prefix("required ").unwrap_or(p).trim();
            // the name is the last word; a function type (`void Function(int) cb`) keeps its parentheses in the type
            match p.rfind(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == '?') {
                Some(i) => Param { name: p[i + 1..].trim_start_matches("this.").trim_start_matches("super.").to_string(), ty: p[..=i].trim().to_string() },
                None => Param { name: p.trim_start_matches("this.").trim_start_matches("super.").to_string(), ty: String::new() },
            }
        })
        .collect();
    (params, ret)
}

pub static DART: Lang = Lang {
    id: "dart",
    exts: &[".dart"],
    grammar: || tree_sitter_dart::LANGUAGE.into(),
    server: "dart",
    lsp_id: "dart",
    line_comment: "//",
    package_sep: '/',
    functions: FUNCTIONS,
    calls: &[("call_expression", "function"), ("new_expression", "type"), ("const_object_expression", "type"), ("constructor_invocation", "constructor"), ("cascade_call_expression", "property")],
    elements: &[],
    // `f<T>(…)`: the function the type is given to
    unwrap: &["parenthesized_expression", "await_expression", "null_assertion_expression", "type_cast_expression", "type", "assignable_expression", "instantiation_expression"],
    unwrap_one: &["unary_expression"],
    member: &[("member_expression", "object", "property"), ("null_aware_member_expression", "object", "property")],
    names: NAMES,
    strings: Strings {
        nodes: &[
            "string_literal",
            "string_literal_single_quotes",
            "string_literal_double_quotes",
            "string_literal_single_quotes_multiple",
            "string_literal_double_quotes_multiple",
        ],
        fragments: &["template_chars_single", "template_chars_double", "template_chars_single_single", "template_chars_double_single", "template_chars_raw_slash", "escape_sequence"],
        interpolations: &["template_substitution"],
    },
    numbers: &["decimal_integer_literal", "decimal_floating_point_literal", "hex_integer_literal"],
    markup_text: &[],
    truthy: &["true"],
    falsy: &["false", "null", "\"\"", "''", "0"],
    iterate: &["map", "forEach", "where", "expand", "any", "every", "firstWhere", "lastWhere", "singleWhere", "fold", "reduce"],
    promise: &["then", "whenComplete", "catchError"],
    follow: Follow {
        declarators: &[("initialized_variable_definition", "name", "value"), ("static_final_declaration", "name", "value"), ("initialized_identifier", "name", "value")],
        destructure: None,
        list: None,
        returns: "return_statement",
        object: None,
        parameter: &["formal_parameter", "formal_parameter_list", "optional_formal_parameters", "constructor_param", "super_formal_parameter"],
    },
    blocks: &["block"],
    awaited: Some(("Future<", ">")),
    not_operand: |n, _| {
        let k = kids(n);
        if n.kind() == "unary_expression" && k.first().is_some_and(|o| o.kind() == "negate_operator") { k.get(1).copied() } else { None }
    },
    discard: |_, _| None,
    params,
    stmt,
    expr,
    body,
    imports,
    function_name,
    function_body,
    reach: Reach {
        decorators: &["annotation"],
        classes: CLASSES,
        bases,
        members,
        // what Dart itself calls: printing, equality and hashing, a missing member
        implicit: |name| matches!(name, "toString" | "hashCode" | "noSuchMethod"),
        accessor,
        not_uses: &["import_or_export", "part_directive", "part_of_directive", "library_name"],
        exposes: &[],
    },
    package,
    callable: &["Function"],
    named_arg: |a, _| {
        if a.kind() != "named_argument" {
            return None;
        }
        let parts = kids(a);
        let name = parts.iter().find(|c| c.kind() == "label").and_then(|l| l.named_child(0))?;
        Some((name, *parts.last().filter(|v| v.kind() != "label")?))
    },
    abstract_member,
    signature,
    // `on E catch (e)` / `on E` handles `E`; a `catch` alone, or `on Object`, handles anything
    caught: |clause, src| {
        if clause.kind() != "type" {
            return None;
        }
        let t = last_part(text(clause, src));
        if t == "Object" { None } else { Some(vec![t]) }
    },
    // `on E catch (e)`: the `catch` follows the type
    caught_as: |clause, src| {
        let c = if clause.kind() == "type" { clause.next_named_sibling().filter(|c| c.kind() == "catch_clause")? } else { clause };
        let name = kids(c).into_iter().find(|k| k.kind() == "identifier")?;
        Some(text(name, src).to_string())
    },
    assigns: &[("assignment_expression", "left")],
    own: Own::Keyword("this"),
    // `void f(Client c)`
    dynamic_import: None,
    typed: |name| {
        let p = name.parent().filter(|p| p.kind() == "formal_parameter" && p.child_by_field_name("name") == Some(name))?;
        kids(kids(p).into_iter().find(|t| t.kind() == "type")?).into_iter().next().filter(|t| t.kind() == "type_identifier")
    },
};

#[cfg(test)]
mod tests {
    use super::{params, signature};

    #[test]
    fn parameters_in_the_order_written() {
        let src = "Future<void> load(int a, String b, {bool? c, int d = 1}) async {}\nvoid save(int a, [int? b]) {}\n";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_dart::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let mut c = tree.root_node().walk();
        let functions: Vec<_> = tree.root_node().named_children(&mut c).filter(|n| n.kind() == "function_declaration").collect();
        assert_eq!(params(functions[0], src), ["a", "b", "c", "d"]);
        assert_eq!(params(functions[1], src), ["a", "b"]);
    }

    #[test]
    fn signatures_as_the_analysis_server_shows_them() {
        let (p, r) = signature("Future<ItemsPage> loadItems(String token, {int limit = 20, String? category})");
        assert_eq!(r.as_deref(), Some("Future<ItemsPage>"));
        assert_eq!(p.iter().map(|x| (x.name.as_str(), x.ty.as_str())).collect::<Vec<_>>(), [("token", "String"), ("limit", "int"), ("category", "String?")]);
        let (p, r) = signature("void addPostFrameCallback(void Function(Duration) callback, {String debugLabel = 'callback'})");
        assert_eq!(r.as_deref(), Some("void"));
        assert_eq!(p[0].name, "callback");
        assert_eq!(p[0].ty, "void Function(Duration)");
        let (p, _) = signature("Repo(this.name, {required super.key})");
        assert_eq!(p.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["name", "key"]);
    }
}
