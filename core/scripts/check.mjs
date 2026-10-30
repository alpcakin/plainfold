// Usage: node scripts/check.mjs [--base <commit>] [--fresh] [--all] [fixture name | project folder …]
// What a change to the core changes: reads every fixture (scripts/fixtures.json) with the core of the last commit
// (or --base) and with the core as it is in the working tree, and compares the two models story by story. Nothing is
// kept up to date by hand: the comparison is always with a commit, so it shows only what the change does.
// A difference is not a failure by itself (a general fix should change other projects too): it is there to be read.
// Names pick fixtures; a folder adds a project of your own. The base's core and its models are kept in
// target/check, one set per commit (--fresh reads the models again). --all prints every changed step.
// Exits with 1 when any model differs, 0 when none does.
import { execSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const core = path.resolve(import.meta.dirname, "..");
const repo = path.resolve(core, "..");
const work = path.join(core, "target", "check");

const args = process.argv.slice(2);
const flag = (f) => (args.includes(f) ? (args.splice(args.indexOf(f), 1), true) : false);
const fresh = flag("--fresh");
const all = flag("--all");
let base = "HEAD";
if (args.includes("--base")) base = args.splice(args.indexOf("--base"), 2)[1];

const say = (s) => process.stdout.write(s + "\n");
const run = (cmd, opts = {}) => execSync(cmd, { stdio: ["ignore", "pipe", "inherit"], encoding: "utf8", ...opts }).trim();

// the projects: the fixtures named (all when none is), and folders given
const known = JSON.parse(fs.readFileSync(path.join(import.meta.dirname, "fixtures.json"), "utf8"));
const named = args.filter((a) => !fs.existsSync(a));
const folders = args.filter((a) => fs.existsSync(a)).map((a) => ({ name: path.basename(path.resolve(a)), abs: path.resolve(a) }));
const unknown = named.filter((n) => !known.some((f) => f.name === n));
if (unknown.length) {
  say(`not a fixture: ${unknown.join(", ")} (fixtures: ${known.map((f) => f.name).join(", ")})`);
  process.exit(2);
}
const projects = [...(named.length || !folders.length ? known.filter((f) => !named.length || named.includes(f.name)) : []).map((f) => ({ ...f, abs: path.resolve(repo, f.path) })), ...folders];

// 1. the two cores: the commit's, built once in its own folder, and the working tree's
const rev = run(`git -C "${repo}" rev-parse --short ${base}`);
const src = path.join(work, `base-${rev}`);
if (!fs.existsSync(path.join(src, "core", "Cargo.toml"))) {
  for (const old of fs.existsSync(work) ? fs.readdirSync(work).filter((d) => d.startsWith("base-")) : []) fs.rmSync(path.join(work, old), { recursive: true, force: true });
  fs.mkdirSync(src, { recursive: true });
  run(`git -C "${repo}" archive ${rev} core | tar -x -C "${src}"`);
}
say(`building the core of ${rev} and the working tree's…`);
run(`cargo build --quiet --manifest-path "${path.join(src, "core", "Cargo.toml")}"`, { env: { ...process.env, CARGO_TARGET_DIR: path.join(work, "target") } });
run(`cargo build --quiet --manifest-path "${path.join(core, "Cargo.toml")}"`);
const bins = { base: path.join(work, "target", "debug", "plainfold-core"), now: path.join(core, "target", "debug", "plainfold-core") };

// the development servers, as CLAUDE.md says, unless others are given
const env = { ...process.env };
const dev = { PLAINFOLD_SERVERS: path.join(repo, "spike/universal/node_modules/.bin"), PLAINFOLD_TSSERVER: path.join(repo, "spike/skeleton/node_modules/typescript/lib") };
for (const [k, v] of Object.entries(dev)) if (!env[k] && fs.existsSync(v)) env[k] = v;

// a model without what differs between two reads of the same code (how long it took), keys in one order
const canonical = (v) => (Array.isArray(v) ? v.map(canonical) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canonical(v[k])])) : v);
const load = (file) => {
  const m = JSON.parse(fs.readFileSync(file, "utf8"));
  delete m.stats;
  return JSON.stringify(canonical(m));
};

const extract = (which, p, out) => {
  const a = ["extract", p.abs, out];
  if (p.python) a.push("--python", path.resolve(repo, p.python));
  const r = spawnSync(bins[which], a, { env, encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.status !== 0) throw new Error(`${which} core on ${p.name}: ${(r.stderr || r.stdout).trim().split("\n").slice(-3).join(" · ")}`);
  return r.stdout.split("\n")[0];
};

// 2. each project, read by both
const tally = { same: [], changed: [], missing: [], failed: [] };
for (const p of projects) {
  if (!fs.existsSync(p.abs)) {
    tally.missing.push(p.name);
    say(`– ${p.name}: not here${p.repo ? ` (${p.repo} at ${p.commit}${p.folder ? `, its ${p.folder} folder` : ""}, into ${p.path})` : ""}`);
    continue;
  }
  const dir = path.join(work, "models", rev);
  fs.mkdirSync(dir, { recursive: true });
  const before = path.join(dir, `${p.name}.json`);
  const after = path.join(work, "models", `${p.name}.now.json`);
  try {
    if (fresh || !fs.existsSync(before)) extract("base", p, before);
    const summary = extract("now", p, after);
    if (load(before) === load(after)) {
      tally.same.push(p.name);
      say(`= ${p.name}: the same (${summary.replace(/^[^:]*: /, "").replace(/ \(.*/, "")})`);
      continue;
    }
    tally.changed.push(p.name);
    const d = spawnSync(bins.now, ["diff", before, after], { encoding: "utf8", maxBuffer: 1 << 28 }).stdout.trimEnd().split("\n");
    const shown = all ? d : d.slice(0, 40);
    say(`≠ ${p.name}:`);
    for (const line of shown) say(`    ${line}`);
    if (shown.length < d.length) say(`    … ${d.length - shown.length} more lines (--all shows them)`);
  } catch (e) {
    tally.failed.push(p.name);
    say(`! ${p.name}: ${e.message}`);
  }
}

say("");
say(`against ${rev}: ${tally.same.length} the same, ${tally.changed.length} changed${tally.changed.length ? ` (${tally.changed.join(", ")})` : ""}${tally.missing.length ? `, ${tally.missing.length} not here` : ""}${tally.failed.length ? `, ${tally.failed.length} failed (${tally.failed.join(", ")})` : ""}`);
process.exit(tally.changed.length || tally.failed.length ? 1 : 0);
