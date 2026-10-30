// Usage: node src/render.ts <model.json> <out.html>
// Puts the story model into the view page. The page draws the diagram itself, the way the app UI will.
import fs from "node:fs";
import path from "node:path";

const [model, out, labelsFile] = process.argv.slice(2);
const view = fs.readFileSync(new URL("./view.html", import.meta.url), "utf8");
const json = fs.readFileSync(model, "utf8").replace(/</g, "\\u003c");
fs.mkdirSync(path.dirname(out), { recursive: true });
const labels = labelsFile ? fs.readFileSync(labelsFile, "utf8").replace(/</g, "\\u003c") : "{}";
fs.writeFileSync(out, view.replace("/*MODEL*/", () => json).replace("/*LABELS*/", () => labels));
console.log(`wrote ${out}`);
