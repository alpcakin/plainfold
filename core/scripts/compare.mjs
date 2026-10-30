// Usage: node scripts/compare.mjs <reference model.json> <core model.json> [story filter]
// Compares the core's stories with a reference model (the TypeScript skeleton's output) step by step.
// A step is compared by its shape and place, not its wording: kind, who to whom, file and line, nesting.
// Labels are allowed to differ (the core names returned values as the caller names them, for example).
import fs from "node:fs";

const [refFile, ourFile, filter] = process.argv.slice(2);
const ref = JSON.parse(fs.readFileSync(refFile, "utf8"));
const our = JSON.parse(fs.readFileSync(ourFile, "utf8"));
const verbose = !!process.env.VERBOSE;

// a story is the same story when its trigger is the same place in the code
const key = (s) => `${s.trigger.kind} ${s.trigger.ref.file}:${s.trigger.ref.from}`;

function steps(story) {
  const names = new Map(story.participants.map((p) => [p.id, `${p.kind}:${p.name}`]));
  const n = (id) => names.get(id) ?? id;
  const out = [];
  const walk = (items, depth) => {
    for (const i of items) {
      const at = `${i.ref.file}:${i.ref.from}`;
      const pad = "  ".repeat(depth);
      if (i.t === "call") out.push(`${pad}call ${n(i.from)} → ${n(i.to)} @${at}${i.effect ? " " + i.effect : ""}`);
      if (i.t === "return") out.push(`${pad}return ${n(i.from)} → ${n(i.to)} @${at}`);
      if (i.t === "self") out.push(`${pad}self ${i.kind} ${n(i.at)} @${at}`);
      if (i.t === "frame") out.push(`${pad}frame ${i.kind} @${at}`);
      if (i.t === "call" && i.body) walk(i.body, depth + 1);
      if (i.t === "frame") {
        walk(i.items, depth + 1);
        if (i.else?.length) { out.push(`${pad}else`); walk(i.else, depth + 1); }
      }
    }
  };
  walk(story.items, 0);
  return out;
}

// longest common subsequence, with the unmatched steps of each side
function diff(a, b) {
  const m = a.length, k = b.length;
  const L = Array.from({ length: m + 1 }, () => new Int32Array(k + 1));
  for (let i = m - 1; i >= 0; i--) for (let j = k - 1; j >= 0; j--) L[i][j] = a[i] === b[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
  const lines = [];
  let i = 0, j = 0;
  while (i < m && j < k) {
    if (a[i] === b[j]) { lines.push("  " + a[i]); i++; j++; }
    else if (L[i + 1][j] >= L[i][j + 1]) lines.push("- " + a[i++]);
    else lines.push("+ " + b[j++]);
  }
  while (i < m) lines.push("- " + a[i++]);
  while (j < k) lines.push("+ " + b[j++]);
  return { same: L[0][0], lines };
}

const ours = new Map(our.stories.map((s) => [key(s), s]));
let total = 0, same = 0, extra = 0;
const missingStories = [];
for (const s of ref.stories) {
  if (filter && !s.title.includes(filter)) continue;
  const o = ours.get(key(s));
  const a = steps(s);
  total += a.length;
  if (!o) { missingStories.push(s.title); continue; }
  ours.delete(key(s));
  const b = steps(o);
  const d = diff(a, b);
  same += d.same;
  extra += b.length - d.same;
  const pa = new Set(s.participants.map((p) => `${p.kind}:${p.name}`)), pb = new Set(o.participants.map((p) => `${p.kind}:${p.name}`));
  const partDiff = [...[...pa].filter((x) => !pb.has(x)).map((x) => "-" + x), ...[...pb].filter((x) => !pa.has(x)).map((x) => "+" + x)];
  const sig = (l) => new Set(l.map((f) => f.text));
  const sa = sig(s.signals), sb = sig(o.signals);
  const sigDiff = [...[...sa].filter((x) => !sb.has(x)).map((x) => "- " + x), ...[...sb].filter((x) => !sa.has(x)).map((x) => "+ " + x)];
  const ok = d.same === a.length && b.length === a.length;
  console.log(`${ok ? "=" : "≠"} ${s.title}: ${d.same}/${a.length} steps match${b.length > d.same ? `, ${b.length - d.same} extra` : ""}${partDiff.length ? ` · participants ${partDiff.join(" ")}` : ""}`);
  for (const x of sigDiff) console.log(`    signal ${x}`);
  if (!ok && verbose) for (const l of d.lines) if (verbose === "all" || !l.startsWith("  ")) console.log("    " + l);
  if (!ok && process.env.VERBOSE === "all") for (const l of d.lines) console.log("    " + l);
}
for (const t of missingStories) console.log(`- story missing: ${t}`);
for (const s of ours.values()) if (!filter || s.title.includes(filter)) console.log(`+ story only in core: ${s.title}`);
console.log(`\n${same}/${total} reference steps reproduced (${total ? ((100 * same) / total).toFixed(1) : 0}%), ${extra} extra steps; stories: ${ref.stories.length - missingStories.length}/${ref.stories.length} found, ${ours.size} only in core`);
