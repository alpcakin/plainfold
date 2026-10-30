// "Go to definition" answers one hop. A story often needs a few more: the name it lands on may just be a local
// alias (`const x = y`) or a value taken out of what another function returned (`const { refresh } = useThing()`).
// This follows those hops with the same two tools only: tree-sitter for the shape, the language server for each jump.
// Node types come from the language's `follow` entry; nothing here names a language.
import { treeFor, calleeName } from "./walk.mjs";

const same = (a, b) => a && b && a.startIndex === b.startIndex && a.endIndex === b.endIndex;
const unwrap = (n, L) => { while (n && L.unwrap.includes(n.type)) n = n.namedChild(0); return n; };
const keyText = (k) => (k ? k.text.replace(/^["'`]|["'`]$/g, "") : null);

// The function node a definition location names, if it names one.
function functionAt(nameNode, L) {
  const p = nameNode.parent;
  if (!p) return null;
  if (L.functions.includes(p.type)) return p;
  const D = L.follow.declarator;
  if (D && p.type === D.type && same(p.childForFieldName(D.name), nameNode)) {
    const v = unwrap(p.childForFieldName(D.value), L);
    if (v && L.functions.includes(v.type)) return v;
  }
  return null;
}

// Return statements that belong to this function (not to functions nested in it).
function ownReturns(fn, L) {
  const out = [];
  (function visit(n) {
    for (const c of n.namedChildren) {
      if (L.functions.includes(c.type)) continue;
      if (c.type === L.follow.returns) out.push(c);
      visit(c);
    }
  })(fn.childForFieldName("body") ?? fn);
  return out;
}

// Follows a definition location until it reaches something that is not just another name for a value.
// Returns { loc, hops, stop } where stop says why it ended.
export async function follow(lsp, loc, depth = 0, hops = []) {
  if (depth > 6) return { loc, hops, stop: "too deep" };
  const entry = await treeFor(loc.file);
  if (!entry) return { loc, hops, stop: "no grammar" };
  const { L, tree } = entry;
  const F = L.follow;
  const node = tree.rootNode.descendantForPosition({ row: loc.line, column: loc.col });
  if (!node) return { loc, hops, stop: "no node" };
  const p = node.parent;
  const jump = async (nameNode, why) => {
    const next = (await lsp.definition(loc.file, nameNode.startPosition.row, nameNode.startPosition.column))[0];
    if (!next || (next.file === loc.file && next.line === loc.line && next.col === loc.col)) return { loc, hops, stop: `${why}: no further definition` };
    return follow(lsp, next, depth + 1, [...hops, why]);
  };

  // `name = otherName` / `const name = obj.member`: a plain alias, keep going
  if (F.declarator && p && p.type === F.declarator.type && same(p.childForFieldName(F.declarator.name), node)) {
    const v = unwrap(p.childForFieldName(F.declarator.value), L);
    if (v && !L.functions.includes(v.type) && !L.calls[v.type]) {
      const n = calleeName(v, L);
      if (n) return jump(n, "alias");
    }
    return { loc, hops, stop: "declaration" };
  }

  // `const { key } = f()` / `const { key: name } = f()`: the value of `key` in what f returns
  if (F.destructure) {
    const D = F.destructure;
    let key = null, pattern = null;
    if (node.type === D.shorthand) { key = node.text; pattern = p; }
    else if (p && p.type === D.pair.type && same(p.childForFieldName(D.pair.value), node)) { key = keyText(p.childForFieldName(D.pair.key)); pattern = p.parent; }
    if (key && pattern && pattern.type === D.pattern) {
      const decl = pattern.parent;
      const call = decl && decl.type === F.declarator.type ? unwrap(decl.childForFieldName(F.declarator.value), L) : null;
      if (!call || !L.calls[call.type]) return { loc, hops, stop: "destructured from something that is not a call" };
      const callee = calleeName(call.childForFieldName(L.calls[call.type]), L);
      if (!callee) return { loc, hops, stop: "destructured from an unnamed call" };
      const fnLoc = (await lsp.definition(loc.file, callee.startPosition.row, callee.startPosition.column))[0];
      if (!fnLoc) return { loc, hops, stop: "returning function not resolved" };
      const fe = await treeFor(fnLoc.file);
      const fnName = fe && fe.tree.rootNode.descendantForPosition({ row: fnLoc.line, column: fnLoc.col });
      const fn = fnName && functionAt(fnName, fe.L);
      if (!fn) return { loc: fnLoc, hops: [...hops, "returned by"], stop: "returning function has no body here" };
      for (const r of ownReturns(fn, fe.L)) {
        const obj = unwrap(r.namedChild(0), fe.L);
        if (!obj || !F.object || obj.type !== F.object.type) continue;
        for (const prop of obj.namedChildren) {
          if (prop.type === F.object.shorthand && prop.text === key) {
            const next = (await lsp.definition(fnLoc.file, prop.startPosition.row, prop.startPosition.column))[0];
            if (next) return follow(lsp, next, depth + 1, [...hops, `returned as ${key}`]);
          }
          if (prop.type === F.object.pair.type && keyText(prop.childForFieldName(F.object.pair.key)) === key) {
            const v = unwrap(prop.childForFieldName(F.object.pair.value), fe.L);
            if (v && fe.L.functions.includes(v.type)) return { loc: { file: fnLoc.file, line: v.startPosition.row, col: v.startPosition.column }, hops: [...hops, `returned as ${key}`], stop: "function value" };
            const n = v && calleeName(v, fe.L);
            if (n) {
              const next = (await lsp.definition(fnLoc.file, n.startPosition.row, n.startPosition.column))[0];
              if (next) return follow(lsp, next, depth + 1, [...hops, `returned as ${key}`]);
            }
          }
        }
      }
      return { loc: fnLoc, hops: [...hops, "returned by"], stop: `no "${key}" found in what it returns` };
    }
  }

  if (p && F.parameter.includes(p.type)) return { loc, hops, stop: "parameter (value comes from the caller)" };
  return { loc, hops, stop: "declaration" };
}
