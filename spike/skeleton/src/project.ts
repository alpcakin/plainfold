// Loads a TypeScript project with the compiler API (the engine behind tsserver) and offers small helpers.
import ts from "typescript";
import path from "node:path";
import type { Ref } from "./model.ts";

export type Project = {
  root: string;
  program: ts.Program;
  checker: ts.TypeChecker;
  sources: ts.SourceFile[];
  rel: (sf: ts.SourceFile) => string;
  ref: (node: ts.Node, end?: ts.Node) => Ref;
  text: (node: ts.Node) => string;
  isOwn: (node: ts.Node) => boolean;
};

export function loadProject(root: string): Project {
  root = path.resolve(root);
  const cfg = ts.readConfigFile(path.join(root, "tsconfig.json"), ts.sys.readFile);
  const parsed = ts.parseJsonConfigFileContent(cfg.config, ts.sys, root);
  const program = ts.createProgram(parsed.fileNames, { ...parsed.options, noEmit: true });
  const checker = program.getTypeChecker();
  const isOwnFile = (f: string) => f.startsWith(root + path.sep) && !f.includes(`${path.sep}node_modules${path.sep}`);
  const sources = program.getSourceFiles().filter((sf) => !sf.isDeclarationFile && isOwnFile(sf.fileName));
  const rel = (sf: ts.SourceFile) => path.relative(root, sf.fileName);
  const line = (sf: ts.SourceFile, pos: number) => sf.getLineAndCharacterOfPosition(pos).line + 1;
  return {
    root,
    program,
    checker,
    sources,
    rel,
    ref: (node, end = node) => {
      const sf = node.getSourceFile();
      return { file: rel(sf), from: line(sf, node.getStart(sf)), to: line(sf, end.getEnd()) };
    },
    text: (node) => node.getText(node.getSourceFile()),
    isOwn: (node) => {
      const sf = node.getSourceFile();
      return !sf.isDeclarationFile && isOwnFile(sf.fileName);
    },
  };
}

// Strips parentheses, await, `!` and type assertions.
export function unwrap(e: ts.Expression): ts.Expression {
  while (ts.isParenthesizedExpression(e) || ts.isAwaitExpression(e) || ts.isNonNullExpression(e) || ts.isAsExpression(e)) e = e.expression;
  return e;
}

// For an identifier, the symbol it finally points at (following imports).
export function resolve(checker: ts.TypeChecker, node: ts.Node): ts.Symbol | undefined {
  let sym = checker.getSymbolAtLocation(node);
  if (sym && sym.flags & ts.SymbolFlags.Alias) {
    const target = checker.getAliasedSymbol(sym);
    if (target && target.declarations?.length) sym = target;
  }
  return sym;
}

// If the identifier was imported from a package (not a project file), returns { module, name }.
// Works even when the package is not installed, because it reads the import statement itself.
export function importOf(checker: ts.TypeChecker, id: ts.Node): { module: string; name: string } | null {
  const sym = checker.getSymbolAtLocation(id);
  const decl = sym?.declarations?.[0];
  if (!decl) return null;
  let spec: ts.ImportSpecifier | ts.ImportClause | ts.NamespaceImport | undefined;
  if (ts.isImportSpecifier(decl) || ts.isImportClause(decl) || ts.isNamespaceImport(decl)) spec = decl;
  if (!spec) return null;
  let n: ts.Node = spec;
  while (n && !ts.isImportDeclaration(n)) n = n.parent;
  if (!n) return null;
  const module = ((n as ts.ImportDeclaration).moduleSpecifier as ts.StringLiteral).text;
  if (module.startsWith(".") || module.startsWith("@/")) return null;
  const name = ts.isImportSpecifier(spec) ? (spec.propertyName ?? spec.name).text : "default";
  return { module, name };
}

// `a.b(x).c(y)` -> root `a`, segments [b(x), c(y)]. Parentheses and await are looked through.
export type Segment = { name: string; args?: readonly ts.Expression[]; node: ts.Expression };
export function chain(e: ts.Expression): { root: ts.Expression; segs: Segment[] } {
  const segs: Segment[] = [];
  e = unwrap(e);
  for (;;) {
    if (ts.isCallExpression(e) && ts.isPropertyAccessExpression(unwrap(e.expression))) {
      const pa = unwrap(e.expression) as ts.PropertyAccessExpression;
      segs.unshift({ name: pa.name.text, args: e.arguments, node: e });
      e = unwrap(pa.expression);
    } else if (ts.isPropertyAccessExpression(e)) {
      segs.unshift({ name: e.name.text, node: e });
      e = unwrap(e.expression);
    } else break;
  }
  return { root: e, segs };
}

export function functionBody(decl: ts.Node): ts.ConciseBody | undefined {
  if (ts.isFunctionDeclaration(decl) || ts.isFunctionExpression(decl) || ts.isArrowFunction(decl) || ts.isMethodDeclaration(decl)) return decl.body;
  if (ts.isVariableDeclaration(decl) && decl.initializer) {
    const init = unwrap(decl.initializer);
    if (ts.isArrowFunction(init) || ts.isFunctionExpression(init)) return init.body;
  }
  return undefined;
}

export function isFunctionLike(n: ts.Node): n is ts.ArrowFunction | ts.FunctionExpression {
  return ts.isArrowFunction(n) || ts.isFunctionExpression(n);
}

// Code text on one line, with long runs squeezed.
export function oneLine(s: string, max = 0): string {
  const t = s.replace(/\s*\n\s*/g, " ").replace(/\(\s+/g, "(").replace(/\s+\)/g, ")").replace(/,\s*\)/g, ")").trim();
  return max && t.length > max ? t.slice(0, max - 1) + "…" : t;
}
