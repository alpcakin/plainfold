// SWR adapter. Handles: `useSWR(key, fetcher)` (from `swr`), whose `fetcher` runs later, when the data is first needed
// and again when it is out of date, given the key (a key written as a list gives its items). It is a frame of what runs
// later, like a query of TanStack Query, so a request the fetcher makes to an address built from the key is an
// arrow to the endpoint that answers it. Not yet: `useSWRMutation`, `useSWRInfinite`, a fetcher set once for the
// whole app (`<SWRConfig value={{ fetcher }}>`), and a key that is a function. Its other calls stay plain calls into
// the package.
use super::{Adapter, Later, is_imported};
use crate::lang::Expr;
use crate::source::{At, one_line};
use crate::story::{Builder, World};
use tree_sitter::Node;

pub struct Swr;

impl Adapter for Swr {
    fn name(&self) -> &'static str {
        "SWR"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["typescript"]
    }
    fn applies(&self, package: &str) -> bool {
        package == "swr"
    }

    fn runs<'w>(&self, w: &World<'w>) -> Vec<At<'w>> {
        // the fetchers handed to `useSWR`: it runs them, so they are no entry points
        let mut out = vec![];
        for (i, f) in w.p.files.iter().enumerate().filter(|(_, f)| f.imports.iter().any(|m| m.module == "swr")) {
            let l = f.lang;
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c).collect::<Vec<Node>>());
                let Expr::Call { callee, args } = (l.expr)(n, &f.text) else { continue };
                if !is_imported(f, callee, &|m| m == "swr", "default") {
                    continue;
                }
                if let Some(run) = args.get(1).map(|a| l.unwrap(*a)).filter(|r| !l.is_function(r.kind()))
                    && let crate::resolve::Target::Own { func, .. } = w.res.target(i, run, w.adapters)
                {
                    out.push(func);
                }
            }
        }
        out
    }

    fn later<'w>(&self, b: &Builder<'w>, call: At<'w>) -> Option<Later<'w>> {
        let f = b.file(call);
        let l = f.lang;
        let Expr::Call { callee, args } = (l.expr)(call.node, &f.text) else { return None };
        if !is_imported(f, callee, &|m| m == "swr", "default") {
            return None;
        }
        let key = l.unwrap(*args.first()?);
        let run = l.unwrap(*args.get(1)?);
        let func = if l.is_function(run.kind()) { b.at(call, run) } else { b.project_decl(b.at(call, run))? };
        // a key written as a list is the arguments the fetcher is given; a key that is a function is not followed
        let given: Vec<At<'w>> = if key.kind() == "array" {
            let mut c = key.walk();
            key.named_children(&mut c).map(|k| b.at(call, l.unwrap(k))).collect()
        } else if l.is_function(key.kind()) {
            vec![]
        } else {
            vec![b.at(call, key)]
        };
        Some(Later { name: "useSWR".into(), func, deps: Some(one_line(f.text_of(key), 0)), when: "when the data is first needed, and again when it is out of date".into(), given })
    }
}
