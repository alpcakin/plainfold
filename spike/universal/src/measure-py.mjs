// Usage: node src/measure-py.mjs <project root> <python of an environment> <label> [DEMO=functionName]
// Python has no compiler to measure against, so two independent engines answer the same question for every call site
// (pyright through LSP, Jedi directly) and every place they disagree on the project's own code is listed for review.
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { treeFor, callSites, structure, functions, printStructure } from "./walk.mjs";
import { startServer, pool } from "./lsp.mjs";
import { follow } from "./follow.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const out = (s) => process.stdout.write(s + "\n");
const [rootArg, pythonArg, label] = process.argv.slice(2);
const python = path.resolve(pythonArg); // the server runs in the project folder, so a relative path would point elsewhere
const root = fs.realpathSync(path.resolve(rootArg));
const real = (f) => { try { return fs.realpathSync(f); } catch { return f; } };
const SKIP = new Set([".git", "node_modules", "__pycache__", ".venv", "venv", ".mypy_cache", ".pytest_cache"]);

const files = [];
(function scan(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.isDirectory()) { if (!SKIP.has(e.name) && !e.name.startsWith("venv")) scan(path.join(dir, e.name)); }
    else if (e.name.endsWith(".py")) files.push(path.join(dir, e.name));
  }
})(root);

const t0 = performance.now();
const sites = [];
for (const f of files) {
  const entry = await treeFor(f);
  for (const s of callSites(entry)) sites.push({ ...s, abs: f, file: path.relative(root, f) });
}
const tWalk = performance.now();

const lsp = await startServer("python", root, { pythonPath: python });
for (const f of files) lsp.open(f);
await lsp.settle();
const tStart = performance.now();
await pool(sites, 16, async (s) => {
  try { s.pyright = (await lsp.definition(s.abs, s.at.line, s.at.col)).map((d) => ({ ...d, file: real(d.file) }))[0] ?? null; }
  catch (err) { s.pyright = null; s.error = String(err.message || err); }
});
const tResolve = performance.now();

// Jedi, in the same environment
const tmp = fs.mkdtempSync(path.join(here, "..", "out", "jedi-"));
fs.writeFileSync(path.join(tmp, "sites.json"), JSON.stringify(sites.map((s) => ({ abs: s.abs, at: s.at }))));
execFileSync(python, [path.join(here, "jedi_goto.py"), root, path.join(tmp, "sites.json"), path.join(tmp, "out.json")], { stdio: "inherit", maxBuffer: 1 << 28 });
const jedi = JSON.parse(fs.readFileSync(path.join(tmp, "out.json"), "utf8"));
sites.forEach((s, i) => { const j = jedi[i]; s.jedi = j && j.file ? { file: real(j.file), line: j.line } : null; });
fs.rmSync(tmp, { recursive: true });
const tJedi = performance.now();

// ---------- classify ----------
const envDir = real(path.resolve(python, "..", ".."));
const where = (d) => !d ? "none" : d.file.startsWith(root + path.sep) && !d.file.startsWith(envDir) ? "own" : /typeshed|[\\/]stdlib[\\/]|builtins\.pyi$/.test(d.file) ? "stdlib" : "library";
const stem = (f) => path.basename(f).replace(/\.pyi?$/, "");
const short = (d) => d ? `${where(d) === "own" ? path.relative(root, d.file) : path.basename(d.file)}:${d.line + 1}` : "—";
const counts = { sites: sites.length, pyright: {}, jedi: {} };
const isTest = (f) => /(^|\/)tests?\//.test(f) || /(^|\/)(test_[^/]*|[^/]*_test|conftest)\.py$/.test(f);
const scoped = { app: { sites: 0, pyright: {}, jedi: {} }, tests: { sites: 0, pyright: {}, jedi: {} } };
const bump = (o, k) => { o[k] = (o[k] || 0) + 1; };
const ownCmp = { bothSame: 0, bothDiffer: 0, pyrightOnly: 0, jediOnly: 0 };
const review = [];
const lines = new Map();
const lineOf = (f, l) => { if (!lines.has(f)) lines.set(f, fs.readFileSync(f, "utf8").split("\n")); return (lines.get(f)[l] ?? "").trim().slice(0, 100); };
for (const s of sites) {
  bump(counts.pyright, where(s.pyright));
  bump(counts.jedi, where(s.jedi));
  const sc = scoped[isTest(s.file) ? "tests" : "app"]; sc.sites++; bump(sc.pyright, where(s.pyright)); bump(sc.jedi, where(s.jedi));
  const pOwn = where(s.pyright) === "own", jOwn = where(s.jedi) === "own";
  if (!pOwn && !jOwn) continue;
  const same = s.pyright && s.jedi && s.pyright.file === s.jedi.file && s.pyright.line === s.jedi.line;
  const k = same ? "bothSame" : pOwn && jOwn ? "bothDiffer" : pOwn ? "pyrightOnly" : "jediOnly";
  bump(ownCmp, k);
  if (k !== "bothSame") review.push({ k, at: `${s.file}:${s.line}`, call: s.text, pyright: short(s.pyright), pyrightLine: s.pyright && lineOf(s.pyright.file, s.pyright.line), jedi: short(s.jedi), jediLine: s.jedi && lineOf(s.jedi.file, s.jedi.line) });
}
// library/stdlib answers: same place, or the same module reached as a stub (.pyi) vs its source (.py)
const libCmp = { same: 0, stubVsSource: 0, other: 0 };
for (const s of sites) {
  if (!s.pyright || !s.jedi || where(s.pyright) === "own" || where(s.jedi) === "own") continue;
  if (s.pyright.file === s.jedi.file && s.pyright.line === s.jedi.line) libCmp.same++;
  else if (stem(s.pyright.file) === stem(s.jedi.file) || s.pyright.file.endsWith(".pyi") !== s.jedi.file.endsWith(".pyi")) libCmp.stubVsSource++;
  else libCmp.other++;
}
// own-code answers sanity check: does the definition carry the name that was called?
let nameOk = 0, nameBad = [];
for (const s of sites) if (where(s.pyright) === "own") {
  const txt = lines.has(s.pyright.file) ? null : null;
  const line = fs.readFileSync(s.pyright.file, "utf8").split("\n")[s.pyright.line] ?? "";
  if (line.slice(s.pyright.col).startsWith(s.name)) nameOk++; else nameBad.push(`${s.file}:${s.line} ${s.name} → ${short(s.pyright)}: ${line.trim().slice(0, 80)}`);
}

// ---------- one structure, to see the shape ----------
let demo = [];
if (process.env.DEMO) {
  for (const f of files) {
    const entry = await treeFor(f);
    const fn = functions(entry).find((x) => x.name === process.env.DEMO);
    if (!fn) continue;
    const items = structure(fn.node, entry.L);
    const mark = async (list) => { for (const i of list) {
      if (i.t === "call") {
        const d = (await lsp.definition(f, i.at.line, i.at.col))[0];
        const fl = d && await follow(lsp, { ...d, file: real(d.file) });
        i.resolved = fl ? short(fl.loc) + (fl.hops.length ? ` (${fl.hops.join(" → ")})` : "") : "unresolved";
      } else { await mark(i.items); await mark(i.else || []); }
    } };
    await mark(items);
    demo = [`${process.env.DEMO} · ${path.relative(root, f)}:${fn.line}`, ...printStructure(items, "  ")];
    break;
  }
}
await lsp.stop();

const ms = (a, b) => Math.round(b - a);
const pyrightNone = sites.filter((x) => !x.pyright).map((x) => `${x.file}:${x.line} ${x.callee} · jedi: ${short(x.jedi)}`);
const errors = {}; for (const x of sites) if (x.error) errors[x.error] = (errors[x.error] || 0) + 1;
const report = { scoped, errors, project: path.basename(root), label, files: files.length, timing_ms: { treeSitterWalk: ms(t0, tWalk), serverStart: ms(tWalk, tStart), pyrightAllSites: ms(tStart, tResolve), jediAllSites: ms(tResolve, tJedi) }, counts, ownCmp, libCmp, ownNameCheck: { ok: nameOk, bad: nameBad.length }, nameBad: nameBad.slice(0, 20), review, pyrightNone, demo };
fs.writeFileSync(path.join(here, "..", "out", `${path.basename(root)}-${label}-py.json`), JSON.stringify(report, null, 2));
out(JSON.stringify({ scoped, errors, project: report.project, label, files: files.length, timing_ms: report.timing_ms, counts, ownCmp, libCmp, ownNameCheck: report.ownNameCheck, reviewCount: review.length }, null, 2));
if (demo.length) out(demo.join("\n"));
