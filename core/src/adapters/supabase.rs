// Supabase adapter. Handles: calls on a Supabase client: table reads and writes (`from("t")` / `table("t")` with
// `select / insert / update / upsert / delete`), database functions (`rpc("f")`), auth (`auth.…`), edge functions
// (`functions.invoke("f")`) and the files of Storage (`storage.from("bucket").upload(…)`: a participant of its own,
// what it does to the files said as reading, writing or deleting; `getPublicUrl` asks nothing of the server). A
// client is one made with `createClient` (or the browser / server variants) from `@supabase/…`, or any chain of calls
// the language server places in a Supabase package: in Python (supabase-py) the client is usually handed around as a
// typed parameter, so where it was made is not in sight. Where the package cannot be found (an edge function's `jsr:`
// import), a client is also one made from that import, or a value of a type written as one.
use super::{Adapter, is_imported};
use crate::lang::Expr;
use crate::model::*;
use crate::source::{At, one_line};
use crate::story::{Builder, Chain, Ctx};
use tree_sitter::Node;

pub struct Supabase;

const CLIENTS: &[&str] = &["createClient", "createBrowserClient", "createServerClient"];
const DB_OPS: &[&str] = &["select", "insert", "update", "upsert", "delete"];
/// supabase-py and the packages it is made of
const PY_PACKAGES: &[&str] = &["supabase", "postgrest", "gotrue", "supabase_auth", "storage3", "supafunc", "realtime"];
/// where a client's own API starts in a chain (`self.db.table(…)`)
const API: &[&str] = &["from", "from_", "table", "rpc", "auth", "functions", "storage"];
/// what only reads the result or runs the query, so says nothing more about it
const PLAIN: &[&str] = &["select", "single", "maybeSingle", "maybe_single", "execute"];
/// what a Storage call does to the files of its bucket; one not listed reads
const STORAGE_WRITES: &[&str] = &["upload", "update", "move", "copy", "uploadToSignedUrl", "createSignedUploadUrl", "createBucket", "updateBucket"];
const STORAGE_DELETES: &[&str] = &["remove", "emptyBucket", "deleteBucket"];

impl Supabase {
    /// Where the client a name refers to was made ("lib/supabase.ts:4"), if it is a Supabase client.
    fn client<'w>(&self, b: &Builder<'w>, id: At<'w>) -> Option<String> {
        let loc = b.w.res.definition(id.file, id.node).into_iter().next()?;
        let file = b.w.p.file_index(&loc.file)?;
        let f = &b.w.p.files[file];
        let n = f.node_at(loc.line as usize, loc.col)?;
        let (decl, value) = f.lang.declared(n)?;
        let init = f.lang.unwrap(value?);
        let Expr::Call { callee, .. } = (f.lang.expr)(init, &f.text) else { return None };
        // a module may be written with its registry and version (`jsr:@supabase/supabase-js@2`)
        if !CLIENTS.iter().any(|c| is_imported(f, callee, &|m| crate::resolve::package_of(m, '/').starts_with("@supabase/"), c)) {
            return None;
        }
        Some(format!("{}:{}", f.rel, decl.start_position().row + 1))
    }

    fn string_arg(&self, b: &Builder, file: usize, args: &Option<Vec<Node>>) -> Option<String> {
        let a = *args.as_ref()?.first()?;
        // a name written there or held in a constant (`const BUCKET = "files"`): a value known at runtime is not one
        // we can name
        let s = crate::story::routes::url_of(b.w.p, b.w.res, file, a)?;
        if s.contains("{…}") { None } else { Some(s) }
    }
}

impl Adapter for Supabase {
    fn name(&self) -> &'static str {
        "Supabase"
    }
    fn handles(&self, package: &str) -> bool {
        package.starts_with("@supabase/") || PY_PACKAGES.contains(&package)
    }

    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let f = b.file(e);
        // where the client's API starts: right after a client made in sight, else at the first of its names
        let k = chain.segs.iter().position(|s| API.contains(&s.name.as_str()))?;
        let made = if k == 0 && f.lang.is_name(chain.root.kind()) { self.client(b, b.at(e, chain.root)) } else { None };
        let (client_at, segs) = match made {
            Some(at) => (format!("Supabase client from {at}"), &chain.segs[..]),
            None => {
                let (package, _) = b.package_of(e, chain)?;
                if !self.handles(&package) {
                    return None;
                }
                (format!("Supabase client (the language server places the call in the {package} package)"), &chain.segs[k..])
            }
        };
        let place = b.place(e);
        let detail = one_line(b.text(e), 0);
        let first = &segs[0];
        let all = |b: &Builder<'w>| segs.iter().map(|s| b.seg_text(e.file, s)).collect::<String>()[1..].to_string();
        let mut effect = None;
        let mut table = None;
        let names_table = matches!(first.name.as_str(), "from" | "from_" | "table") && first.args.as_ref().is_some_and(|a| !a.is_empty());
        let (to, label, how) = if names_table {
            let to = b.part(Participant { id: "db".into(), name: "database".into(), kind: PartKind::Database, group: Group::Supabase, sub: None, at: None });
            // a table named by a literal is known; one named by a value is only known at runtime, but what is done to it is
            let t = self.string_arg(b, e.file, &first.args);
            match &t {
                Some(t) => {
                    let db = b.part_mut("db").unwrap();
                    let mut tables: Vec<String> = db.sub.as_deref().unwrap_or("").split(", ").filter(|s| !s.is_empty()).map(String::from).collect();
                    if !tables.contains(t) {
                        tables.push(t.clone());
                    }
                    db.sub = Some(tables.join(", "));
                }
                None => {
                    let arg = first.args.as_ref().and_then(|a| a.first()).map(|a| one_line(b.text(b.at(e, *a)), 30)).unwrap_or_default();
                    b.gap(format!("{}: the table is named by {arg}, only known at runtime", one_line(&b.seg_text(e.file, first)[1..], 40)), place.clone());
                }
            }
            let op = segs[1..].iter().find(|s| DB_OPS.contains(&s.name.as_str()));
            effect = Some(match op.map(|o| o.name.as_str()) {
                None | Some("select") => Effect::Reads,
                Some("delete") => Effect::Deletes,
                _ => Effect::Writes,
            });
            let op_text = op.map(|o| format!(".{}({})", o.name, o.args.as_deref().unwrap_or(&[]).iter().map(|a| b.short(b.at(e, *a))).collect::<Vec<_>>().join(", "))).unwrap_or_default();
            let more = segs[1..].iter().any(|s| !op.is_some_and(|o| std::ptr::eq(o, s)) && !PLAIN.contains(&s.name.as_str()));
            let label = format!("{}{op_text}{}", &b.seg_text(e.file, first)[1..], if more { " …" } else { "" });
            let how = match &t {
                Some(t) => format!("{client_at}, table \"{t}\""),
                None => format!("{client_at}, a table named at runtime"),
            };
            table = t;
            (to, label, how)
        } else if let Some(fname) = (first.name == "rpc").then(|| self.string_arg(b, e.file, &first.args)).flatten() {
            let to = b.part(Participant { id: "db".into(), name: "database".into(), kind: PartKind::Database, group: Group::Supabase, sub: None, at: None });
            table = Some(fname.clone());
            (to, format!("rpc(\"{fname}\")"), format!("{client_at}, database function \"{fname}\""))
        } else if first.name == "storage" {
            let from = segs.get(1).filter(|x| x.name == "from");
            let bucket = from.and_then(|x| self.string_arg(b, e.file, &x.args));
            let op = segs.get(if from.is_some() { 2 } else { 1 }).map(|x| x.name.as_str()).unwrap_or("");
            // a file's public address is made by the client itself: nothing is asked of the server
            if op == "getPublicUrl" {
                let mut out: Vec<Item> = segs.iter().flat_map(|x| x.args.clone().unwrap_or_default()).flat_map(|a| b.expr(b.at(e, a), ctx)).collect();
                out.push(Item::Own { id: b.id(), at: ctx.this.clone(), kind: SelfKind::Local, label: one_line(&all(b), 56), detail: Some("makes the file's public address; nothing is asked of Supabase".into()), place: place.clone() });
                return Some(out);
            }
            let to = b.part(Participant { id: "storage".into(), name: "Supabase Storage".into(), kind: PartKind::Service, group: Group::Supabase, sub: None, at: None });
            match &bucket {
                Some(name) => {
                    let st = b.part_mut("storage").unwrap();
                    let mut buckets: Vec<String> = st.sub.as_deref().unwrap_or("").split(", ").filter(|s| !s.is_empty()).map(String::from).collect();
                    if !buckets.contains(name) {
                        buckets.push(name.clone());
                    }
                    st.sub = Some(buckets.join(", "));
                }
                None if from.is_some() => {
                    let arg = from.and_then(|x| x.args.as_ref()?.first().copied()).map(|a| one_line(b.text(b.at(e, a)), 30)).unwrap_or_default();
                    b.gap(format!("{}: the bucket is named by {arg}, only known at runtime", one_line(&all(b), 40)), place.clone());
                }
                None => {}
            }
            effect = Some(if STORAGE_WRITES.contains(&op) {
                Effect::Writes
            } else if STORAGE_DELETES.contains(&op) {
                Effect::Deletes
            } else {
                Effect::Reads
            });
            let how = match &bucket {
                Some(name) => format!("{client_at}, storage bucket \"{name}\""),
                None => format!("{client_at}, storage"),
            };
            (to, all(b), how)
        } else if first.name == "auth" {
            let to = b.part(Participant { id: "auth".into(), name: "Supabase Auth".into(), kind: PartKind::Auth, group: Group::Supabase, sub: None, at: None });
            (to, all(b), format!("{client_at}, auth"))
        } else if let Some(fname) = (first.name == "functions" && segs.get(1).is_some_and(|s| s.name == "invoke")).then(|| self.string_arg(b, e.file, &segs[1].args)).flatten() {
            let to = b.part(Participant { id: format!("edge:{fname}"), name: fname.clone(), kind: PartKind::Service, group: Group::Supabase, sub: Some("edge function".into()), at: None });
            (to, format!("functions.invoke(\"{fname}\")"), format!("{client_at}, edge function by name"))
        } else {
            let to = b.part(Participant { id: "supabase".into(), name: "Supabase".into(), kind: PartKind::Service, group: Group::Supabase, sub: None, at: None });
            let label = all(b);
            b.gap(format!("{}: this Supabase call is not understood yet", one_line(&label, 40)), place.clone());
            (to, label, client_at.clone())
        };
        let mut out: Vec<Item> = vec![];
        // what the client comes from, when it is made here (`get_db().table(…)`)
        if !f.lang.is_name(chain.root.kind()) {
            out.extend(b.expr(b.at(e, chain.root), ctx));
        }
        for s in &chain.segs {
            for a in s.args.as_deref().unwrap_or(&[]) {
                let items = b.expr(b.at(e, *a), ctx);
                out.extend(items);
            }
        }
        let back = b.receiver(e).unwrap_or("result".into());
        let (unused, into) = b.result_use(e);
        out.push(Item::Call(Call {
            id: b.id(),
            from: ctx.this.clone(),
            to: to.clone(),
            label: one_line(&label, 60),
            detail: Some(detail),
            link: Link { kind: LinkKind::Exact, how },
            effect,
            at: place.clone(),
            target: None,
            body: None,
            note: None,
            opens: None,
            story: None,
            table,
            touches: None,
            unused,
            into,
            ends: vec![],
            changes: vec![],
        }));
        out.push(Item::Return { id: b.id(), from: to, to: ctx.this.clone(), label: back, detail: None, at: place, throws: false, early: false, passes: None });
        Some(out)
    }
}
