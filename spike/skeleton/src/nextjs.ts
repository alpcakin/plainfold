// Next.js framework adapter (app router): which files are screens and endpoints, and which URL reaches which.
import ts from "typescript";
import type { Project } from "./project.ts";

export type Route = {
  kind: "page" | "endpoint";
  path: string; // "/api/books", "/books/[id]"
  method?: string; // endpoints only
  file: string;
  decl: ts.FunctionDeclaration;
};

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

export function isExpo(p: Project): boolean {
  return p.sources.some((sf) => /^(?:src\/)?app\/_layout\.[tj]sx?$/.test(p.rel(sf)));
}

export function routes(p: Project): Route[] {
  const out: Route[] = [];
  const expo = isExpo(p);
  for (const sf of p.sources) {
    const file = p.rel(sf);
    if (expo) {
      // Expo Router: every file in app/ is a screen; "_layout" and "+…" files are not; "(group)" and "index" don't show in the URL.
      const m = /^(?:src\/)?app\/(.+)\.[tj]sx?$/.exec(file);
      if (!m || m[1].split("/").some((x) => x.startsWith("_") || x.startsWith("+"))) continue;
      const segs = m[1].split("/").filter((x) => x !== "index" && !/^\(.*\)$/.test(x));
      const decl = defaultExport(p, sf);
      if (decl) out.push({ kind: "page", path: "/" + segs.join("/"), file, decl });
      continue;
    }
    const m = /^(?:src\/)?app\/(.*?)(page|route)\.(tsx?|jsx?)$/.exec(file);
    if (!m) continue;
    const segs = m[1].split("/").filter((s) => s && !/^\(.*\)$/.test(s)); // route groups "(x)" don't change the URL
    const urlPath = "/" + segs.join("/");
    for (const st of sf.statements) {
      if (!ts.isFunctionDeclaration(st) || !st.name) continue;
      const mods = ts.getModifiers(st)?.map((x) => x.kind) ?? [];
      if (!mods.includes(ts.SyntaxKind.ExportKeyword)) continue;
      if (m[2] === "page" && mods.includes(ts.SyntaxKind.DefaultKeyword)) out.push({ kind: "page", path: urlPath, file, decl: st });
      if (m[2] === "route" && METHODS.includes(st.name.text)) out.push({ kind: "endpoint", path: urlPath, method: st.name.text, file, decl: st });
    }
  }
  return out;
}

// `export default function X() {}` or `export default X;` where X is a function declaration.
function defaultExport(p: Project, sf: ts.SourceFile): ts.FunctionDeclaration | undefined {
  for (const st of sf.statements) {
    if (ts.isFunctionDeclaration(st) && st.name && ts.getModifiers(st)?.some((x) => x.kind === ts.SyntaxKind.DefaultKeyword)) return st;
    if (ts.isExportAssignment(st) && !st.isExportEquals && ts.isIdentifier(st.expression)) {
      const d = p.checker.getSymbolAtLocation(st.expression)?.valueDeclaration;
      if (d && ts.isFunctionDeclaration(d) && d.name) return d;
    }
  }
  return undefined;
}

// Matches a URL written in the code ("/api/books?page=1", "/books/{…}") to a route.
// Literal segments must match exactly; "{…}" (a value only known at runtime) matches a dynamic segment.
export function matchRoute(all: Route[], url: string, kind: Route["kind"], method?: string): Route | undefined {
  const parts = url.split(/[?#]/)[0].split("/").filter(Boolean);
  const score = (r: Route) => {
    const rp = r.path.split("/").filter(Boolean);
    if (rp.length !== parts.length) return -1;
    let s = 0;
    for (let i = 0; i < rp.length; i++) {
      const dyn = /^\[.*\]$/.test(rp[i]);
      if (parts[i] === "{…}") { if (!dyn) return -1; }
      else if (dyn) s += 0;
      else if (rp[i] === parts[i]) s += 1;
      else return -1;
    }
    return s;
  };
  return all
    .filter((r) => r.kind === kind && (!method || r.method === method))
    .map((r) => [r, score(r)] as const)
    .filter(([, s]) => s >= 0)
    .sort((a, b) => b[1] - a[1])[0]?.[0];
}

// The URL text of a string or template literal, with runtime parts shown as "{…}".
export function urlText(e: ts.Expression): string | null {
  if (ts.isStringLiteral(e) || ts.isNoSubstitutionTemplateLiteral(e)) return e.text;
  if (ts.isTemplateExpression(e)) return e.head.text + e.templateSpans.map((s) => "{…}" + s.literal.text).join("");
  return null;
}
