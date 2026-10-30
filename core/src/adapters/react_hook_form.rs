// react-hook-form adapter. Handles: the submit handler a form makes with `handleSubmit(onValid)`, taken from what
// `useForm()` or `useFormContext()` returns (`const { handleSubmit } = useForm()`, `form.handleSubmit(…)`). What
// runs when the form is sent is the function it is given. A `handleSubmit` from anywhere else (a project's own
// hook around `useForm`) is not followed. Its other calls stay plain calls into the package.
use super::{Adapter, is_imported};
use crate::lang::Expr;
use crate::source::SourceFile;
use tree_sitter::Node;

pub struct ReactHookForm;

const PACKAGE: &str = "react-hook-form";
const SUBMIT: &str = "handleSubmit";

/// The value `name` is declared with in a scope around `at` (`const name = v`), or, when it is taken from an object
/// (`const { key: name } = v`), the value and that key.
fn declared<'p>(f: &'p SourceFile, at: Node<'p>, name: &str) -> Option<(Node<'p>, Option<String>)> {
    let mut scope = at.parent();
    while let Some(s) = scope {
        let mut c = s.walk();
        for st in s.named_children(&mut c).filter(|n| matches!(n.kind(), "lexical_declaration" | "variable_declaration")) {
            let mut d = st.walk();
            for decl in st.named_children(&mut d).filter(|n| n.kind() == "variable_declarator") {
                let (Some(target), Some(value)) = (decl.child_by_field_name("name"), decl.child_by_field_name("value")) else { continue };
                if target.kind() == "identifier" && f.text_of(target) == name {
                    return Some((value, None));
                }
                if target.kind() == "object_pattern" {
                    let mut e = target.walk();
                    for p in target.named_children(&mut e) {
                        let key = match p.kind() {
                            "shorthand_property_identifier_pattern" if f.text_of(p) == name => Some(name.to_string()),
                            "pair_pattern" if p.child_by_field_name("value").is_some_and(|v| v.kind() == "identifier" && f.text_of(v) == name) => {
                                p.child_by_field_name("key").map(|k| f.text_of(k).to_string())
                            }
                            _ => None,
                        };
                        if key.is_some() {
                            return Some((value, key));
                        }
                    }
                }
            }
        }
        scope = s.parent();
    }
    None
}

/// `useForm(…)` or `useFormContext()`, imported from the package.
fn makes_form(f: &SourceFile, v: Node) -> bool {
    let Expr::Call { callee, .. } = (f.lang.expr)(f.lang.unwrap(v), &f.text) else { return false };
    ["useForm", "useFormContext"].iter().any(|n| is_imported(f, callee, &|m| m == PACKAGE, n))
}

impl Adapter for ReactHookForm {
    fn name(&self) -> &'static str {
        "react-hook-form"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn applies(&self, package: &str) -> bool {
        package == PACKAGE
    }
    fn wrapped<'p>(&self, file: &'p SourceFile, call: Node<'p>) -> Option<Node<'p>> {
        let l = file.lang;
        let Expr::Call { callee, args } = (l.expr)(call, &file.text) else { return None };
        let c = l.unwrap(callee);
        let made = if l.is_name(c.kind()) {
            declared(file, c, file.text_of(c)).is_some_and(|(v, key)| key.as_deref() == Some(SUBMIT) && makes_form(file, v))
        } else if let Some((obj, prop)) = l.member_fields(c.kind()) {
            let (Some(o), Some(p)) = (c.child_by_field_name(obj), c.child_by_field_name(prop)) else { return None };
            file.text_of(p) == SUBMIT && l.is_name(o.kind()) && declared(file, o, file.text_of(o)).is_some_and(|(v, key)| key.is_none() && makes_form(file, v))
        } else {
            false
        };
        if made { args.first().copied() } else { None }
    }
}
