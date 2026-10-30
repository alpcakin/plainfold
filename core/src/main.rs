// plainfold core, command line for development.
//   plainfold-core extract <project> <model.json>   stories of a project, as the view reads them
//   plainfold-core function <project> <name> <model.json>   one story starting at a named function (no adapter needed)
//   plainfold-core resolve <project>                how many call sites the language servers resolve
//   plainfold-core entries <project>                functions something outside the project starts, and how
//   plainfold-core tree <file>                      the syntax tree of one file
//   plainfold-core needs <project>                  the languages a project uses and the state of their servers
//   plainfold-core install <project>                downloads the servers it lacks into the store
//   plainfold-core watch <project>                  prints which source files changed since it started
//   plainfold-core diff <old model.json> <new model.json> [--json]   what changed between two readings, story by story
//   plainfold-core uses <model.json> [words]        what the stories reach, by how many; with words, where each match is used
//   plainfold-core again <project> <first.json> <second.json> <command…>   reads, runs the command (an edit), reads
//                                                   again with the language servers kept running from the first read
//   plainfold-core english <model.json> <english.json> [--keep <dir>] [--show <n>]   the steps in plain English, from
//                                                   the model OPENAI_API_KEY reaches; --show prints request n and sends nothing
// Options: --servers <dir> (language server executables), --tsserver <dir>, --python <interpreter>,
//          --store <dir> (downloaded language servers; also PLAINFOLD_STORE)
use plainfold_core::{adapters, lang, model, resolve, servers, source, story};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut setup = servers::Setup::default();
    let mut take = |flag: &str| -> Option<PathBuf> {
        let i = args.iter().position(|a| a == flag)?;
        args.remove(i);
        Some(PathBuf::from(args.remove(i)))
    };
    setup.servers = take("--servers").or_else(|| std::env::var_os("PLAINFOLD_SERVERS").map(PathBuf::from));
    setup.tsserver = take("--tsserver").or_else(|| std::env::var_os("PLAINFOLD_TSSERVER").map(PathBuf::from));
    setup.store = take("--store").or_else(|| std::env::var_os("PLAINFOLD_STORE").map(PathBuf::from));
    // servers run in the project folder, so paths given here must not be relative
    setup.python = take("--python").map(|p| std::path::absolute(&p).unwrap_or(p));
    setup.servers = setup.servers.map(|p| std::path::absolute(&p).unwrap_or(p));
    setup.tsserver = setup.tsserver.map(|p| std::path::absolute(&p).unwrap_or(p));
    setup.store = setup.store.map(|p| std::path::absolute(&p).unwrap_or(p));
    match args.first().map(String::as_str) {
        Some("extract") => extract(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]), &setup),
        Some("function") => function(&PathBuf::from(&args[1]), &args[2], &PathBuf::from(&args[3]), &setup),
        Some("resolve") => resolve_report(&PathBuf::from(&args[1]), &setup),
        Some("entries") => entries(&PathBuf::from(&args[1]), &setup),
        Some("needs") => {
            for n in plainfold_core::needs(&PathBuf::from(&args[1]), &setup) {
                println!("{}: {} · {}", n.kind, n.state, n.detail);
            }
        }
        Some("install") => {
            for n in plainfold_core::needs(&PathBuf::from(&args[1]), &setup).into_iter().filter(|n| n.state == "download") {
                println!("downloading the {} ({})", n.label, n.detail);
                let last = std::cell::Cell::new(0u64);
                let r = plainfold_core::install(&n.kind, &setup, &|p| {
                    if let plainfold_core::Progress::Downloading { done, .. } = p
                        && done >= last.get() + 4_000_000
                    {
                        last.set(done);
                        println!("  {:.1} MB", done as f64 / 1e6);
                    }
                });
                println!("  {}", r.map(|_| "done".to_string()).unwrap_or_else(|e| format!("failed: {e}")));
            }
        }
        Some("watch") => {
            let root = PathBuf::from(&args[1]);
            let t0 = Instant::now();
            let since = plainfold_core::changes::hashes(&root);
            println!("{} source files hashed in {} ms; watching (ctrl-c stops)", since.len(), t0.elapsed().as_millis());
            let _w = plainfold_core::changes::watch(&root, since, |changes| {
                println!("{}", if changes.is_empty() { "no changes".to_string() } else { changes.iter().map(|c| format!("{:?} {}", c.how, c.file)).collect::<Vec<_>>().join(", ") });
            })
            .expect("watch");
            loop {
                std::thread::park();
            }
        }
        Some("diff") => print_diff(&args[1], &args[2], args.iter().any(|a| a == "--json")),
        Some("uses") => print_uses(&args[1], args.get(2).map(String::as_str)),
        Some("english") => english(&args[1..]),
        Some("again") => again(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]), &PathBuf::from(&args[3]), &args[4..], &setup),
        Some("tree") => {
            let p = PathBuf::from(&args[1]);
            let l = lang::lang_for(&p).expect("no grammar for this file");
            let text = std::fs::read_to_string(&p).unwrap();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&(l.grammar)()).unwrap();
            println!("{}", parser.parse(&text, None).unwrap().root_node().to_sexp());
        }
        _ => eprintln!("usage: plainfold-core extract <project> <model.json> | needs <project> | install <project> | resolve <project> | tree <file>"),
    }
}

fn resolve_report(root: &PathBuf, setup: &servers::Setup) {
    let t0 = Instant::now();
    let project = source::Project::load(root).expect("project");
    let t_load = t0.elapsed();
    let res = resolve::Resolver::start(&project, setup);
    let t_ready = t0.elapsed();
    let mut names = vec![];
    for (i, f) in project.files.iter().enumerate() {
        for n in resolve::call_names(f) {
            names.push((i, n));
        }
    }
    res.prefetch(&names);
    let t_resolve = t0.elapsed();
    let (mut own, mut outside, mut none) = (0, 0, 0);
    for (i, n) in &names {
        match res.definition(*i, *n).first() {
            Some(l) if project.is_own(&l.file) => own += 1,
            Some(_) => outside += 1,
            None => none += 1,
        }
    }
    let missing = res.missing_modules();
    println!("{}: {} files, {} call sites", project.name, project.files.len(), names.len());
    println!("  resolved to own code {own}, outside the project {outside}, unresolved {none}");
    for (k, s) in &res.servers {
        println!("  {k}: {} with {}; ready ({})", s.client.name, s.with, s.readiness);
    }
    for (kind, why) in &res.failed {
        println!("  no {kind} server: {why}");
    }
    for (kind, why) in res.ended() {
        println!("  the {kind} server stopped while reading, so these counts are incomplete: {why}");
    }
    if !missing.is_empty() {
        println!("  not installed: {}", missing.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", "));
    }
    println!("  time: load {} ms, servers ready {} ms, resolve {} ms", t_load.as_millis(), (t_ready - t_load).as_millis(), (t_resolve - t_ready).as_millis());
    res.stop();
}

fn entries(root: &PathBuf, setup: &servers::Setup) {
    let project = source::Project::load(root).expect("project");
    let adapters = adapters::Adapters::for_project(&project);
    let res = resolve::Resolver::start(&project, setup);
    let world = story::World::new(&project, &res, &adapters);
    let t0 = Instant::now();
    let found = story::entries::find(&world, &Default::default());
    let list = &found.entries;
    let mut counts = std::collections::BTreeMap::new();
    let mut last = usize::MAX;
    for e in list {
        let f = &project.files[e.decl.file];
        if e.decl.file != last {
            println!("{}", f.rel);
            last = e.decl.file;
        }
        let how = match e.how.at() {
            Some(at) => format!("{} {}", e.how.kind(), source::one_line(project.files[at.file].text_of(at.node), 70)),
            None => e.how.kind().to_string(),
        };
        let ext = if e.extends.is_empty() { String::new() } else { format!(" · extends {} (outside)", e.extends.join(", ")) };
        println!("  {:>4}  {}  · {how}{ext}", e.decl.node.start_position().row + 1, e.name);
        *counts.entry(e.how.kind()).or_insert(0) += 1;
    }
    for (i, list) in &found.files {
        println!("top-level code of {}: {} statements", project.files[*i].rel, list.len());
    }
    println!("{} entry points in {} files ({} ms): {counts:?}", list.len(), project.files.len(), t0.elapsed().as_millis());
    res.stop();
}

fn extract(root: &PathBuf, out: &PathBuf, setup: &servers::Setup) {
    let x = plainfold_core::extract(root, setup, &|_| {}, &|_| false).expect("project");
    let model = &x.model;
    std::fs::create_dir_all(out.parent().unwrap_or(std::path::Path::new("."))).ok();
    std::fs::write(out, serde_json::to_string_pretty(model).unwrap()).expect("write model");
    println!(
        "{}: {} stories ({} ms: servers ready in {} ms) · adapters: {}",
        model.name,
        model.stories.len(),
        model.stats.ms,
        x.ready_ms,
        x.adapters.join(", ")
    );
    for n in &model.setup {
        println!("  note: {n}");
    }
    for s in &model.stories {
        let count = count(&s.items);
        println!("  {}: {} participants, {count} items, {} signals, {} gaps", s.title, s.participants.len(), s.signals.len(), s.gaps.len());
    }
}

/// Two reads of a project with a command run between them, the second with the servers the first kept running.
fn again(root: &PathBuf, first: &PathBuf, second: &PathBuf, command: &[String], setup: &servers::Setup) {
    let say = |x: &plainfold_core::Extracted| {
        let m = &x.model;
        println!("{}: {} stories ({} ms: servers ready in {} ms)", m.name, m.stories.len(), m.stats.ms, x.ready_ms);
        for s in &x.servers {
            println!("  {}: {}", s.name, s.readiness);
        }
    };
    let (x, kept) = plainfold_core::read(root, setup, &|_| {}, &|_| false, None).expect("project");
    std::fs::write(first, serde_json::to_string_pretty(&x.model).unwrap()).expect("write model");
    say(&x);
    if let Some((cmd, rest)) = command.split_first() {
        let ok = std::process::Command::new(cmd).args(rest).current_dir(root).status().is_ok_and(|s| s.success());
        println!("ran {}: {}", command.join(" "), if ok { "ok" } else { "failed" });
    }
    for h in kept.held() {
        println!("  kept {}: {} MB", h.name, h.bytes.unwrap_or(0) / 1_000_000);
    }
    let (x, _stopped_at_the_end) = plainfold_core::read(root, setup, &|_| {}, &|_| false, Some(kept)).expect("project");
    std::fs::write(second, serde_json::to_string_pretty(&x.model).unwrap()).expect("write model");
    say(&x);
}

fn count(items: &[model::Item]) -> usize {
    items
        .iter()
        .map(|i| {
            1 + match i {
                model::Item::Frame { items, els, .. } => count(items) + count(els.as_deref().unwrap_or(&[])),
                model::Item::Call(c) => count(c.body.as_deref().unwrap_or(&[])),
                _ => 0,
            }
        })
        .sum()
}

fn function(root: &PathBuf, name: &str, out: &PathBuf, setup: &servers::Setup) {
    let project = source::Project::load(root).expect("project");
    let adapters = adapters::Adapters::for_project(&project);
    let res = resolve::Resolver::start(&project, setup);
    let world = story::World::new(&project, &res, &adapters);
    let mut found = None;
    'files: for (i, f) in project.files.iter().enumerate() {
        let mut stack = vec![f.tree.root_node()];
        while let Some(n) = stack.pop() {
            if f.lang.is_function(n.kind()) && (f.lang.function_name)(n).is_some_and(|x| f.text_of(x) == name) {
                found = Some(source::At { file: i, node: n });
                break 'files;
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
    }
    let Some(decl) = found else {
        eprintln!("no function named {name}");
        res.stop();
        return;
    };
    let mut b = story::Builder::new(&world);
    let s = b.function_story(decl, None);
    let files = b.files.iter().filter_map(|f| project.files.iter().find(|x| &x.rel == f).map(|x| (f.clone(), x.text.clone()))).collect();
    let mut uses = std::mem::take(&mut b.uses);
    for u in &mut uses {
        u.place.story = s.id.clone();
    }
    let index = story::reach::index(uses, std::slice::from_ref(&s.id));
    let model = model::ProjectModel { name: project.name.clone(), stories: vec![s], index, files, stats: model::Stats { ms: 0, llm_calls: 0 }, setup: vec![], missing: vec![] };
    std::fs::write(out, serde_json::to_string_pretty(&model).unwrap()).expect("write model");
    println!("{}: {} items", model.stories[0].title, count(&model.stories[0].items));
    res.stop();
}

/// A step and the step it is inside.
fn find<'a>(items: &'a [model::Item], id: &str, parent: Option<&'a model::Item>) -> Option<(&'a model::Item, Option<&'a model::Item>)> {
    for i in items {
        if i.id() == id {
            return Some((i, parent));
        }
        for (_, l) in i.lists() {
            if let Some(f) = find(l, id, Some(i)) {
                return Some(f);
            }
        }
    }
    None
}

/// What the stories reach, most-used first; with words, every use of each thing whose id or name holds them.
fn english(args: &[String]) {
    use plainfold_core::english;
    use std::sync::atomic::AtomicBool;
    let opt = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let model: model::ProjectModel = serde_json::from_str(&std::fs::read_to_string(&args[0]).expect("model")).expect("model json");
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    let provider = english::openai::OpenAi::with_key(&key);
    let name = english::Provider::model(&provider);
    let chunks = english::plan(&model, &name);
    let steps: usize = chunks.iter().map(|c| c.steps.len()).sum();
    if let Some(n) = opt("--show") {
        let n: usize = n.parse().expect("--show <n>");
        println!("{} requests, {steps} steps\n", chunks.len());
        if let Some(c) = chunks.get(n) {
            let v: serde_json::Value = serde_json::from_str(&c.user).unwrap();
            println!("{}\n", v["code"].as_str().unwrap_or(""));
            for s in v["steps"].as_array().unwrap() {
                println!("{s}");
            }
        }
        return;
    }
    if key.is_empty() {
        eprintln!("OPENAI_API_KEY is not set: only what was kept is used");
    }
    let keep = opt("--keep").map(PathBuf::from);
    let t0 = Instant::now();
    let texts = std::sync::Mutex::new(english::Texts { model: name.clone(), texts: Default::default() });
    let verbose = std::env::var_os("VERBOSE").is_some();
    let summary = english::describe(&chunks, &provider, keep.as_deref(), &AtomicBool::new(false), &|b| {
        eprintln!("  {}/{} steps", b.done, b.total);
        if verbose {
            for (k, t, why) in &b.rejected {
                println!("REJECTED {k}\n    {t:?}: {why}");
            }
            for (k, s) in &b.texts {
                println!("{k}\n    {}", s.text);
            }
        }
        texts.lock().unwrap().texts.extend(b.texts.clone());
    });
    std::fs::write(&args[1], serde_json::to_string(&texts.into_inner().unwrap()).unwrap()).expect("write");
    println!(
        "{name}: {} of {} steps described in {} s; {} requests ({} kept), {} tokens in, {} out",
        summary.described,
        summary.steps,
        t0.elapsed().as_secs(),
        summary.requests,
        summary.kept,
        summary.tokens_in,
        summary.tokens_out
    );
    for (why, n) in &summary.rejected {
        println!("  not shown, {why}: {n}");
    }
    if let Some(e) = summary.error {
        println!("  error: {e}");
    }
}

fn print_uses(path: &str, words: Option<&str>) {
    let m: model::ProjectModel = serde_json::from_str(&std::fs::read_to_string(path).expect("read the model")).expect("a model.json");
    let stories = |t: &model::Thing| t.uses.iter().map(|u| u.story.as_str()).collect::<std::collections::BTreeSet<_>>().len();
    let kind = |t: &model::Thing| match &t.within {
        Some(w) => format!("in {w}"),
        None => format!("{:?}", t.kind).to_lowercase(),
    };
    let Some(q) = words.map(str::to_lowercase) else {
        let mut all: Vec<&model::Thing> = m.index.iter().collect();
        all.sort_by_key(|t| std::cmp::Reverse(stories(t)));
        println!("{} things reached by {} stories", all.len(), m.stories.len());
        for t in all {
            println!("{:>5} {:<12} {}", stories(t), kind(t), t.id);
        }
        return;
    };
    for t in m.index.iter().filter(|t| t.id.to_lowercase().contains(&q) || t.name.to_lowercase().contains(&q)) {
        println!("{}  ({}) · {} stories, {} uses", t.id, kind(t), stories(t), t.uses.len());
        for u in &t.uses {
            let story = m.stories.iter().find(|s| s.id == u.story);
            let title = story.map(|s| s.title.as_str()).unwrap_or(&u.story);
            let step = u.step.as_deref().zip(story).and_then(|(id, s)| find(&s.items, id, None)).map(|(i, _)| i.label().to_string());
            let step = match (step, u.drawn) {
                (Some(l), true) => format!("at {l}"),
                (Some(l), false) => format!("inside {l}"),
                (None, _) => String::new(),
            };
            let how = format!("{:?}", u.how).to_lowercase();
            println!("    {how:<8} {title}   {}:{}   {step}", u.at.file, u.at.from);
        }
    }
}

fn print_diff(old: &str, new: &str, json: bool) {
    use plainfold_core::diff::Mark;
    let read = |p: &str| -> model::ProjectModel { serde_json::from_str(&std::fs::read_to_string(p).expect("read the model")).expect("a model.json") };
    let (old, new) = (read(old), read(new));
    let changes = plainfold_core::diff::diff(&old, &new);
    if json {
        println!("{}", serde_json::to_string_pretty(&changes).unwrap());
        return;
    }
    for r in plainfold_core::renames::renames(&old, &new) {
        let what = match (r.was_name != r.name, r.was_file != r.file) {
            (true, true) => format!("renamed and moved {} → {}, {} → {}", r.was_name, r.name, r.was_file, r.file),
            (true, false) => format!("renamed {} → {}", r.was_name, r.name),
            _ => format!("moved {}: {} → {}", r.name, r.was_file, r.file),
        };
        println!("{what}  ({} → {})", r.was, r.id);
    }
    if changes.is_empty() {
        println!("no differences");
    }
    let sign = |m: Mark| match m {
        Mark::Added => "+",
        Mark::Removed => "−",
        Mark::Changed => "~",
    };
    for s in &changes {
        let renamed = [s.renamed.as_ref().map(|w| format!("renamed from {w}")), s.moved.as_ref().map(|f| format!("moved from {f}"))].into_iter().flatten().collect::<Vec<_>>().join(", ");
        let renamed = if renamed.is_empty() { renamed } else { format!("  ({renamed})") };
        println!("{} {}  ({}){renamed}", sign(s.mark), s.title, s.id);
        let Some(story) = new.stories.iter().find(|x| x.id == s.id) else { continue };
        for st in &s.steps {
            let (label, inside) = match &st.item {
                Some(gone) => (gone.label().to_string(), st.parent.as_deref().and_then(|p| find(&story.items, p, None)).map(|(p, _)| p.label().to_string())),
                None => {
                    let (it, parent) = find(&story.items, &st.id, None).expect("a step of the new story");
                    (it.label().to_string(), parent.map(|p| p.label().to_string()))
                }
            };
            let was = st.was.as_ref().map(|w| format!("   (was {})", if w.label != label { &w.label } else { w.detail.as_deref().unwrap_or("") })).unwrap_or_default();
            let inside = inside.map(|p| format!("   in {p}")).unwrap_or_default();
            println!("    {} {label}{was}{inside}", sign(st.mark));
        }
    }
}
