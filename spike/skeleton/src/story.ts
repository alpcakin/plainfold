// Builds one story (a trigger and everything after it) by walking the code from the entry point.
// Structure only: arrows, order, branches and loops come from the AST, the type checker and framework rules.
import ts from "typescript";
import { chain, functionBody, importOf, isFunctionLike, oneLine, resolve, unwrap, type Project, type Segment } from "./project.ts";
import { isExpo, matchRoute, routes, urlText, type Route } from "./nextjs.ts";
import type { Call, Coverage, Finding, Group, Item, Participant, ProjectModel, Ref, Story } from "./model.ts";

type Trigger = { attr: ts.JsxAttribute; event: string; handler: ts.Node; element: string; text: string };

type Ctx = {
  self: string; // participant doing the work
  group: Group;
  returnTo: string | null; // who receives `return` (null inside callbacks: their return goes nowhere visible)
  endpoint: boolean; // returns are HTTP responses
  env: Map<string, string>; // React state values known as code text; "?" = unknown
  stack: ts.Node[]; // functions being expanded, against cycles
  lane?: boolean; // runs inside the caller (same lifeline)
  hook?: boolean; // walking a custom hook: its return value is not a render
  inLoop?: boolean; // inside a repeated row
  quiet?: boolean; // drawing the screen: plain helper calls (formatting) are not part of the story
};

const PROMISE = new Set(["then", "catch", "finally"]);
const ITER = new Set(["map", "forEach", "flatMap", "filter", "find", "findIndex", "some", "every", "reduce"]);
const SUPABASE_CLIENTS = new Set(["createClient", "createBrowserClient", "createServerClient"]);
const DB_OPS = ["select", "insert", "update", "upsert", "delete"];

const boring = (items: Item[]) => items.every((i) => i.t === "self" && i.kind === "local");
const crosses = (items: Item[], parts: Map<string, Participant>): boolean =>
  items.some((i) =>
    i.t === "call" ? !["function", "component"].includes(parts.get(i.to)?.kind ?? "") || crosses(i.body ?? [], parts)
    : i.t === "self" ? i.kind === "reads"
    : i.t === "frame" ? crosses(i.items, parts) || crosses(i.else ?? [], parts)
    : false);
const blockOf = (s: ts.Statement): readonly ts.Statement[] => (ts.isBlock(s) ? s.statements : [s]);

export class StoryBuilder {
  p: Project;
  checker: ts.TypeChecker;
  routes: Route[];
  expo: boolean;
  parts = new Map<string, Participant>();
  signals: Finding[] = [];
  gaps: Finding[] = [];
  walked: { name: string; decl: ts.Node }[] = [];
  n = 0;
  callers: Map<ts.Node, Set<string>> | null = null;
  files = new Set<string>();
  meaningful = false;

  constructor(p: Project) {
    this.p = p;
    this.checker = p.program.getTypeChecker();
    this.routes = routes(p);
    this.expo = isExpo(p);
  }

  // ---------- entry: a screen opening ----------

  screen(route: Route): string {
    const group = this.pageGroup(route.decl);
    return this.part({ id: this.pid(route.decl), name: route.decl.name!.text, kind: "screen", group, sub: `${route.path} · ${route.file}`, ref: this.p.ref(route.decl) });
  }

  pageGroup(decl: ts.Node): Group {
    if (this.expo) return "device";
    return /^\s*["']use client["']/.test(decl.getSourceFile().text) ? "browser" : "server";
  }

  screenOpens(route: Route): Story {
    const decl = route.decl;
    const self = this.screen(route);
    const ctx: Ctx = { self, group: this.pageGroup(decl), returnTo: null, endpoint: false, env: new Map(), stack: [decl] };
    const items = this.component(decl, ctx);
    this.walked.unshift({ name: decl.name!.text, decl });
    return this.finish(`open:${route.path}`, `${decl.name!.text} opens`, { kind: "screen-open", route: route.path, ref: this.p.ref(decl) }, items);
  }

  // ---------- entry: a user action on a screen ----------

  // onClick / onSubmit / <form action> handlers in a screen's JSX, with the text people see on the control.
  triggers(decl: ts.Node): Trigger[] {
    const out: Trigger[] = [];
    const visit = (n: ts.Node) => {
      if (ts.isJsxAttribute(n) && n.initializer && ts.isJsxExpression(n.initializer) && n.initializer.expression) {
        const name = n.name.getText();
        const el = n.parent.parent as ts.JsxOpeningLikeElement;
        const ev = name === "action" || name === "onSubmit" ? "submit" : /^on[A-Z]/.test(name) ? name.slice(2).toLowerCase() : null;
        const v = unwrap(n.initializer.expression);
        const handler = isFunctionLike(v) ? v : ts.isIdentifier(v) ? this.projectDecl(v) : null;
        if (ev && handler && ev !== "change") {
          const holder = ts.isJsxOpeningElement(el) ? el.parent : el;
          const button = ev === "submit" ? this.find(holder, (x) => (ts.isJsxElement(x) && x.openingElement.tagName.getText() === "button") || false) : holder;
          out.push({ attr: n, event: ev, handler, element: `<${el.tagName.getText()} ${oneLine(this.p.text(n))}>`, text: button && ts.isJsxElement(button) ? this.jsxText(button) : "" });
        }
      }
      ts.forEachChild(n, visit);
    };
    visit(decl);
    return out;
  }

  userAction(route: Route, t: Trigger): Story {
    const decl = route.decl;
    const self = this.screen(route);
    const ctx: Ctx = { self, group: this.pageGroup(decl), returnTo: null, endpoint: false, env: new Map(), stack: [decl, t.handler] };
    const items: Item[] = [
      { t: "self", id: this.id(), at: self, kind: "trigger", label: `${t.event} ${t.element}${t.text ? ` · “${t.text}”` : ""}`, detail: oneLine(this.p.text(t.attr.parent.parent)), ref: this.p.ref(t.attr) },
      ...this.callback(t.handler, ctx),
    ];
    this.navigation(t.handler);
    const hname = isFunctionLike(t.handler) ? "" : this.declName(t.handler);
    if (hname) this.walked.push({ name: hname, decl: t.handler });
    const title = t.text ? `“${t.text}” on ${decl.name!.text}` : `${t.element} on ${decl.name!.text}`;
    return this.finish(`action:${route.path}#${hname || t.event}`, title, { kind: "user-action", route: route.path, ref: this.p.ref(t.attr), event: /^on[A-Z]/.test(t.attr.name.getText()) ? t.attr.name.getText().slice(2, 3).toLowerCase() + t.attr.name.getText().slice(3) : t.event, control: /^<([\w.]+)/.exec(t.element)?.[1], text: t.text || undefined, handler: hname || undefined }, items);
  }

  find(n: ts.Node, test: (x: ts.Node) => boolean): ts.Node | undefined {
    if (test(n)) return n;
    return ts.forEachChild(n, (c) => this.find(c, test));
  }

  jsxText(e: ts.JsxElement): string {
    return oneLine(e.children.filter(ts.isJsxText).map((c) => c.text.trim()).filter(Boolean).join(" "), 32);
  }

  // ---------- statements ----------

  stmts(list: readonly ts.Statement[], ctx: Ctx): Item[] {
    const out: Item[] = [];
    let pending: ts.Statement[] = [];
    const flush = () => {
      if (pending.length) out.push(this.local(pending, ctx));
      pending = [];
    };
    const add = (st: ts.Statement, items: Item[]) => {
      if (items.length) { flush(); out.push(...items); } else pending.push(st);
    };
    for (const st of list) {
      if (ts.isReturnStatement(st)) { add(st, this.ret(st, ctx)); break; }
      if (ts.isIfStatement(st)) {
        const cond = this.expr(st.expression, ctx);
        const then = this.stmts(blockOf(st.thenStatement), ctx);
        const els = st.elseStatement ? this.stmts(blockOf(st.elseStatement), ctx) : [];
        if (cond.length || !boring(then) || !boring(els)) {
          flush();
          out.push(...cond, { t: "frame", id: this.id(), kind: "alt", label: oneLine(this.p.text(st.expression)), ref: this.p.ref(st), items: then, else: els.length ? els : undefined });
        } else pending.push(st);
        continue;
      }
      if (ts.isIterationStatement(st, false)) {
        const body = this.stmts(blockOf(st.statement), ctx);
        if (!boring(body)) {
          flush();
          const head = this.p.text(st).split("{")[0].trim();
          out.push({ t: "frame", id: this.id(), kind: "loop", label: oneLine(head, 48), ref: this.p.ref(st), items: body });
        } else pending.push(st);
        continue;
      }
      if (ts.isTryStatement(st)) {
        flush();
        out.push(...this.stmts(st.tryBlock.statements, ctx));
        if (st.catchClause) {
          const c = this.stmts(st.catchClause.block.statements, ctx);
          if (!boring(c)) out.push({ t: "frame", id: this.id(), kind: "alt", label: "catch", ref: this.p.ref(st.catchClause), items: c });
          else pending.push(st);
        }
        continue;
      }
      if (ts.isBlock(st)) { flush(); out.push(...this.stmts(st.statements, ctx)); continue; }
      if (ts.isVariableStatement(st)) {
        add(st, st.declarationList.declarations.flatMap((d) => (d.initializer ? this.expr(d.initializer, ctx) : [])));
        continue;
      }
      if (ts.isExpressionStatement(st)) { add(st, this.expr(st.expression, ctx)); continue; }
      pending.push(st);
    }
    flush();
    return out;
  }

  // Plain statements that don't cross a boundary, folded into one step named after what they declare.
  local(list: ts.Statement[], ctx: Ctx): Item {
    const declared = list.filter(ts.isVariableStatement).flatMap((st) => st.declarationList.declarations.flatMap((d) => this.names(d.name)));
    const bits = list.map((st, i) => {
      if (ts.isVariableStatement(st)) return list.findIndex((x) => ts.isVariableStatement(x)) === i ? `const ${declared.join(", ")}` : "";
      if (ts.isIfStatement(st)) return `if (${oneLine(this.p.text(st.expression), 30)}) …`;
      if (ts.isExpressionStatement(st)) return oneLine(this.p.text(st.expression), 40);
      return oneLine(this.p.text(st), 40);
    }).filter(Boolean);
    return {
      t: "self", id: this.id(), at: ctx.self, kind: "local",
      label: oneLine(bits.join("; "), 56),
      detail: list.map((s) => oneLine(this.p.text(s))).join("\n"),
      ref: this.p.ref(list[0], list[list.length - 1]),
    };
  }

  names(b: ts.BindingName): string[] {
    if (ts.isIdentifier(b)) return [b.text];
    return b.elements.flatMap((e) => (ts.isOmittedExpression(e) ? [] : this.names(e.name)));
  }

  ret(st: ts.ReturnStatement, ctx: Ctx): Item[] {
    const e = st.expression ? unwrap(st.expression) : undefined;
    const ref = this.p.ref(st);
    if (ctx.endpoint && e) {
      const r = this.response(e);
      if (r) {
        const pre = r.args.flatMap((a) => this.expr(a, ctx));
        return [...pre, { t: "return", id: this.id(), from: ctx.self, to: ctx.returnTo ?? "caller", label: r.label, detail: oneLine(this.p.text(st)), ref }];
      }
    }
    const pre = e ? this.expr(e, ctx) : [];
    if (!ctx.returnTo || ctx.returnTo === ctx.self) return pre;
    return [...pre, { t: "return", id: this.id(), from: ctx.self, to: ctx.returnTo, label: e ? this.short(e) : "return", detail: oneLine(this.p.text(st)), ref }];
  }

  // NextResponse.json(body, { status }), Response.json(...), new Response(...), NextResponse.redirect(url)
  response(e: ts.Expression): { label: string; args: readonly ts.Expression[] } | null {
    let args: readonly ts.Expression[] | undefined;
    let redirect = false;
    if (ts.isNewExpression(e) && ts.isIdentifier(e.expression) && ["Response", "NextResponse"].includes(e.expression.text)) args = e.arguments ?? [];
    if (ts.isCallExpression(e) && ts.isPropertyAccessExpression(e.expression) && ts.isIdentifier(e.expression.expression)) {
      const obj = e.expression.expression.text, fn = e.expression.name.text;
      const imp = importOf(this.checker, e.expression.expression);
      if ((obj === "Response" || imp?.module === "next/server") && ["json", "redirect"].includes(fn)) { args = e.arguments; redirect = fn === "redirect"; }
    }
    if (!args) return null;
    const init = args[1] && unwrap(args[1]);
    let status = redirect ? "307" : "200";
    if (init && ts.isObjectLiteralExpression(init)) {
      for (const pr of init.properties) if (ts.isPropertyAssignment(pr) && pr.name.getText() === "status") status = this.p.text(pr.initializer);
    }
    if (redirect && init && ts.isNumericLiteral(init)) status = init.text;
    const body = args[0] ? this.short(unwrap(args[0])) : "";
    return { label: oneLine(`${redirect ? "redirect" : status} ${body}`), args };
  }

  // A value as a short label: `{ error: "Not logged in" }`, `{ books, page }`, `null`.
  short(e: ts.Expression): string {
    e = unwrap(e);
    const full = oneLine(this.p.text(e));
    if (ts.isObjectLiteralExpression(e) && full.length > 36) {
      const keys = e.properties.map((pr) =>
        ts.isSpreadAssignment(pr) ? `...${oneLine(this.p.text(pr.expression), 12)}`
        : ts.isPropertyAssignment(pr) && ts.isStringLiteral(pr.initializer) ? `${pr.name.getText()}: ${pr.initializer.getText()}`
        : pr.name ? pr.name.getText() : "…");
      return `{ ${keys.join(", ")} }`;
    }
    return oneLine(full, 40);
  }

  // ---------- expressions ----------

  expr(e: ts.Node | undefined, ctx: Ctx): Item[] {
    if (!e || isFunctionLike(e) || ts.isFunctionDeclaration(e)) return [];
    if (ts.isJsxElement(e) || ts.isJsxSelfClosingElement(e) || ts.isJsxFragment(e)) return this.jsx(e, ctx);
    if (ts.isCallExpression(e)) {
      const r = this.call(e, ctx);
      if (r) return r;
    }
    if (ts.isConditionalExpression(e) || (ts.isBinaryExpression(e) && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken].includes(e.operatorToken.kind))) {
      const cond = ts.isConditionalExpression(e) ? e.condition : e.left;
      const a = ts.isConditionalExpression(e) ? e.whenTrue : e.right;
      const b = ts.isConditionalExpression(e) ? e.whenFalse : undefined;
      const pre = this.expr(cond, ctx), then = this.expr(a, ctx), els = this.expr(b, ctx);
      if (boring(then) && boring(els)) return pre;
      const op = ts.isBinaryExpression(e) && e.operatorToken.kind !== ts.SyntaxKind.AmpersandAmpersandToken ? `not (${oneLine(this.p.text(cond))})` : oneLine(this.p.text(cond));
      return [...pre, { t: "frame", id: this.id(), kind: "alt", label: op, ref: this.p.ref(e), items: then, else: els.length ? els : undefined }];
    }
    const out: Item[] = [];
    ts.forEachChild(e, (c) => { out.push(...this.expr(c, ctx)); });
    return out;
  }

  call(e: ts.CallExpression, ctx: Ctx): Item[] | null {
    const callee = unwrap(e.expression);
    if (isFunctionLike(callee)) return this.callback(callee, ctx);
    const { root, segs } = chain(e);
    const last = segs[segs.length - 1];
    if (last && last.node === e) {
      if (PROMISE.has(last.name)) return this.promise(e, ctx);
      if (ITER.has(last.name) && last.args?.[0] && isFunctionLike(last.args[0])) {
        const r = this.loop(e, last, ctx);
        if (r) return r;
      }
    }
    if (ts.isIdentifier(root) && segs.length && this.supabaseClient(root)) return this.supabase(e, root, segs, ctx);
    if (ts.isIdentifier(root) && segs.length === 1 && ["push", "replace"].includes(segs[0].name) && this.isRouter(root)) return this.navigate(e, segs[0], ctx);
    if (ts.isCallExpression(root) && ts.isIdentifier(root.expression)) {
      const id = root.expression;
      const imp = importOf(this.checker, id);
      if (imp?.module === "next/headers" && ["cookies", "headers"].includes(imp.name)) {
        const label = id.text + "()" + segs.map((s) => this.segText(s)).join("");
        return [{ t: "self", id: this.id(), at: ctx.self, kind: "reads", label: oneLine(label, 48), detail: `reads the incoming request (${imp.module})`, ref: this.p.ref(e) }];
      }
      if (root === e) {
        if (!imp && id.text === "fetch" && !this.projectDecl(id)) return this.fetch(e, ctx);
        const st = this.stateOf(id);
        if (st?.setter) return [this.setState(e, st.state, ctx)];
      }
    }
    const decl = this.target(e.expression);
    if (decl) return this.projectCall(e, decl, ctx);
    return null;
  }

  segText(s: Segment): string {
    return "." + s.name + (s.args ? `(${s.args.map((a) => oneLine(this.p.text(a))).join(", ")})` : "");
  }

  // fetch(...).then(...).then(...).finally(...): the base call first, then each handler in order.
  promise(e: ts.CallExpression, ctx: Ctx): Item[] {
    const handlers: { name: string; fn?: ts.Expression }[] = [];
    let cur: ts.Expression = e;
    while (ts.isCallExpression(cur) && ts.isPropertyAccessExpression(cur.expression) && PROMISE.has(cur.expression.name.text)) {
      handlers.unshift({ name: cur.expression.name.text, fn: cur.arguments[0] });
      cur = unwrap(cur.expression.expression);
    }
    const out = this.expr(cur, ctx);
    for (const h of handlers) {
      if (!h.fn || !isFunctionLike(h.fn)) continue;
      const items = this.callback(h.fn, { ...ctx, returnTo: null });
      if (h.name === "catch") out.push({ t: "frame", id: this.id(), kind: "alt", label: "catch", ref: this.p.ref(h.fn), items });
      else out.push(...items);
    }
    return out;
  }

  // Is the answer's status read, and is a network failure caught? Either as fetch(…).then(res => …).catch(…) or as const res = await fetch(…) inside try.
  checkAnswer(e: ts.CallExpression, call: Call) {
    let checked = false, caught = false, res: string | null = null, scope: ts.Node | undefined;
    let cur: ts.Node = e;
    while (ts.isPropertyAccessExpression(cur.parent) && PROMISE.has(cur.parent.name.text) && ts.isCallExpression(cur.parent.parent)) {
      const name = cur.parent.name.text, fn = cur.parent.parent.arguments[0];
      if (name === "catch") caught = true;
      if (name === "then" && !res && fn && isFunctionLike(fn) && fn.parameters[0] && ts.isIdentifier(fn.parameters[0].name)) { res = fn.parameters[0].name.text; scope = fn; }
      cur = cur.parent.parent;
    }
    if (!res) {
      let n: ts.Node = e;
      while (ts.isAwaitExpression(n.parent) || ts.isParenthesizedExpression(n.parent)) n = n.parent;
      if (ts.isVariableDeclaration(n.parent) && ts.isIdentifier(n.parent.name)) { res = n.parent.name.text; scope = this.enclosing(e); }
    }
    if (res && scope) checked = this.readsStatus(scope, res);
    for (let n: ts.Node = e; n && !ts.isFunctionLike(n); n = n.parent) if (ts.isTryStatement(n.parent) && n.parent.tryBlock === n) caught = true;
    const codes = [...new Set(this.returnLabels(call.body ?? [], call.to).map((l) => /^([45]\d\d)\b/.exec(l)?.[1]).filter(Boolean))];
    const text = `${e.expression.getText()}(${e.arguments.map((a) => this.short(a)).join(", ")})`;
    if (!checked && !caught) this.signal(`${text} never reads res.ok or res.status and has no catch: error answers${codes.length ? ` (${codes.join(", ")})` : ""} and network failures are handled like success`, this.p.ref(e));
    else if (!checked) this.signal(`${text} never reads res.ok or res.status: error answers${codes.length ? ` (${codes.join(", ")})` : ""} are handled like success`, this.p.ref(e));
    else if (!caught) this.signal(`${text} has no catch: a network failure is not handled here`, this.p.ref(e));
  }

  returnLabels(items: Item[], from: string): string[] {
    return items.flatMap((i) => (i.t === "return" && i.from === from ? [i.label] : i.t === "frame" ? this.returnLabels([...i.items, ...(i.else ?? [])], from) : []));
  }

  enclosing(n: ts.Node): ts.Node | undefined {
    for (let p = n.parent; p; p = p.parent) if (ts.isFunctionLike(p)) return p;
    return undefined;
  }

  readsStatus(scope: ts.Node, name: string): boolean {
    let found = false;
    const visit = (n: ts.Node) => {
      if (ts.isPropertyAccessExpression(n) && ts.isIdentifier(n.expression) && n.expression.text === name && ["ok", "status"].includes(n.name.text)) found = true;
      if (!found) ts.forEachChild(n, visit);
    };
    visit(scope);
    return found;
  }

  // The body of a function, callback or handler, walked as the current participant.
  callback(fn: ts.Node, ctx: Ctx): Item[] {
    const body = functionBody(fn);
    if (!body) return [];
    if (ts.isBlock(body)) return this.stmts(body.statements, ctx);
    const items = this.expr(body, ctx);
    if (items.length) return items;
    return [{ t: "self", id: this.id(), at: ctx.self, kind: "local", label: oneLine(this.p.text(body), 48), detail: oneLine(this.p.text(fn)), ref: this.p.ref(fn) }];
  }

  // xs.map((x) => …) with something worth showing inside becomes a loop frame.
  loop(e: ts.CallExpression, last: Segment, ctx: Ctx): Item[] | null {
    const fn = last.args![0] as ts.ArrowFunction;
    const inner = this.callback(fn, { ...ctx, returnTo: null, inLoop: true });
    if (boring(inner)) return null;
    const target = (e.expression as ts.PropertyAccessExpression).expression;
    const param = fn.parameters.map((x) => x.name.getText()).join(", ");
    return [...this.expr(target, ctx), { t: "frame", id: this.id(), kind: "loop", label: `${oneLine(this.p.text(target), 24)}.${last.name}(${param})`, ref: this.p.ref(e), items: inner }];
  }

  // ---------- boundaries ----------

  fetch(e: ts.CallExpression, ctx: Ctx): Item[] {
    const url = e.arguments[0] ? urlText(unwrap(e.arguments[0])) : null;
    let method = "GET";
    const init = e.arguments[1] && unwrap(e.arguments[1]);
    if (init && ts.isObjectLiteralExpression(init))
      for (const pr of init.properties) if (ts.isPropertyAssignment(pr) && pr.name.getText() === "method" && ts.isStringLiteral(pr.initializer)) method = pr.initializer.text.toUpperCase();
    const ref = this.p.ref(e);
    const detail = oneLine(this.p.text(e));
    if (url?.startsWith("/")) {
      const route = matchRoute(this.routes, url, "endpoint", method);
      if (route) {
        const to = this.part({ id: `${method} ${route.path}`, name: `${method} ${route.path}`, kind: "endpoint", group: "server", sub: route.file, ref: this.p.ref(route.decl) });
        const call: Call = { t: "call", id: this.id(), from: ctx.self, to, label: `${method} ${url}`, detail, ref, target: this.p.ref(route.decl),
          link: { kind: "exact", how: `Next.js routing: ${route.path} → ${route.file}, export ${method}` } };
        if (!ctx.stack.includes(route.decl)) {
          const inner: Ctx = { self: to, group: "server", returnTo: ctx.self, endpoint: true, env: new Map(), stack: [...ctx.stack, route.decl] };
          call.body = this.callback(route.decl, inner);
          call.touches = this.touched(call.body);
          this.walked.push({ name: `${method} ${route.path}`, decl: route.decl });
        }
        this.checkAnswer(e, call);
        return [call];
      }
      this.gap(`${method} ${url}: no file in app/ answers this URL`, ref);
    }
    const host = url && /^https?:\/\//.test(url) ? new URL(url.replace(/\{…\}/g, "x")).host : null;
    const to = this.part({ id: `out:${host ?? "?"}`, name: host ?? "unknown address", kind: "service", group: "outside" });
    if (!host) this.gap(`${detail}: the address is only known at runtime`, ref);
    return [{ t: "call", id: this.id(), from: ctx.self, to, label: `${method} ${url ?? "…"}`, detail, ref, link: host ? { kind: "exact", how: "URL written in the code" } : { kind: "inferred", how: "address not known statically" } }];
  }

  supabaseClient(id: ts.Identifier): ts.VariableDeclaration | null {
    const sym = resolve(this.checker, id);
    const d = sym?.valueDeclaration;
    if (!d || !ts.isVariableDeclaration(d) || !d.initializer) return null;
    const init = unwrap(d.initializer);
    if (!ts.isCallExpression(init) || !ts.isIdentifier(init.expression)) return null;
    const imp = importOf(this.checker, init.expression);
    return imp && imp.module.startsWith("@supabase/") && SUPABASE_CLIENTS.has(imp.name) ? d : null;
  }

  supabase(e: ts.CallExpression, root: ts.Identifier, segs: Segment[], ctx: Ctx): Item[] {
    const client = this.supabaseClient(root)!;
    const clientAt = `${this.p.rel(client.getSourceFile())}:${this.p.ref(client).from}`;
    const ref = this.p.ref(e);
    const detail = oneLine(this.p.text(e));
    const first = segs[0];
    let to: string, label: string, how: string, effect: "reads" | "writes" | "deletes" | undefined, table: string | undefined;
    if (first.name === "from" && first.args?.[0] && ts.isStringLiteral(first.args[0])) {
      const table0 = first.args[0].text;
      const tableName = table0;
      const db = this.parts.get("db");
      const tables = new Set((db?.sub ?? "").split(", ").filter(Boolean)).add(tableName);
      to = this.part({ id: "db", name: "database", kind: "database", group: "supabase" });
      this.parts.get("db")!.sub = [...tables].join(", ");
      const op = segs.slice(1).find((s) => DB_OPS.includes(s.name));
      effect = !op || op.name === "select" ? "reads" : op.name === "delete" ? "deletes" : "writes";
      const opText = op ? `.${op.name}(${(op.args ?? []).map((a) => this.short(a)).join(", ")})` : "";
      const more = segs.slice(1).filter((s) => s !== op && !["select", "single", "maybeSingle"].includes(s.name)).length > 0;
      table = table0;
      label = this.segText(first).slice(1) + opText + (more ? " …" : "");
      how = `Supabase client from ${clientAt}, table "${tableName}"`;
    } else if (first.name === "rpc" && first.args?.[0] && ts.isStringLiteral(first.args[0])) {
      to = this.part({ id: "db", name: "database", kind: "database", group: "supabase" });
      label = `rpc("${first.args[0].text}")`;
      table = first.args[0].text;
      how = `Supabase client from ${clientAt}, database function "${first.args[0].text}"`;
    } else if (first.name === "auth") {
      to = this.part({ id: "auth", name: "Supabase Auth", kind: "auth", group: "supabase" });
      label = segs.map((s) => this.segText(s)).join("").slice(1);
      how = `Supabase client from ${clientAt}, auth`;
    } else if (first.name === "functions" && segs[1]?.name === "invoke" && segs[1].args?.[0] && ts.isStringLiteral(segs[1].args[0])) {
      const fn = segs[1].args[0].text;
      to = this.part({ id: `edge:${fn}`, name: fn, kind: "service", group: "supabase", sub: "edge function" });
      label = `functions.invoke("${fn}")`;
      how = `Supabase client from ${clientAt}, edge function by name`;
    } else {
      to = this.part({ id: "supabase", name: "Supabase", kind: "service", group: "supabase" });
      label = segs.map((s) => this.segText(s)).join("").slice(1);
      how = `Supabase client from ${clientAt}`;
      this.gap(`${oneLine(label, 40)}: this Supabase call is not understood yet`, ref);
    }
    const args = segs.flatMap((s) => s.args ?? []).flatMap((a) => this.expr(a, ctx));
    return [
      ...args,
      { t: "call", id: this.id(), from: ctx.self, to, label: oneLine(label, 60), detail, ref, effect, table, link: { kind: "exact", how } },
      { t: "return", id: this.id(), from: to, to: ctx.self, label: this.receiver(e), ref },
    ];
  }

  // What the code calls the result: `const { data, error } = await …` -> "{ data, error }".
  receiver(e: ts.Expression): string {
    let n: ts.Node = e;
    while (ts.isAwaitExpression(n.parent) || ts.isParenthesizedExpression(n.parent)) n = n.parent;
    const parent = n.parent;
    if (ts.isVariableDeclaration(parent)) return oneLine(parent.name.getText());
    return "result";
  }

  // ---------- project functions and components ----------

  projectDecl(id: ts.Node): ts.Node | null {
    return this.fromSymbol(resolve(this.checker, id));
  }

  // The project function a callee expression finally points at: through imports, aliases,
  // destructured hook results (`const { refresh } = useX()`), object members (`list.refresh`) and refs (`ref.current`).
  target(x: ts.Expression): ts.Node | null {
    x = unwrap(x);
    if (ts.isIdentifier(x)) return this.fromSymbol(resolve(this.checker, x));
    if (!ts.isPropertyAccessExpression(x)) return null;
    if (x.name.text === "current" && ts.isIdentifier(x.expression)) {
      const d = resolve(this.checker, x.expression)?.valueDeclaration;
      const init = d && ts.isVariableDeclaration(d) && d.initializer ? unwrap(d.initializer) : undefined;
      if (init && ts.isCallExpression(init) && this.isReact(init.expression, "useRef") && init.arguments[0]) return this.target(init.arguments[0]);
    }
    return this.fromSymbol(this.checker.getSymbolAtLocation(x.name));
  }

  fromSymbol(sym: ts.Symbol | undefined, depth = 0): ts.Node | null {
    const d = sym?.valueDeclaration ?? sym?.declarations?.[0];
    if (!d || depth > 6) return null;
    if (this.p.isOwn(d) && functionBody(d)) return d;
    if (ts.isShorthandPropertyAssignment(d)) return this.fromSymbol(this.checker.getShorthandAssignmentValueSymbol(d), depth + 1);
    if (ts.isPropertyAssignment(d)) {
      const v = unwrap(d.initializer);
      if (isFunctionLike(v)) return v;
      return ts.isIdentifier(v) ? this.fromSymbol(resolve(this.checker, v), depth + 1) : null;
    }
    if (ts.isVariableDeclaration(d) && d.initializer) {
      const v = unwrap(d.initializer);
      if (ts.isCallExpression(v) && this.isReact(v.expression, "useCallback") && v.arguments[0]) {
        const fn = unwrap(v.arguments[0]);
        return isFunctionLike(fn) ? fn : null;
      }
      return ts.isIdentifier(v) ? this.fromSymbol(resolve(this.checker, v), depth + 1) : null;
    }
    if (ts.isBindingElement(d) && ts.isObjectBindingPattern(d.parent) && ts.isVariableDeclaration(d.parent.parent) && d.parent.parent.initializer) {
      const type = this.checker.getTypeAtLocation(d.parent.parent.initializer);
      const name = (d.propertyName ?? d.name) as ts.Identifier;
      return ts.isIdentifier(name) ? this.fromSymbol(this.checker.getPropertyOfType(type, name.text), depth + 1) : null;
    }
    return null;
  }

  projectCall(e: ts.CallExpression, decl: ts.Node, ctx: Ctx): Item[] {
    // A function declared inside the current function is part of it, not a separate participant.
    if (this.nestedIn(decl, ctx.stack)) return this.callback(decl, ctx);
    const name = this.declName(decl);
    const hookLike = /^use[A-Z]/.test(name);
    // A hook, or a function from one, runs inside the screen: same lane, shown as a foldable row, not a participant.
    const sameLane = ctx.group === "device" && (hookLike || ctx.hook || ctx.lane || this.insideHook(decl));
    const to = sameLane ? ctx.self : this.part({ id: this.pid(decl), name, kind: "function", group: ctx.group, sub: this.p.rel(decl.getSourceFile()), ref: this.p.ref(decl) });
    const call: Call = { t: "call", id: this.id(), from: ctx.self, to, label: oneLine(this.p.text(e), 40), detail: oneLine(this.p.text(e)), ref: this.p.ref(e), target: this.p.ref(decl),
      link: { kind: "exact", how: `TypeScript: resolved to ${this.p.rel(decl.getSourceFile())}:${this.p.ref(decl).from}` } };
    if (ctx.stack.includes(decl)) return [call];
    const hook = /^use[A-Z]/.test(name) && ts.isBlock(functionBody(decl)!);
    const inner = this.trial(() => {
      const c: Ctx = { self: to, group: ctx.group, returnTo: ctx.self, endpoint: false, env: new Map(), stack: [...ctx.stack, decl], hook, lane: sameLane, quiet: ctx.quiet || sameLane, inLoop: ctx.inLoop };
      return hook ? this.component(decl, c) : this.callback(decl, c);
    });
    if (!inner && hookLike) { if (!sameLane) this.parts.delete(to); return []; }
    if (inner) {
      this.walked.push({ name, decl });
      const others = [...(this.callersOf(decl))].filter((c) => c !== this.parts.get(ctx.self)?.name);
      call.body = inner;
      call.touches = this.touched(inner);
      if (others.length) call.note = `shared · also called from ${others.join(", ")}`;
      return [call];
    }
    // Pure helper: one arrow there and one back, typed by the checker. Helpers that return nothing (logging)
    // and plain formatting while the screen is drawn say nothing about the story.
    const sig = this.checker.getSignatureFromDeclaration(decl as ts.SignatureDeclaration);
    let type = sig ? this.checker.typeToString(this.checker.getReturnTypeOfSignature(sig)) : "result";
    if (ctx.quiet || /^(Promise<)?void>?$/.test(type)) { if (!sameLane) this.parts.delete(to); return []; }
    if (ts.isAwaitExpression(e.parent)) type = type.replace(/^Promise<(.*)>$/, "$1");
    return [call, { t: "return", id: this.id(), from: to, to: ctx.self, label: type, detail: "return type from the type checker", ref: this.p.ref(decl) }];
  }

  // "reads items", "writes books", "calls count_items": what a folded call goes on to touch.
  touched(items: Item[]): string[] {
    const out = new Set<string>();
    const w = (list: Item[]) => { for (const i of list) { if (i.t === "call" && i.table) out.add(`${i.effect ?? "calls"} ${i.table}`); if (i.t === "call" && i.body) w(i.body); if (i.t === "frame") { w(i.items); w(i.else ?? []); } } };
    w(items);
    return [...out];
  }

  // Walks a callee; keeps the result only if it crosses a boundary (network, database, request input).
  trial(walk: () => Item[]): Item[] | null {
    const parts = new Set(this.parts.keys()), walked = this.walked.length;
    const items = walk();
    if (crosses(items, this.parts)) return items;
    for (const k of this.parts.keys()) if (!parts.has(k)) this.parts.delete(k);
    this.walked.length = walked;
    return null;
  }

  // React component: state, first render, effects, then the render that follows the state changes.
  component(decl: ts.Node, ctx: Ctx): Item[] {
    const body = functionBody(decl);
    if (!body || !ts.isBlock(body)) return [];
    this.navigation(decl);
    const states: { st: ts.Statement; name: string; init: string }[] = [];
    const effects: ts.CallExpression[] = [];
    const rest: ts.Statement[] = [];
    for (const st of body.statements) {
      const d = ts.isVariableStatement(st) ? st.declarationList.declarations[0] : undefined;
      const init = d?.initializer && unwrap(d.initializer);
      if (d && init && ts.isCallExpression(init) && this.isReact(init.expression, "useState") && ts.isArrayBindingPattern(d.name)) {
        states.push({ st, name: d.name.elements[0].getText(), init: init.arguments[0] ? oneLine(this.p.text(init.arguments[0])) : "undefined" });
        continue;
      }
      const ex = ts.isExpressionStatement(st) ? unwrap(st.expression) : undefined;
      if (ex && ts.isCallExpression(ex) && (this.isReact(ex.expression, "useEffect") || this.isReact(ex.expression, "useLayoutEffect") || this.isFocusEffect(ex.expression))) { effects.push(ex); continue; }
      rest.push(st);
    }
    const env = ctx.env;
    for (const s of states) env.set(s.name, s.init);
    const out: Item[] = [];
    if (states.length)
      out.push({ t: "self", id: this.id(), at: ctx.self, kind: "state", label: oneLine(`useState: ${states.map((s) => `${s.name} = ${s.init}`).join(", ")}`, 56), ref: this.p.ref(states[0].st, states[states.length - 1].st) });
    out.push(...this.render(rest, ctx, 1));
    const before = JSON.stringify([...env]);
    for (const ef of effects) {
      const focus = this.isFocusEffect(ef.expression);
      let fn: ts.Expression | undefined = ef.arguments[0] && unwrap(ef.arguments[0]);
      let depsNode: ts.Expression | undefined = ef.arguments[1];
      if (focus && fn && ts.isCallExpression(fn) && this.isReact(fn.expression, "useCallback")) { depsNode = fn.arguments[1]; fn = fn.arguments[0] && unwrap(fn.arguments[0]); }
      if (!fn || !isFunctionLike(fn)) continue;
      const deps = depsNode ? oneLine(this.p.text(depsNode)) : "";
      const when = focus ? "each time the screen comes into view" : deps === "[]" ? "after first render" : deps ? `after first render, again when ${deps} changes` : "after every render";
      const items = this.callback(fn, { ...ctx, returnTo: null });
      if (!items.length || boring(items)) continue; // effects that touch nothing outside the screen are not part of the story
      out.push({ t: "frame", id: this.id(), kind: "effect", label: `${focus ? "useFocusEffect" : "useEffect"}(…${deps ? ", " + deps : ""})`, note: when, ref: this.p.ref(ef), items });
    }
    if (JSON.stringify([...env]) !== before) {
      const seen = new Set<string>();
      const mark = (list: Item[]) => { for (const i of list) { seen.add(`${i.t}:${i.ref.file}:${i.ref.from}:${i.ref.to}`); if (i.t === "frame") { mark(i.items); mark(i.else ?? []); } if (i.t === "call" && i.body) mark(i.body); } };
      mark(out);
      const fresh = (list: Item[]): Item[] => list.filter((i) => !seen.has(`${i.t}:${i.ref.file}:${i.ref.from}:${i.ref.to}`));
      const again = fresh(this.render(rest, ctx, 2));
      if (again.some((i) => i.t !== "self")) out.push(...again);
    }
    return out;
  }

  // One render pass. Conditions on state with a known value are decided; the rest become alt frames.
  render(list: readonly ts.Statement[], ctx: Ctx, pass: number): Item[] {
    const out: Item[] = [];
    const decided: string[] = [];
    const walk = (stmts: readonly ts.Statement[]): boolean => {
      for (const st of stmts) {
        if (ts.isIfStatement(st)) {
          const v = this.decide(st.expression, ctx.env);
          if (v === undefined) {
            const then: Item[] = [], els: Item[] = [];
            const keep = out.length;
            walk(blockOf(st.thenStatement)); then.push(...out.splice(keep));
            if (st.elseStatement) { walk(blockOf(st.elseStatement)); els.push(...out.splice(keep)); }
            out.push({ t: "frame", id: this.id(), kind: "alt", label: oneLine(this.p.text(st.expression)), ref: this.p.ref(st), items: then, else: els.length ? els : undefined });
            continue;
          }
          decided.push(`${oneLine(this.p.text(st.expression))} is ${v}`);
          const branch = v ? st.thenStatement : st.elseStatement;
          if (branch && walk(blockOf(branch))) return true;
          continue;
        }
        if (ts.isReturnStatement(st)) {
          if (ctx.hook) return true;
          const e = st.expression && unwrap(st.expression);
          out.push({ t: "self", id: this.id(), at: ctx.self, kind: "render", label: e ? this.jsxLabel(e) : "nothing",
            detail: `render ${pass}${decided.length ? " · " + decided.join(", ") : ""}`, ref: this.p.ref(st) });
          if (e) out.push(...this.expr(e, { ...ctx, quiet: true }));
          return true;
        }
        if (pass === 1) out.push(...this.stmts([st], ctx));
      }
      return false;
    };
    walk(list);
    return out;
  }

  decide(cond: ts.Expression, env: Map<string, string>): boolean | undefined {
    cond = unwrap(cond);
    if (ts.isPrefixUnaryExpression(cond) && cond.operator === ts.SyntaxKind.ExclamationToken) {
      const v = this.decide(cond.operand, env);
      return v === undefined ? undefined : !v;
    }
    if (ts.isIdentifier(cond)) {
      const v = env.get(cond.text);
      if (v === "true") return true;
      if (v === "false" || v === "null" || v === "undefined" || v === '""' || v === "0") return false;
    }
    return undefined;
  }

  jsxLabel(e: ts.Expression): string {
    const el = ts.isJsxElement(e) ? e.openingElement : ts.isJsxSelfClosingElement(e) ? e : null;
    if (!el) return oneLine(this.p.text(e), 40);
    const tag = el.tagName.getText();
    const text = ts.isJsxElement(e) ? e.children.filter(ts.isJsxText).map((c) => c.text.trim()).filter(Boolean).join(" ") : "";
    return text ? `<${tag}>${oneLine(text, 32)}</${tag}>` : `<${tag}>`;
  }

  jsx(e: ts.JsxElement | ts.JsxSelfClosingElement | ts.JsxFragment, ctx: Ctx): Item[] {
    const out: Item[] = [];
    const open = ts.isJsxElement(e) ? e.openingElement : ts.isJsxSelfClosingElement(e) ? e : null;
    if (open) {
      const tag = open.tagName;
      const decl = /^[A-Z]/.test(tag.getText()) ? this.projectDecl(tag) : null;
      for (const a of open.attributes.properties) {
        if (!ts.isJsxAttribute(a) || !a.initializer || !ts.isJsxExpression(a.initializer) || !a.initializer.expression) continue;
        const name = a.name.getText();
        if (/^on[A-Z]/.test(name) || name === "action") continue; // a user action: another story
        if (!decl && tag.getText() === "img" && name === "src" && !urlText(unwrap(a.initializer.expression)))
          this.gap(`<img src={${oneLine(this.p.text(a.initializer.expression))}}>: the browser loads a picture from an address only known at runtime`, this.p.ref(a));
        if (!decl) out.push(...this.expr(a.initializer.expression, ctx));
      }
      if (decl) {
        const name = this.declName(decl);
        const to = this.part({ id: this.pid(decl), name, kind: "component", group: ctx.group, sub: this.p.rel(decl.getSourceFile()), ref: this.p.ref(decl) });
        // `key` is React bookkeeping, not something the component receives
        const props = open.attributes.properties.filter((a) => !(ts.isJsxAttribute(a) && a.name.getText() === "key")).map((a) => this.p.text(a));
        const label = `<${tag.getText()}${props.length ? " " + props.join(" ") : ""}${ts.isJsxSelfClosingElement(open) ? " />" : ">"}`;
        const call: Call = { t: "call", id: this.id(), from: ctx.self, to, label: oneLine(label, 40), detail: oneLine(this.p.text(e)), ref: this.p.ref(e), target: this.p.ref(decl),
          link: { kind: "exact", how: `JSX element resolved to ${this.p.rel(decl.getSourceFile())}:${this.p.ref(decl).from}` } };
        out.push(call);
        if (!ctx.stack.includes(decl)) {
          const inner = this.trial(() => this.component(decl, { self: to, group: ctx.group, returnTo: null, endpoint: false, env: new Map(), stack: [...ctx.stack, decl] }));
          if (inner) { this.walked.push({ name, decl }); call.body = inner; call.touches = this.touched(inner); }
        }
        if (!call.body && !ctx.inLoop) { this.parts.delete(to); out.pop(); return out; }
        return out;
      }
    }
    if (!ts.isJsxSelfClosingElement(e))
      for (const c of e.children) {
        if (ts.isJsxExpression(c)) out.push(...this.expr(c.expression, ctx));
        else if (!ts.isJsxText(c)) out.push(...this.expr(c, ctx));
      }
    return out;
  }

  // <Link href="…"> and router.push("…") that point at a page that doesn't exist.
  navigation(decl: ts.Node) {
    const check = (url: string | null, at: ts.Node, how: string) => {
      if (!url || !url.startsWith("/")) return;
      if (!matchRoute(this.routes, url, "page")) this.signal(`${how} goes to ${url.replace(/\{…\}/g, "{…}")}, but no page in app/ answers that URL`, this.p.ref(at));
    };
    const visit = (n: ts.Node) => {
      if ((ts.isJsxSelfClosingElement(n) || ts.isJsxOpeningElement(n)) && importOf(this.checker, n.tagName)?.module === "next/link") {
        for (const a of n.attributes.properties) {
          if (!ts.isJsxAttribute(a) || a.name.getText() !== "href" || !a.initializer) continue;
          const v = ts.isJsxExpression(a.initializer) ? a.initializer.expression : a.initializer;
          if (v) check(urlText(unwrap(v as ts.Expression)), a, `<Link href={${oneLine(this.p.text(v), 30)}}>`);
        }
      }
      if (ts.isCallExpression(n) && ts.isPropertyAccessExpression(n.expression) && ["push", "replace"].includes(n.expression.name.text) && n.arguments[0])
        if (/router/i.test(n.expression.expression.getText())) check(urlText(unwrap(n.arguments[0])), n, oneLine(this.p.text(n), 30));
      ts.forEachChild(n, visit);
    };
    visit(decl);
  }

  // ---------- small helpers ----------

  isReact(callee: ts.Expression, name: string): boolean {
    const id = unwrap(callee);
    if (ts.isIdentifier(id)) { const imp = importOf(this.checker, id); return imp?.module === "react" && imp.name === name; }
    return ts.isPropertyAccessExpression(id) && id.name.text === name && id.expression.getText() === "React";
  }

  // `const router = useRouter()` from next/navigation (or next/router)
  isRouter(id: ts.Identifier): boolean {
    const d = resolve(this.checker, id)?.valueDeclaration;
    const init = d && ts.isVariableDeclaration(d) && d.initializer ? unwrap(d.initializer) : undefined;
    if (!init || !ts.isCallExpression(init) || !ts.isIdentifier(init.expression)) return false;
    const imp = importOf(this.checker, init.expression);
    return !!imp && ["next/navigation", "next/router"].includes(imp.module) && imp.name === "useRouter";
  }

  navigate(e: ts.CallExpression, seg: Segment, ctx: Ctx): Item[] {
    const url = seg.args?.[0] ? urlText(unwrap(seg.args[0])) : null;
    const route = url?.startsWith("/") ? matchRoute(this.routes, url, "page") : undefined;
    const ref = this.p.ref(e);
    if (!route) return [{ t: "self", id: this.id(), at: ctx.self, kind: "local", label: oneLine(this.p.text(e), 40), ref }];
    const to = this.screen(route);
    return [{ t: "call", id: this.id(), from: ctx.self, to, label: oneLine(this.p.text(e), 40), detail: oneLine(this.p.text(e)), ref, target: this.p.ref(route.decl), opens: route.path,
      link: { kind: "exact", how: `Next.js routing: ${route.path} → ${route.file}` } }];
  }

  isFocusEffect(callee: ts.Expression): boolean {
    const id = unwrap(callee);
    const imp = ts.isIdentifier(id) ? importOf(this.checker, id) : null;
    return !!imp && imp.name === "useFocusEffect" && (imp.module === "expo-router" || imp.module.startsWith("@react-navigation/"));
  }

  stateOf(id: ts.Identifier): { state: string; setter: boolean } | null {
    const d = this.checker.getSymbolAtLocation(id)?.declarations?.[0];
    if (!d || !ts.isBindingElement(d) || !ts.isArrayBindingPattern(d.parent) || !ts.isVariableDeclaration(d.parent.parent)) return null;
    const init = d.parent.parent.initializer && unwrap(d.parent.parent.initializer);
    if (!init || !ts.isCallExpression(init) || !this.isReact(init.expression, "useState")) return null;
    return { state: d.parent.elements[0].getText(), setter: d.parent.elements.indexOf(d) === 1 };
  }

  setState(e: ts.CallExpression, state: string, ctx: Ctx): Item {
    const arg = e.arguments[0] && unwrap(e.arguments[0]);
    const literal = arg && (arg.kind === ts.SyntaxKind.TrueKeyword || arg.kind === ts.SyntaxKind.FalseKeyword || arg.kind === ts.SyntaxKind.NullKeyword || ts.isNumericLiteral(arg) || ts.isStringLiteral(arg));
    ctx.env.set(state, literal ? this.p.text(arg) : "?");
    return { t: "self", id: this.id(), at: ctx.self, kind: "state", label: oneLine(this.p.text(e), 48), detail: `changes the state "${state}", which renders the component again`, ref: this.p.ref(e) };
  }

  // Declared inside a custom hook (`const fetchX = useCallback(…)` or a function in its body).
  insideHook(decl: ts.Node): boolean {
    for (let n = decl.parent; n; n = n.parent) if (ts.isFunctionDeclaration(n) && n.name && /^use[A-Z]/.test(n.name.text)) return true;
    return false;
  }

  nestedIn(decl: ts.Node, stack: ts.Node[]): boolean {
    for (let n = decl.parent; n; n = n.parent) if (stack.includes(n)) return true;
    return false;
  }

  declName(decl: ts.Node): string {
    const n = (decl as ts.FunctionDeclaration).name;
    if (n) return n.getText();
    for (let p: ts.Node | undefined = decl.parent; p; p = p.parent) {
      if (ts.isVariableDeclaration(p) || ts.isPropertyAssignment(p)) return p.name.getText();
      if (!ts.isCallExpression(p) && !ts.isParenthesizedExpression(p) && !ts.isAsExpression(p)) break;
    }
    return "anonymous";
  }

  pid(decl: ts.Node) {
    return `${this.p.rel(decl.getSourceFile())}#${this.declName(decl)}`;
  }

  // Which named functions call this declaration anywhere in the project (endpoints shown as "GET /path").
  callersOf(decl: ts.Node): Set<string> {
    if (!this.callers) {
      const callers = (this.callers = new Map<ts.Node, Set<string>>());
      for (const sf of this.p.sources) {
        const visit = (n: ts.Node) => {
          if (ts.isCallExpression(n)) {
            const target = this.projectDecl(ts.isPropertyAccessExpression(n.expression) ? n.expression.name : n.expression);
            let f: ts.Node | undefined = n.parent;
            while (f && !(ts.isFunctionDeclaration(f) && f.name)) f = f.parent;
            if (target && f) {
              const r = this.routes.find((x) => x.decl === f && x.kind === "endpoint");
              const label = r ? `${r.method} ${r.path}` : this.declName(f);
              if (!callers.has(target)) callers.set(target, new Set());
              callers.get(target)!.add(label);
            }
          }
          ts.forEachChild(n, visit);
        };
        visit(sf);
      }
    }
    return this.callers.get(decl) ?? new Set();
  }

  part(p: Participant): string {
    if (!this.parts.has(p.id)) this.parts.set(p.id, p);
    return p.id;
  }

  id() {
    return `i${++this.n}`;
  }

  signal(text: string, ref: Ref) {
    if (!this.signals.some((s) => s.text === text)) this.signals.push({ text, ref });
  }

  gap(text: string, ref: Ref) {
    if (!this.gaps.some((s) => s.text === text)) this.gaps.push({ text, ref });
  }

  // ---------- output ----------

  finish(id: string, title: string, trigger: Story["trigger"], items: Item[]): Story {
    const refs: Ref[] = [];
    const files = new Set<string>();
    const collect = (list: Item[]) => {
      for (const i of list) {
        refs.push(i.t === "frame" ? { ...i.ref, to: i.ref.from } : i.ref);
        files.add(i.ref.file);
        if (i.t === "call" && i.target) files.add(i.target.file);
        if (i.t === "call" && i.body) collect(i.body);
        if (i.t === "frame") { collect(i.items); collect(i.else ?? []); }
      }
    };
    collect(items);
    for (const p of this.parts.values()) if (p.ref) files.add(p.ref.file);

    // Every line inside a walked function should sit behind some step (signature and lone brackets aside).
    const coverage: Coverage[] = this.walked.map(({ name, decl }) => {
      const r = this.p.ref(decl);
      const lines = decl.getSourceFile().text.split("\n");
      const missing: number[] = [];
      let total = 0;
      for (let l = r.from + 1; l <= r.to; l++) {
        const t = lines[l - 1].trim();
        if (!t || /^[\])}>;,]*$/.test(t) || t.startsWith("//")) continue;
        total++;
        if (!refs.some((x) => x.file === r.file && x.from <= l && l <= x.to)) missing.push(l);
      }
      return { fn: name, file: r.file, total, covered: total - missing.length, missing };
    });

    const order: Record<Group, number> = { device: 0, browser: 0, server: 1, supabase: 2, outside: 3 };
    const participants = [...this.parts.values()].sort((a, b) => order[a.group] - order[b.group]);
    this.files = files;
    this.meaningful = crosses(items, this.parts);
    return { id, title, trigger, participants, items, signals: this.signals, gaps: this.gaps, coverage };
  }
}

// Every story in the project worth showing: each screen opening and each user action that crosses a boundary.
export function discover(p: Project): ProjectModel {
  const t0 = performance.now();
  const stories: Story[] = [];
  const files = new Set<string>();
  const keep = (b: StoryBuilder, s: Story) => {
    if (!b.meaningful) return;
    stories.push(s);
    b.files.forEach((f) => files.add(f));
  };
  const errors: string[] = [];
  const only = process.env.ONLY;
  for (const r of routes(p).filter((x) => x.kind === "page" && (!only || x.path === only))) {
    try {
      const b = new StoryBuilder(p);
      keep(b, b.screenOpens(r));
      for (const t of new StoryBuilder(p).triggers(r.decl)) {
        const a = new StoryBuilder(p);
        keep(a, a.userAction(r, t));
      }
    } catch (err) {
      errors.push(`${r.file}: ${err instanceof Error ? err.message : String(err)}`);
    }
  }
  if (errors.length) console.error(`failed on ${errors.length} screens:\n  ` + errors.slice(0, 8).join("\n  "));
  // Packages that aren't installed: recognized by their import name only.
  const unresolved = new Set<string>();
  for (const sf of p.sources)
    for (const st of sf.statements)
      if (ts.isImportDeclaration(st)) {
        const spec = (st.moduleSpecifier as ts.StringLiteral).text;
        if (!spec.startsWith(".") && !spec.startsWith("@/") && !ts.resolveModuleName(spec, sf.fileName, p.program.getCompilerOptions(), ts.sys).resolvedModule) unresolved.add(spec);
      }
  if (unresolved.size)
    for (const s of stories) s.gaps.push({ text: `not installed in the sample, so recognized by import name only: ${[...unresolved].sort().join(", ")}`, ref: { file: "package.json", from: 1, to: 1 } });
  const src: Record<string, string> = {};
  for (const f of [...files].sort()) src[f] = p.program.getSourceFile(`${p.root}/${f}`)?.text ?? "";
  return { name: p.root.split("/").pop()!, stories, files: src, stats: { ms: Math.round(performance.now() - t0), llmCalls: 0 } };
}
