// APScheduler adapter. Handles: jobs handed to a scheduler of the `apscheduler` package, by `add_job(f, …)`, by
// `add_schedule(f, …)` (its version 4) or by the `@scheduled_job(…)` decorator. Each job is a story of its own that
// starts when its trigger fires, named by when that is, as the code writes it: a trigger's name and its arguments
// (`"interval", minutes=5`; `"cron", day_of_week="sun", hour=20`) or a trigger object (`IntervalTrigger(hours=24)`,
// `CronTrigger.from_crontab("0 20 * * 0")`). Values are the code's own: `minutes=settings.every` reads
// "every settings.every minutes". Where a job is handed over, the step refers to its story. The scheduler's other
// calls (making it, `start()`) stay plain calls into the package. A job named by a string (`"module:func"`) or
// written in the call (a `lambda`) is not followed: that call stays a plain one too.
use super::{Adapter, Start};
use crate::lang::Expr;
use crate::model::{Call, Item, Link, LinkKind};
use crate::resolve::{Resolver, Target, callee_name};
use crate::source::{At, Project, SourceFile, one_line};
use crate::story::{Builder, Chain, Ctx, World, string_value};
use tree_sitter::Node;

pub struct ApScheduler;

const PACKAGE: &str = "apscheduler";
/// The methods that hand a scheduler a job.
const ADD: &[&str] = &["add_job", "add_schedule"];
/// The decorator that does.
const DECORATOR: &str = "scheduled_job";
/// An interval trigger's arguments, largest first, each with its unit's singular.
const UNITS: &[(&str, &str)] = &[("weeks", "week"), ("days", "day"), ("hours", "hour"), ("minutes", "minute"), ("seconds", "second")];
/// A cron trigger's fields, largest first.
const CRON: &[&str] = &["year", "month", "day", "week", "day_of_week", "hour", "minute", "second"];

/// A job as the code hands it over.
struct Job<'t> {
    /// the function as written (a name), or the definition the decorator is on
    func: Node<'t>,
    decorated: bool,
    /// the trigger as written: its name (`"cron"`) or an object. None: the job runs once, at once.
    trigger: Option<Node<'t>>,
    /// the keyword arguments given with it (the trigger's among them)
    named: Vec<(String, Node<'t>)>,
}

/// A call's positional arguments, and its keyword ones by name.
fn arguments<'t>(f: &SourceFile, call: Node<'t>) -> (Vec<Node<'t>>, Vec<(String, Node<'t>)>) {
    let l = f.lang;
    let (mut pos, mut named) = (vec![], vec![]);
    let Expr::Call { args, .. } = (l.expr)(call, &f.text) else { return (pos, named) };
    for a in args {
        match (l.named_arg)(a, &f.text) {
            Some((k, v)) => named.push((f.text_of(k).to_string(), v)),
            None => pos.push(a),
        }
    }
    (pos, named)
}

/// The name of the method a call is, when it is one that hands over a job (`scheduler.add_job`): the name node.
fn method_name<'t>(f: &SourceFile, call: Node<'t>) -> Option<Node<'t>> {
    let l = f.lang;
    let Expr::Call { callee, .. } = (l.expr)(call, &f.text) else { return None };
    let callee = l.unwrap(callee);
    let (_, prop) = l.member_fields(callee.kind())?;
    let name = callee.child_by_field_name(prop)?;
    let text = f.text_of(name);
    (ADD.contains(&text) || text == DECORATOR).then_some(name)
}

/// The job a call hands over, when the language server places the method it calls in APScheduler.
fn job<'t>(p: &Project, res: &Resolver, file: usize, call: Node<'t>) -> Option<Job<'t>> {
    let f = &p.files[file];
    let l = f.lang;
    let name = method_name(f, call)?;
    let loc = res.definition(file, name).into_iter().next()?;
    if p.is_own(&loc.file) || (l.package)(&loc.file).as_deref() != Some(PACKAGE) {
        return None;
    }
    let (pos, named) = arguments(f, call);
    let arg = |i: usize, key: &str| named.iter().find(|(k, _)| k == key).map(|(_, v)| *v).or_else(|| pos.get(i).copied());
    if f.text_of(name) == DECORATOR {
        // `@scheduler.scheduled_job("cron", …)` on a definition
        let dec = call.parent().filter(|d| l.reach.decorators.contains(&d.kind()))?;
        let func = dec.parent().and_then(|d| d.child_by_field_name("definition")).filter(|d| l.is_function(d.kind()))?;
        return Some(Job { func, decorated: true, trigger: arg(0, "trigger"), named });
    }
    let func = l.unwrap(arg(0, "func")?);
    Some(Job { func, decorated: false, trigger: arg(1, "trigger"), named })
}

/// A value as the code writes it: a string's text, anything else its code. And whether it is written out (a string
/// or a number), not computed.
fn value(f: &SourceFile, n: Node) -> (String, bool) {
    match string_value(f.lang, &f.text, n) {
        Some(s) => (s, true),
        None => {
            let code = one_line(f.text_of(n), 0);
            let literal = code.parse::<f64>().is_ok();
            (code, literal)
        }
    }
}

/// When a job runs, from its trigger as written.
fn when(f: &SourceFile, job: &Job) -> String {
    let l = f.lang;
    let Some(t) = job.trigger.map(|t| l.unwrap(t)) else { return words(f, "date", &job.named) };
    if let Some(kind) = string_value(l, &f.text, t) {
        return words(f, &kind, &job.named);
    }
    if let Expr::Call { callee, .. } = (l.expr)(t, &f.text) {
        let (pos, named) = arguments(f, t);
        match callee_name(callee, l).map(|n| f.text_of(n)) {
            Some("IntervalTrigger") if pos.is_empty() => return words(f, "interval", &named),
            Some("CronTrigger") if pos.is_empty() => return words(f, "cron", &named),
            Some("DateTrigger") => {
                let date = pos.first().map(|d| vec![("run_date".to_string(), *d)]).unwrap_or(named);
                return words(f, "date", &date);
            }
            Some("from_crontab") if pos.len() == 1 => return format!("cron {}", value(f, pos[0]).0),
            _ => {}
        }
    }
    format!("on its trigger {}", one_line(f.text_of(t), 40))
}

/// A trigger's kind and arguments in words: `every 5 minutes`, `every sun at 20:00`, `cron minute=0`.
fn words(f: &SourceFile, kind: &str, named: &[(String, Node)]) -> String {
    let get = |k: &str| named.iter().find(|(n, _)| n == k).map(|(_, v)| value(f, *v));
    match kind {
        "interval" => every(&UNITS.iter().filter_map(|(many, one)| get(many).map(|(v, _)| (v, *one, *many))).collect::<Vec<_>>()),
        "cron" => {
            let fields: Vec<(&str, (String, bool))> = CRON.iter().filter_map(|k| get(k).map(|v| (*k, v))).collect();
            if fields.is_empty() {
                return "on a cron trigger".into();
            }
            clock(&fields).unwrap_or_else(|| format!("cron {}", fields.iter().map(|(k, (v, _))| format!("{k}={v}")).collect::<Vec<_>>().join(", ")))
        }
        "date" => get("run_date").map(|(d, _)| format!("once at {d}")).unwrap_or_else(|| "once, right away".into()),
        other => format!("on its {other} trigger"),
    }
}

/// An interval in words, from its values and their units (singular, plural), largest first: `every 2 hours 30
/// minutes`, `every hour`.
fn every(parts: &[(String, &str, &str)]) -> String {
    match parts {
        [] => "on an interval".into(),
        [(v, one, _)] if v == "1" => format!("every {one}"),
        _ => format!("every {}", parts.iter().map(|(v, one, many)| format!("{v} {}", if v == "1" { one } else { many })).collect::<Vec<_>>().join(" ")),
    }
}

/// A cron trigger that fires at one time of day, every day or on the weekdays written: `every sun at 20:00`. Fields
/// smaller than the smallest one given start at zero, so `hour=20` alone is 20:00.
fn clock(fields: &[(&str, (String, bool))]) -> Option<String> {
    if fields.iter().any(|(k, (_, literal))| !["day_of_week", "hour", "minute", "second"].contains(k) || !literal) {
        return None;
    }
    let get = |k: &str| fields.iter().find(|(n, _)| *n == k).map(|(_, (v, _))| v.as_str());
    let num = |v: &str| v.parse::<u32>().ok();
    let hour = num(get("hour")?)?;
    let minute = match get("minute") {
        Some(m) => num(m)?,
        None => 0,
    };
    if hour > 23 || minute > 59 || get("second").is_some_and(|s| num(s) != Some(0)) {
        return None;
    }
    let days = get("day_of_week").map(|d| format!("every {d}")).unwrap_or_else(|| "every day".into());
    Some(format!("{days} at {hour:02}:{minute:02}"))
}

impl Adapter for ApScheduler {
    fn name(&self) -> &'static str {
        "APScheduler"
    }
    fn langs(&self) -> &'static [&'static str] {
        &["python"]
    }
    fn applies(&self, package: &str) -> bool {
        package == PACKAGE
    }

    fn starts<'w>(&self, w: &World<'w>) -> Vec<Start<'w>> {
        let p = w.p;
        let mut calls: Vec<(usize, Node<'w>, Node<'w>)> = vec![];
        for (i, f) in p.files.iter().enumerate().filter(|(i, _)| w.res.ready(*i)) {
            let mut stack = vec![f.tree.root_node()];
            while let Some(n) = stack.pop() {
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c));
                if f.lang.call_field(n.kind()).is_some()
                    && let Some(name) = method_name(f, n)
                {
                    calls.push((i, n, name));
                }
            }
        }
        w.res.prefetch(&calls.iter().map(|&(i, _, name)| (i, name)).collect::<Vec<_>>());
        let mut out = vec![];
        for (i, call, _) in calls {
            let f = &p.files[i];
            let Some(job) = job(p, w.res, i, call) else { continue };
            let (func, by) = if job.decorated {
                (At { file: i, node: job.func }, At { file: i, node: call.parent().unwrap_or(call) })
            } else {
                if f.lang.is_function(job.func.kind()) {
                    continue;
                }
                match w.res.target(i, job.func, w.adapters) {
                    Target::Own { func, .. } if p.files[func.file].lang.is_function(func.node.kind()) => (func, At { file: i, node: call }),
                    _ => continue,
                }
            };
            out.push(Start { kind: "schedule", func, by, when: when(f, &job), place: None, optional: false, around: None });
        }
        out.sort_by_key(|s| (s.by.file, s.by.node.start_byte()));
        out
    }

    /// Where a job is handed over: a step that refers to the job's story.
    fn call<'w>(&self, b: &mut Builder<'w>, e: At<'w>, chain: &Chain<'w>, ctx: &Ctx) -> Option<Vec<Item>> {
        let l = b.lang(e);
        let call = l.unwrap(e.node);
        chain.segs.last().filter(|s| s.node == call && ADD.contains(&s.name.as_str()))?;
        let job = job(b.w.p, b.w.res, e.file, call)?;
        if l.is_function(job.func.kind()) {
            return None;
        }
        let func = b.project_decl(b.at(e, job.func)).filter(|d| b.is_function(*d))?;
        let code = one_line(b.text(e), 0);
        let (unused, into) = b.result_use(e);
        Some(vec![Item::Call(Call {
            id: b.id(),
            from: ctx.this.clone(),
            to: ctx.this.clone(),
            label: one_line(&code, 56),
            detail: Some(code),
            link: Link { kind: LinkKind::Exact, how: "APScheduler: the scheduler runs the job when its trigger fires".into() },
            effect: None,
            at: b.place(e),
            target: Some(b.place(func)),
            body: None,
            note: Some(format!("runs {}", when(b.file(e), &job))),
            opens: None,
            story: Some(format!("schedule:{}", b.pid(func))),
            table: None,
            touches: None,
            unused,
            into,
            ends: vec![],
            changes: vec![],
        })])
    }
}

#[cfg(test)]
mod tests {
    use super::{clock, every};

    fn fields(list: &[(&'static str, &str, bool)]) -> Vec<(&'static str, (String, bool))> {
        list.iter().map(|(k, v, l)| (*k, (v.to_string(), *l))).collect()
    }

    #[test]
    fn an_interval_reads_with_its_units() {
        assert_eq!(every(&[("5".into(), "minute", "minutes")]), "every 5 minutes");
        assert_eq!(every(&[("1".into(), "hour", "hours")]), "every hour");
        assert_eq!(every(&[("2".into(), "hour", "hours"), ("1".into(), "minute", "minutes")]), "every 2 hours 1 minute");
        assert_eq!(every(&[("cfg.every".into(), "minute", "minutes")]), "every cfg.every minutes");
        assert_eq!(every(&[]), "on an interval");
    }

    #[test]
    fn a_cron_at_one_time_of_day_reads_as_a_clock() {
        assert_eq!(clock(&fields(&[("day_of_week", "sun", true), ("hour", "20", true), ("minute", "0", true)])).as_deref(), Some("every sun at 20:00"));
        assert_eq!(clock(&fields(&[("hour", "8", true)])).as_deref(), Some("every day at 08:00"));
        assert_eq!(clock(&fields(&[("hour", "8", true), ("minute", "5", true), ("second", "0", true)])).as_deref(), Some("every day at 08:05"));
        // anything else is written as its fields
        assert_eq!(clock(&fields(&[("minute", "0", true)])), None);
        assert_eq!(clock(&fields(&[("hour", "*/2", true)])), None);
        assert_eq!(clock(&fields(&[("day", "1", true), ("hour", "0", true)])), None);
        assert_eq!(clock(&fields(&[("hour", "cfg.hour", false)])), None);
        assert_eq!(clock(&fields(&[("hour", "8", true), ("second", "30", true)])), None);
    }
}
