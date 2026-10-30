// What tree-sitter node types mean, per language. This is the only place that knows a language's syntax;
// the walker, the resolver and the story builder only see the shapes below.
// Adding a language = one module here (node types + a few small accessors) and a language server in `lsp::servers`.
use tree_sitter::{Language, Node};

mod dart;
mod python;
mod typescript;

/// A statement, reduced to the shapes a story needs.
pub enum Stmt<'t> {
    Return(Option<Node<'t>>),
    /// Ends the function by throwing this value; none: the one being handled (`raise`, `rethrow`).
    Throw(Option<Node<'t>>),
    /// `els` holds the statements of the else part; an `else if` / `elif` is itself an `If` statement there.
    If { cond: Node<'t>, then: Vec<Node<'t>>, els: Vec<Node<'t>> },
    /// `body_node` marks where the loop's head (the part written before its body) ends.
    Loop { body_node: Node<'t>, body: Vec<Node<'t>> },
    Try { body: Vec<Node<'t>>, catches: Vec<(Node<'t>, Vec<Node<'t>>)>, finally: Vec<Node<'t>> },
    Block(Vec<Node<'t>>),
    /// A declaration or assignment: the names it binds, the values evaluated, and the keyword as written (`const`, or empty).
    Decl { keyword: String, names: Vec<String>, values: Vec<Node<'t>> },
    /// A statement that evaluates these expressions and does nothing else a story shows (`assert`): what they call
    /// is part of the story.
    Eval(Vec<Node<'t>>),
    Expr(Node<'t>),
    /// `break` / `continue` in the loop it is written in: this round of the loop ends here.
    Jump,
    /// A nested function or class: declared here, not run here.
    Definition,
    Other,
}

/// An expression, reduced to the shapes a story needs. Everything else is walked through its children.
pub enum Expr<'t> {
    Call { callee: Node<'t>, args: Vec<Node<'t>> },
    /// `a ? b : c`, `a && b`, `a || b`. `negated` means the right side runs when the condition is false (`||`, `??`, `or`).
    Cond { cond: Node<'t>, then: Node<'t>, els: Option<Node<'t>>, negated: bool },
    /// A function value (arrow, lambda, function expression): its body runs later, not here.
    Function,
    Other,
}

/// `import { name as local } from "module"` → { local, module, name }. `name` is "default" or "*" for those forms.
#[derive(Clone, Debug)]
pub struct Import {
    pub local: String,
    pub module: String,
    pub name: String,
    pub line: usize, // 0-based row of the imported name
    pub col: usize,  // byte column of the imported name
    /// where the module is written (row, byte column): asking there for a definition reaches the module's file
    pub module_at: (usize, usize),
    /// the bytes of the function or class it is written in, where the name it gives is known only there; None: the
    /// whole file
    pub scope: Option<(usize, usize)>,
}

impl Import {
    /// The module is written as a path (`./store`, `/assets/logo.svg`): a file of the project, never a package.
    pub fn is_path(&self) -> bool {
        self.module.starts_with('.') || self.module.starts_with('/')
    }
}

pub struct Strings {
    pub nodes: &'static [&'static str],
    pub fragments: &'static [&'static str],
    pub interpolations: &'static [&'static str],
}

pub struct Follow {
    /// `name = value`: node type, name field, value field. A language may write it several ways (a local, a field,
    /// a constant).
    pub declarators: &'static [(&'static str, &'static str, &'static str)],
    /// `{ key: name } = value`, `{ key } = value`
    pub destructure: Option<Destructure>,
    /// `[a, b] = value`, `a, b = value`: the node types of such a pattern, and those of a list written out (`[x, y]`,
    /// `x, y`) whose items it takes by place
    pub list: Option<(&'static [&'static str], &'static [&'static str])>,
    pub returns: &'static str,
    /// object literal: node type, pair type, key field, value field, shorthand type
    pub object: Option<(&'static str, &'static str, &'static str, &'static str, &'static str)>,
    pub parameter: &'static [&'static str],
}

pub struct Destructure {
    pub pattern: &'static str,
    pub pair: (&'static str, &'static str, &'static str), // pair type, key field, value field
    pub shorthand: &'static str,
}

/// How code reaches a function other than by calling it by name, and what the language runs by itself. Used to tell
/// which functions something outside the project starts.
pub struct Reach {
    /// a decorator: what is written in it is applied to the definition it stands on
    pub decorators: &'static [&'static str],
    /// classes and interfaces: a method can be reached through one of them that it extends
    pub classes: &'static [&'static str],
    /// the base classes and interfaces a class names, as expressions to resolve
    pub bases: for<'t> fn(Node<'t>) -> Vec<Node<'t>>,
    /// the name nodes of the members a class or interface declares
    pub members: for<'t> fn(Node<'t>) -> Vec<Node<'t>>,
    /// a method the language calls by itself, never by its name in code (a constructor, `__eq__`)
    pub implicit: fn(&str) -> bool,
    /// a function that runs when it is read like a value (a getter, a `@property`)
    pub accessor: fn(Node, &str) -> bool,
    /// a name inside one of these is not a use of what it names (an import, a list of exported names)
    pub not_uses: &'static [&'static str],
    /// a name written directly in one of these is made visible to other files, not handed to anything that runs it
    /// (`export default save`): top-level code that does only that does no work
    pub exposes: &'static [&'static str],
}

pub struct Lang {
    pub id: &'static str,
    pub exts: &'static [&'static str],
    pub grammar: fn() -> Language,
    /// which language server answers for this language, and the LSP language id
    pub server: &'static str,
    pub lsp_id: &'static str,
    pub line_comment: &'static str,
    /// what separates a package from the rest of a module name (`pkg/sub`, `pkg.sub`)
    pub package_sep: char,
    /// bodies that run when called
    pub functions: &'static [&'static str],
    /// a call: node type → field holding what is called
    pub calls: &'static [(&'static str, &'static str)],
    /// things that are not calls in the syntax but invoke something (a component used as an element)
    pub elements: &'static [(&'static str, &'static str)],
    pub unwrap: &'static [&'static str],
    /// looked through only when they hold a single expression: Dart's `unary_expression` is `!x` as well as the
    /// node around `await x`
    pub unwrap_one: &'static [&'static str],
    /// member access: node type → (object field, property field)
    pub member: &'static [(&'static str, &'static str, &'static str)],
    pub names: &'static [&'static str],
    pub strings: Strings,
    /// number literals
    pub numbers: &'static [&'static str],
    /// text written in markup and shown as it is (`<p>Loading…</p>`)
    pub markup_text: &'static [&'static str],
    /// literal spellings of values that are always true / always false in a condition
    pub truthy: &'static [&'static str],
    pub falsy: &'static [&'static str],
    /// methods that take a callback and run it once per element (`xs.map(x => …)`)
    pub iterate: &'static [&'static str],
    /// methods that attach a handler to a pending result (`p.then(…)`); the last one listed handles failure
    pub promise: &'static [&'static str],
    pub follow: Follow,
    /// node types of a statement block (a body that is not a single expression)
    pub blocks: &'static [&'static str],
    /// `!x` / `not x`: the operand
    pub not_operand: for<'t> fn(Node<'t>, &str) -> Option<Node<'t>>,
    /// an expression that throws its operand's value away (`void x`): the operand
    pub discard: for<'t> fn(Node<'t>, &str) -> Option<Node<'t>>,
    /// parameter names of a function, as written
    pub params: fn(Node, &str) -> Vec<String>,
    /// a pending result's type and what it resolves to when awaited (`Promise<T>` → `T`), if the language has one
    pub awaited: Option<(&'static str, &'static str)>,
    pub stmt: for<'t> fn(Node<'t>, &str) -> Stmt<'t>,
    pub expr: for<'t> fn(Node<'t>, &str) -> Expr<'t>,
    /// statements of a body (a block, or a single statement)
    pub body: for<'t> fn(Node<'t>) -> Vec<Node<'t>>,
    pub imports: fn(Node, &str) -> Vec<Import>,
    /// the name a function is known by: its own name, or the variable / property / key it is assigned to
    pub function_name: for<'t> fn(Node<'t>) -> Option<Node<'t>>,
    /// the body node of a function
    pub function_body: for<'t> fn(Node<'t>) -> Option<Node<'t>>,
    pub reach: Reach,
    /// the installed package a location outside the project belongs to (`supabase`, `@scope/name`); None when it is
    /// the language's own library or its platform (`len`, `JSON.parse`, Node's `path`)
    pub package: fn(&std::path::Path) -> Option<String>,
    /// how the language server writes a type that is a function, in a signature (`(x) -> T`, `() => void`)
    pub callable: &'static [&'static str],
    /// an argument given by name (`target=job`, `onPressed: save`): its name and its value
    pub named_arg: for<'t> fn(Node<'t>, &str) -> Option<(Node<'t>, Node<'t>)>,
    /// a member declared without a body (an abstract method, an interface's method), from the name node a definition
    /// points at: calls to it run the versions the classes extending its class write
    pub abstract_member: for<'t> fn(Node<'t>) -> Option<Node<'t>>,
    /// a signature as the language server shows it on hover: its parameters, and its return type
    pub signature: fn(&str) -> (Vec<Param>, Option<String>),
    /// the types a `catch` clause handles, as written (the last part of a dotted name); None: every value
    pub caught: fn(Node, &str) -> Option<Vec<String>>,
    /// the name a `catch` clause gives what it caught (`as e`, `catch (e)`)
    pub caught_as: fn(Node, &str) -> Option<String>,
    /// assignments, with the field that holds what is assigned to (`a.b = c`, `a[k] += 1`)
    pub assigns: &'static [(&'static str, &'static str)],
    /// how a method names its own object
    pub own: Own,
    /// what a call is made on when it loads a module while the code runs (`import("./x")`): the call gives the module
    pub dynamic_import: Option<&'static str>,
    /// the name of the type written for a name where it is declared (a parameter's or a variable's annotation), or of
    /// what a type alias stands for, from the name node; None when that type is not one plain name (or a generic of
    /// one): a member of a namespace, a union, a type made from others
    pub typed: for<'t> fn(Node<'t>) -> Option<Node<'t>>,
}

/// How a method names its own object: as its first parameter (Python's `self`), or by a keyword's node (`this`). A
/// field written without its object (Dart) is known by where the language server places it.
pub enum Own {
    FirstParam,
    Keyword(&'static str),
}

/// The last part of a dotted name, as written (`errors.NotFound` → `NotFound`).
pub fn last_part(name: &str) -> String {
    name.rsplit('.').next().unwrap_or(name).trim().to_string()
}

impl Follow {
    /// The name and value fields of a node type that declares a name with a value.
    pub fn declarator(&self, kind: &str) -> Option<(&'static str, &'static str)> {
        self.declarators.iter().find(|(k, _, _)| *k == kind).map(|(_, n, v)| (*n, *v))
    }
    /// Whether the node type declares a name with a value.
    pub fn declares(&self, kind: &str) -> bool {
        self.declarators.iter().any(|(k, _, _)| *k == kind)
    }
}

impl Lang {
    pub fn is_function(&self, kind: &str) -> bool {
        self.functions.contains(&kind)
    }
    pub fn call_field(&self, kind: &str) -> Option<&'static str> {
        self.calls.iter().find(|(k, _)| *k == kind).map(|(_, f)| *f)
    }
    pub fn element_field(&self, kind: &str) -> Option<&'static str> {
        self.elements.iter().find(|(k, _)| *k == kind).map(|(_, f)| *f)
    }
    pub fn member_fields(&self, kind: &str) -> Option<(&'static str, &'static str)> {
        self.member.iter().find(|(k, _, _)| *k == kind).map(|(_, o, p)| (*o, *p))
    }
    pub fn is_name(&self, kind: &str) -> bool {
        self.names.contains(&kind)
    }
    /// Looks through parentheses, `await`, type assertions and the like.
    pub fn unwrap<'t>(&self, mut n: Node<'t>) -> Node<'t> {
        // a comment written inside the parentheses (`return ( // note`) is not what they hold
        let inner = |n: Node<'t>| {
            let mut c = n.walk();
            n.named_children(&mut c).filter(|x| !x.kind().ends_with("comment")).collect::<Vec<_>>()
        };
        while self.unwrap.contains(&n.kind()) || (self.unwrap_one.contains(&n.kind()) && inner(n).len() == 1) {
            match inner(n).first() {
                Some(c) => n = *c,
                None => break,
            }
        }
        n
    }
    /// The declaration a name is declared in with a value (`name = value`), and that value.
    pub fn declared<'t>(&self, name: Node<'t>) -> Option<(Node<'t>, Option<Node<'t>>)> {
        let p = name.parent()?;
        let (n, v) = self.follow.declarator(p.kind())?;
        (p.child_by_field_name(n) == Some(name)).then(|| (p, p.child_by_field_name(v)))
    }
    /// The function a name node names, where the name is written in the function's head (`def f`, or inside a
    /// signature node: `void f()`).
    pub fn function_of<'t>(&self, name: Node<'t>) -> Option<Node<'t>> {
        let mut n = name.parent();
        for _ in 0..3 {
            let x = n?;
            if self.is_function(x.kind()) {
                return ((self.function_name)(x) == Some(name)).then_some(x);
            }
            n = x.parent();
        }
        None
    }
    /// An argument's value, without the name it is given by.
    pub fn arg_value<'t>(&self, a: Node<'t>, src: &str) -> Node<'t> {
        (self.named_arg)(a, src).map(|(_, v)| v).unwrap_or(a)
    }
}

pub static LANGS: &[&Lang] = &[&typescript::TYPESCRIPT, &typescript::TSX, &typescript::JAVASCRIPT, &python::PYTHON, &dart::DART];

pub fn lang_for(path: &std::path::Path) -> Option<&'static Lang> {
    let ext = path.extension()?.to_str()?;
    let ext = format!(".{ext}");
    LANGS.iter().copied().find(|l| l.exts.contains(&ext.as_str()))
}

// ---------- helpers shared by the language modules ----------

pub(crate) fn named_children<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}

pub(crate) fn field_children<'t>(n: Node<'t>, field: &str) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.children_by_field_name(field, &mut c).collect()
}

/// `break` or `continue` (`n`): a jump in the loop it is written in, up to its function. A `break` that a `switch`
/// holds first ends a case, not a round of a loop.
pub(crate) fn jump<'t>(n: Node<'t>, loops: &[&str], switches: &[&str], functions: &[&str]) -> Stmt<'t> {
    let breaks = n.kind().starts_with("break");
    let mut p = n.parent();
    while let Some(x) = p {
        if functions.contains(&x.kind()) || (breaks && switches.contains(&x.kind())) {
            return Stmt::Other;
        }
        if loops.contains(&x.kind()) {
            return Stmt::Jump;
        }
        p = x.parent();
    }
    Stmt::Other
}

/// The parts of a path, as text.
pub(crate) fn parts(p: &std::path::Path) -> Vec<&str> {
    p.components().map(|c| c.as_os_str().to_str().unwrap_or("")).collect()
}

pub(crate) fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.byte_range()]
}

/// Identifiers bound by a name or a destructuring pattern.
pub(crate) fn bound_names(n: Node, src: &str, names: &[&str], out: &mut Vec<String>) {
    if names.contains(&n.kind()) || n.kind().ends_with("identifier_pattern") {
        out.push(text(n, src).to_string());
        return;
    }
    for c in named_children(n) {
        // in `{ key: name }` only the value side is bound; in `name = default` only the left
        if let Some(v) = c.child_by_field_name("value").filter(|_| c.kind().contains("pair")) {
            bound_names(v, src, names, out);
        } else if let Some(l) = c.child_by_field_name("left").filter(|_| c.kind().contains("assignment_pattern")) {
            bound_names(l, src, names, out);
        } else {
            bound_names(c, src, names, out);
        }
    }
}

/// A parameter as a language server's signature shows it: its name (with `*`, `**` or `...` when it takes the rest)
/// and its type, without a default value.
#[derive(Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: String,
}

/// A signature written with its types after the names (`name: type`, `-> type`): TypeScript and Python.
pub(crate) fn signature_after(signature: &str) -> (Vec<Param>, Option<String>) {
    (parameters(signature), return_type(signature))
}

/// Splits at the commas (or another separator) that are not inside brackets.
pub(crate) fn top_level(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut from) = (vec![], 0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '>' if !s[..i].ends_with('-') && !s[..i].ends_with('=') => depth -= 1,
            // `=` of an arrow (`=>`) or a comparison is not a default value's
            '=' if sep == '=' && (s[i + 1..].starts_with(['>', '=']) || s[..i].ends_with(['=', '!', '<', '>'])) => {}
            c if c == sep && depth == 0 => {
                out.push(&s[from..i]);
                from = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[from..]);
    out
}

/// The parameters of a signature a language server shows in a hover (`(method) def submit(fn: (…) -> T, /,
/// *args: …) -> Future[T]`, `(method) A.on(event: string, listener: () => void): void`). Markers that are not
/// parameters (`/`, `*`) are left out.
pub fn parameters(signature: &str) -> Vec<Param> {
    let mut signature = signature.trim_start();
    if let Some((label, rest)) = signature.strip_prefix('(').and_then(|r| r.split_once(')')) {
        if label.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '_') {
            signature = rest;
        }
    }
    let Some(start) = signature.find('(') else { return vec![] };
    let inner = &signature[start + 1..];
    let mut depth = 1i32;
    let mut end = inner.len();
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    top_level(&inner[..end], ',')
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "/" && *p != "*")
        .map(|p| {
            let mut parts = top_level(p, ':').into_iter();
            let name = parts.next().unwrap_or("").trim().trim_end_matches('?').to_string();
            let ty = parts.collect::<Vec<_>>().join(":");
            let ty = top_level(&ty, '=').first().map(|t| t.trim().to_string()).unwrap_or_default();
            Param { name, ty }
        })
        .collect()
}

/// The parameter an argument goes to: by its name, else by its position (the rest go to a parameter that takes them).
pub fn param_for<'a>(params: &'a [Param], position: usize, name: Option<&str>) -> Option<&'a Param> {
    let rest = |p: &&Param, two: bool| if two { p.name.starts_with("**") } else { (p.name.starts_with('*') && !p.name.starts_with("**")) || p.name.starts_with("...") };
    match name {
        Some(n) => params.iter().find(|p| p.name == n).or_else(|| params.iter().find(|p| rest(p, true))),
        None => params.iter().take_while(|p| !rest(p, false) && !rest(p, true)).nth(position).or_else(|| params.iter().find(|p| rest(p, false))),
    }
}

/// The return type in a signature as a language server shows it on hover: whatever follows the parameter list,
/// after the `:` / `->` / `=>` that introduces it (`function f(a: string): boolean` → `boolean`,
/// `def f(a: int) -> str` → `str`, `const f: (a) => Promise<X>` → `Promise<X>`).
pub fn return_type(signature: &str) -> Option<String> {
    // servers label what the name is first: "(method) …", "(function) …"
    let mut signature = signature.trim_start();
    if let Some((label, rest)) = signature.strip_prefix('(').and_then(|r| r.split_once(')')) {
        if label.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '_') {
            signature = rest;
        }
    }
    let start = signature.find('(')?;
    let mut depth = 0i32;
    let mut end = None;
    for (i, c) in signature[start..].char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let rest = signature[end?..].trim_start();
    let rest = rest.strip_prefix("=>").or_else(|| rest.strip_prefix("->")).or_else(|| rest.strip_prefix(':'))?;
    let t = rest.lines().next()?.trim().trim_end_matches([';', ':']).trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::{param_for, parameters, return_type};

    #[test]
    fn return_types_from_hover_signatures() {
        assert_eq!(return_type("function isValid(value: string, kind: string): boolean").as_deref(), Some("boolean"));
        assert_eq!(return_type("const load: (id: string) => Promise<Item[]>").as_deref(), Some("Promise<Item[]>"));
        assert_eq!(return_type("(method) Store.get<T>(key: string, opts?: { a: (x: number) => void }): T | undefined").as_deref(), Some("T | undefined"));
        assert_eq!(return_type("(function) def total(items: list[int]) -> int").as_deref(), Some("int"));
        assert_eq!(return_type("def f(\n    a: int,\n    b: str\n) -> dict[str, int]").as_deref(), Some("dict[str, int]"));
        assert_eq!(return_type("const value: number"), None);
    }

    #[test]
    fn parameters_from_hover_signatures() {
        let ty = |sig: &str, at: usize, name: Option<&str>| param_for(&parameters(sig), at, name).map(|p| p.ty.clone());
        let submit = "(method) def submit(\n    fn: (**_P@submit) -> _T@submit,\n    /,\n    *args: _P@submit.args\n) -> Future[_T@submit]";
        assert_eq!(ty(submit, 0, None).as_deref(), Some("(**_P@submit) -> _T@submit"));
        assert_eq!(ty(submit, 2, None).as_deref(), Some("_P@submit.args"));
        let thread = "(class) def Thread(group: None = None, target: ((...) -> object) | None = None, name: str | None = None)";
        assert_eq!(ty(thread, 0, Some("target")).as_deref(), Some("((...) -> object) | None"));
        assert_eq!(ty("(method) def setdefault(key: str, default: _VT, /) -> _VT", 1, None).as_deref(), Some("_VT"));
        let on = "(method) Emitter.on<K extends string>(event: K, listener: (e: Map<K, number>) => void, opts?: { once: boolean }): this";
        assert_eq!(ty(on, 1, None).as_deref(), Some("(e: Map<K, number>) => void"));
        assert_eq!(ty(on, 2, None).as_deref(), Some("{ once: boolean }"));
        assert_eq!(ty("(function) add_role: Unknown", 0, None), None);
    }

    #[test]
    fn a_comment_in_parentheses_is_not_what_they_hold() {
        let l = &super::typescript::TYPESCRIPT;
        let mut p = tree_sitter::Parser::new();
        p.set_language(&(l.grammar)()).unwrap();
        let src = "const v = ( // a note\n  save(x)\n);\n";
        let tree = p.parse(src, None).unwrap();
        let mut stack = vec![tree.root_node()];
        let mut paren = None;
        while let Some(n) = stack.pop() {
            if n.kind() == "parenthesized_expression" {
                paren = Some(n);
                break;
            }
            stack.extend((0..n.named_child_count()).filter_map(|i| n.named_child(i as u32)));
        }
        let inner = l.unwrap(paren.expect("parentheses"));
        assert_eq!(inner.kind(), "call_expression");
        assert_eq!(&src[inner.byte_range()], "save(x)");
    }

    #[test]
    fn imports_say_where_their_module_is_written() {
        // each imported name, and the text written where its module is said to be
        let modules = |l: &super::Lang, src: &str| -> Vec<(String, String)> {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&(l.grammar)()).unwrap();
            let tree = p.parse(src, None).unwrap();
            let lines: Vec<&str> = src.lines().collect();
            (l.imports)(tree.root_node(), src).into_iter().map(|i| (i.local, lines[i.module_at.0][i.module_at.1..].chars().take(i.module.len()).collect())).collect()
        };
        let pairs = |v: &[(&str, &str)]| v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>();
        let py = modules(&super::python::PYTHON, "from store import save as keep, load\nimport store.items as items\nimport json\n");
        assert_eq!(py, pairs(&[("keep", "store"), ("load", "store"), ("items", "store.items"), ("json", "json")]));
        let ts = modules(&super::typescript::TYPESCRIPT, "import save, { load as read } from \"./store\";\nimport * as items from '@app/items';\n");
        assert_eq!(ts, pairs(&[("save", "./store"), ("read", "./store"), ("items", "@app/items")]));
    }

    #[test]
    fn a_python_import_inside_a_function_is_known_only_there() {
        let src = "import json
try:
    import fast
except ImportError:
    pass

def load():
    from store import items
    return items

class Box:
    import os
";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&(super::python::PYTHON.grammar)()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let scoped: Vec<(String, Option<&str>)> = (super::python::PYTHON.imports)(tree.root_node(), src).into_iter().map(|i| (i.local, i.scope.map(|(a, b)| src[a..b].lines().next().unwrap_or("")))).collect();
        let want = [("json", None), ("fast", None), ("items", Some("def load():")), ("os", Some("class Box:"))];
        assert_eq!(scoped, want.iter().map(|(a, b)| (a.to_string(), *b)).collect::<Vec<_>>());
    }

    #[test]
    fn a_break_in_a_switch_ends_a_case_not_a_round() {
        // for each `break` / `continue`, in order: whether it is a jump in a loop
        let jumps = |l: &super::Lang, src: &str| -> Vec<bool> {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&(l.grammar)()).unwrap();
            let tree = p.parse(src, None).unwrap();
            let mut all = vec![];
            let mut stack = vec![tree.root_node()];
            while let Some(n) = stack.pop() {
                if n.kind() == "break_statement" || n.kind() == "continue_statement" {
                    all.push(n);
                }
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c));
            }
            all.sort_by_key(|n| n.start_byte());
            all.into_iter().map(|n| matches!((l.stmt)(n, src), super::Stmt::Jump)).collect()
        };
        let ts = "function f(xs) { for (const x of xs) { switch (x) { case 1: break; } if (!x) continue; break; } }";
        assert_eq!(jumps(&super::typescript::TYPESCRIPT, ts), [false, true, true]);
        let py = "def f(xs):\n    for x in xs:\n        if not x:\n            continue\n        break\n";
        assert_eq!(jumps(&super::python::PYTHON, py), [true, true]);
        let dart = "void f(List<int> xs) {\n  for (final x in xs) {\n    switch (x) {\n      case 1:\n        break;\n    }\n    if (x == 0) continue;\n  }\n}\n";
        assert_eq!(jumps(&super::dart::DART, dart), [false, true]);
    }

    #[test]
    fn throws_and_what_catches_handle() {
        // for each statement that throws, the value thrown (or "-" for the one being handled); for each catch clause,
        // the types it handles ("*" for anything) and the name it gives what it caught ("-" for none)
        let read = |l: &super::Lang, src: &str, clause: &str| -> (Vec<String>, Vec<String>) {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&(l.grammar)()).unwrap();
            let tree = p.parse(src, None).unwrap();
            let (mut thrown, mut caught) = (vec![], vec![]);
            let mut stack = vec![tree.root_node()];
            let mut all = vec![];
            while let Some(n) = stack.pop() {
                all.push(n);
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c));
            }
            all.sort_by_key(|n| n.start_byte());
            for n in all {
                if let super::Stmt::Throw(v) = (l.stmt)(n, src) {
                    thrown.push(v.map(|v| src[v.byte_range()].to_string()).unwrap_or("-".into()));
                }
                if n.kind() == clause && n.parent().is_some_and(|p| p.kind() == "try_statement") {
                    let name = (l.caught_as)(n, src).unwrap_or("-".into());
                    caught.push(format!("{} {name}", (l.caught)(n, src).map(|t| t.join(",")).unwrap_or("*".into())));
                }
            }
            (thrown, caught)
        };
        let strs = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let py = "try:\n    f()\nexcept (A, m.B) as e:\n    raise C(1) from e\nexcept D, E:\n    raise\nexcept Exception:\n    pass\nexcept:\n    pass\n";
        assert_eq!(read(&super::python::PYTHON, py, "except_clause"), (strs(&["C(1)", "-"]), strs(&["A,B e", "D,E -", "* -", "* -"])));
        let ts = "try { f(); } catch (e) { throw new Denied(\"no\"); }";
        assert_eq!(read(&super::typescript::TYPESCRIPT, ts, "catch_clause"), (strs(&["new Denied(\"no\")"]), strs(&["* e"])));
        let dart = "void f() {\n  try { g(); } on m.A catch (e) { throw B(); } on Object { rethrow; }\n}\n";
        assert_eq!(read(&super::dart::DART, dart, "type"), (strs(&["B()", "-"]), strs(&["A e", "* -"])));
    }
}
