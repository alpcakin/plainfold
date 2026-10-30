// Usage: node src/extract.ts <project root> <out.json>
// Finds every story in the project and writes the model the view draws.
import fs from "node:fs";
import path from "node:path";
import { loadProject } from "./project.ts";
import { discover } from "./story.ts";
import type { Item } from "./model.ts";

const [root, out] = process.argv.slice(2);
const model = discover(loadProject(root));
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, JSON.stringify(model, null, 2));

const count = (items: Item[]): number =>
  items.reduce((n, i) => n + 1 + (i.t === "frame" ? count(i.items) + count(i.else ?? []) : i.t === "call" ? count(i.body ?? []) : 0), 0);
console.log(`${model.name}: ${model.stories.length} stories (${model.stats.ms} ms, ${model.stats.llmCalls} LLM calls)`);
for (const s of model.stories) {
  console.log(`  ${s.title}: ${s.participants.length} participants, ${count(s.items)} items, ${s.signals.length} signals, ${s.gaps.length} gaps`);
  for (const c of s.coverage) console.log(`    coverage ${c.fn}: ${c.covered}/${c.total}${c.missing.length ? " missing " + c.missing.join(",") : ""}`);
}
