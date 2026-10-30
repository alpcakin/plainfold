// Usage: node scripts/story.mjs <model.json> <part of a story id> [depth]: stories whose id has that part, as indented text (N=3 sets how many). For reading what a change did without the app.
import fs from "node:fs";
const [, , file, q, depthArg] = process.argv;
const m = JSON.parse(fs.readFileSync(file, "utf8"));
const maxDepth = Number(depthArg ?? 9);
const stories = m.stories.filter((s) => s.id.includes(q));
const nm = (id, s) => s.participants.find((p) => p.id === id)?.name ?? id;
const one = (x) => String(x ?? "").replace(/\s+/g, " ").slice(0, 100);
function walk(items, s, d) {
  for (const it of items ?? []) {
    const pad = "  ".repeat(d);
    const ref = it.ref ? ` [${it.ref.file.split("/").pop()}:${it.ref.from}]` : "";
    switch (it.t) {
      case "call": console.log(`${pad}${nm(it.from, s)} -> ${nm(it.to, s)}: ${one(it.label)}${it.into ? " → " + it.into : ""}${it.link?.kind !== "exact" ? " {" + it.link?.kind + "}" : ""}${ref}`); if (d < maxDepth) walk(it.body, s, d + 1); break;
      case "return": console.log(`${pad}<- ${nm(it.to, s)}: ${one(it.label)}${it.throws ? " (throws)" : ""}${ref}`); break;
      case "self": console.log(`${pad}• ${it.kind}: ${one(it.label)}${it.change ? " ~" + JSON.stringify(it.change).slice(0, 60) : ""}${ref}`); break;
      case "frame": console.log(`${pad}[${it.kind}] ${one(it.label)}${it.note ? " (" + it.note + ")" : ""}${ref}`); if (d < maxDepth) { walk(it.items, s, d + 1); if (it.els) { console.log(pad + "[else]"); walk(it.els, s, d + 1); } } break;
      default: console.log(`${pad}?${it.t} ${one(JSON.stringify(it))}`);
    }
  }
}
for (const s of stories.slice(0, Number(process.env.N ?? 3))) {
  console.log(`=== ${s.id} · trigger ${s.trigger?.kind} ${one(s.trigger?.handler ?? s.trigger?.route)} · gaps ${s.gaps?.length ?? 0}`);
  walk(s.items, s, 0);
  for (const g of s.gaps ?? []) console.log(`  ! gap: ${one(g.text)}`);
}
console.log(`(${stories.length} matching stories)`);
