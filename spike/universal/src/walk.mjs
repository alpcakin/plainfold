// Language-neutral walker. Everything language-specific comes from the LANGS entry; nothing here names a language.
import fs from "node:fs";
import { Parser, Language } from "web-tree-sitter";
import { LANGS, langFor } from "./langs.mjs";

await Parser.init();
const parsers = {};
async function parserFor(lang) {
  if (!parsers[lang]) {
    const p = new Parser();
    p.setLanguage(await Language.load(LANGS[lang].grammar));
    parsers[lang] = p;
  }
  return parsers[lang];
}

const trees = new Map();
// Parsed file (cached), or null when no grammar handles it.
export async function treeFor(file) {
  if (trees.has(file)) return trees.get(file);
  const lang = langFor(file);
  let entry = null;
  if (lang && fs.existsSync(file)) {
    const text = fs.readFileSync(file, "utf8");
    entry = { lang, L: LANGS[lang], text, tree: (await parserFor(lang)).parse(text) };
  }
  trees.set(file, entry);
  return entry;
}

const oneLine = (s, max = 90) => {
  const t = s.replace(/\s*\n\s*/g, " ").trim();
  return t.length > max ? t.slice(0, max - 1) + "…" : t;
};

// The name node that "go to definition" should be asked about for a callee expression, or null.
export function calleeName(n, L) {
  while (n && L.unwrap.includes(n.type)) n = n.namedChild(0);
  if (!n) return null;
  if (L.names.includes(n.type)) return n;
  if (L.member[n.type]) return calleeName(n.childForFieldName(L.member[n.type]), L);
  return null;
}

function site(kind, node, callee, name) {
  return {
    kind,
    line: node.startPosition.row + 1,
    callee: oneLine(callee.text, 60),
    name: name.text,
    at: { line: name.startPosition.row, col: name.startPosition.column }, // 0-based, UTF-16: what LSP expects
    text: oneLine(node.text),
  };
}

// Every call site in a file, in evaluation order (a call comes after its callee and arguments).
export function callSites(entry) {
  const { L, tree } = entry;
  const out = [];
  (function visit(n) {
    for (const c of n.namedChildren) visit(c);
    if (L.calls[n.type]) {
      const callee = n.childForFieldName(L.calls[n.type]);
      const name = callee && calleeName(callee, L);
      if (name) out.push(site("call", n, callee, name));
    } else if (L.elements[n.type]) {
      const tag = n.childForFieldName(L.elements[n.type]);
      const name = tag && calleeName(tag, L);
      if (name) out.push(site("element", n, tag, name));
    }
  })(tree.rootNode);
  return out;
}

// The shape of one function body: calls in evaluation order, branches and loops as frames, callbacks as their own frame.
export function structure(fn, L) {
  const body = fn.childForFieldName("body") ?? fn;
  const walk = (n) => {
    const items = [];
    for (const c of n.namedChildren) items.push(...node(c));
    return items;
  };
  const node = (n) => {
    if (L.functions.includes(n.type)) return [{ t: "frame", kind: "callback", label: oneLine(n.text, 40), items: walk(n.childForFieldName("body") ?? n) }];
    const br = L.branches[n.type];
    if (br) {
      const part = (f) => (f && n.childForFieldName(f) ? walk(n.childForFieldName(f)) : []);
      const cond = br.cond && n.childForFieldName(br.cond);
      return [...(cond ? node(cond) : []), { t: "frame", kind: n.type.replace(/_(statement|expression|clause)$/, ""), label: cond ? oneLine(cond.text, 50) : "", items: part(br.then), else: [...part(br.else), ...part(br.finally)] }];
    }
    if (L.loops[n.type]) return [{ t: "frame", kind: "loop", label: oneLine(n.text, 40), items: walk(n) }];
    const inner = walk(n);
    if (L.calls[n.type]) {
      const callee = n.childForFieldName(L.calls[n.type]);
      const name = callee && calleeName(callee, L);
      if (name) inner.push({ t: "call", ...site("call", n, callee, name) });
    } else if (L.elements[n.type]) {
      const tag = n.childForFieldName(L.elements[n.type]);
      const name = tag && calleeName(tag, L);
      if (name) inner.push({ t: "call", ...site("element", n, tag, name) });
    }
    return inner;
  };
  return walk(body);
}

// Named functions in a file: the declared name, or the variable/property they are assigned to.
export function functions(entry) {
  const { L, tree } = entry;
  const out = [];
  (function visit(n) {
    if (L.functions.includes(n.type)) {
      let name = L.functionName(n);
      const p = n.parent;
      if (!name && p && L.follow.declarator && p.type === L.follow.declarator.type) name = p.childForFieldName(L.follow.declarator.name);
      if (name) out.push({ name: name.text, node: n, line: n.startPosition.row + 1 });
    }
    for (const c of n.namedChildren) visit(c);
  })(tree.rootNode);
  return out;
}

export function printStructure(items, indent = "") {
  const lines = [];
  for (const i of items) {
    if (i.t === "call") lines.push(`${indent}→ ${i.callee}${i.resolved ? `   ⟶ ${i.resolved}` : ""}`);
    else {
      lines.push(`${indent}[${i.kind}${i.label ? " " + i.label : ""}]`);
      lines.push(...printStructure(i.items, indent + "  "));
      if (i.else?.length) { lines.push(`${indent}[else]`); lines.push(...printStructure(i.else, indent + "  ")); }
    }
  }
  return lines;
}
