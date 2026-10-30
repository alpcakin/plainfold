// A minimal LSP client over stdio: just enough to open files and ask "go to definition".
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { LANGS, langFor } from "./langs.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const bin = (name) => path.join(here, "..", "node_modules", ".bin", name);

class Client {
  constructor(cmd, args, cwd, settings) {
    this.proc = spawn(cmd, args, { cwd, stdio: ["pipe", "pipe", "pipe"] });
    this.buf = Buffer.alloc(0);
    this.id = 0;
    this.pending = new Map();
    this.settings = settings;
    this.opened = new Set();
    this.last = Date.now();
    this.proc.stdout.on("data", (d) => this.data(d));
    this.proc.stderr.on("data", () => {});
  }
  data(d) {
    this.last = Date.now();
    this.buf = Buffer.concat([this.buf, d]);
    for (;;) {
      const h = this.buf.indexOf("\r\n\r\n");
      if (h < 0) return;
      const len = +/Content-Length: (\d+)/i.exec(this.buf.subarray(0, h).toString())[1];
      if (this.buf.length < h + 4 + len) return;
      const msg = JSON.parse(this.buf.subarray(h + 4, h + 4 + len).toString("utf8"));
      this.buf = this.buf.subarray(h + 4 + len);
      this.message(msg);
    }
  }
  message(msg) {
    if (msg.id !== undefined && !msg.method) {
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      clearTimeout(p.timer);
      return msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result);
    }
    if (msg.method && msg.id !== undefined) {
      // requests from the server: answer the ones that matter, null for the rest
      let result = null;
      if (msg.method === "workspace/configuration") result = (msg.params.items || []).map((i) => this.settings[i.section] ?? {});
      if (process.env.LSP_DEBUG) process.stderr.write(`[server→client] ${msg.method} ${JSON.stringify(msg.params).slice(0, 300)} → ${JSON.stringify(result).slice(0, 200)}\n`);
      this.send({ jsonrpc: "2.0", id: msg.id, result });
    }
  }
  send(o) {
    const s = JSON.stringify(o);
    this.proc.stdin.write(`Content-Length: ${Buffer.byteLength(s)}\r\n\r\n${s}`);
  }
  request(method, params, ms = 120000) {
    const id = ++this.id;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`timeout: ${method}`)); }, ms);
      this.pending.set(id, { resolve, reject, timer });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }
  notify(method, params) { this.send({ jsonrpc: "2.0", method, params }); }
  // LSP has no standard "ready" signal, and answers given before the server has loaded the project or its environment
  // look valid but are weaker. Wait until the server has been quiet for a while (it logs and asks for settings while
  // it sets up), with an upper bound.
  async settle(quietMs = 1500, maxMs = 30000) {
    const start = Date.now();
    while (Date.now() - this.last < quietMs && Date.now() - start < maxMs) await new Promise((r) => setTimeout(r, 100));
  }
  open(file) {
    if (this.opened.has(file)) return;
    this.opened.add(file);
    const lang = langFor(file);
    this.notify("textDocument/didOpen", { textDocument: { uri: pathToFileURL(file).href, languageId: lang ? LANGS[lang].lspId : "plaintext", version: 1, text: fs.readFileSync(file, "utf8") } });
  }
  // Locations a name at (line, col) resolves to: [{ file, line, col }] (0-based), empty when unresolved.
  async definition(file, line, col) {
    this.open(file);
    const r = await this.request("textDocument/definition", { textDocument: { uri: pathToFileURL(file).href }, position: { line, character: col } });
    const list = r == null ? [] : Array.isArray(r) ? r : [r];
    return list.map((l) => {
      const uri = l.targetUri ?? l.uri, range = l.targetSelectionRange ?? l.range;
      return { file: fileURLToPath(uri), line: range.start.line, col: range.start.character };
    });
  }
  async stop() {
    try { await this.request("shutdown", null, 5000); } catch {}
    this.notify("exit", null);
    this.proc.kill();
  }
}

// Starts the language server for a kind of source ("typescript", "python") on a project root.
export async function startServer(kind, root, opts = {}) {
  const settings = {};
  let client, init = {};
  if (kind === "typescript") {
    client = new Client(bin("typescript-language-server"), ["--stdio"], root, settings);
    // The syntax-only helper server answers before the project has loaded, and its "definition" stops at the import.
    // That answer looks valid but is weaker, so only the full (semantic) server may answer.
    init = { tsserver: { useSyntaxServer: "never", ...(opts.tsserver ? { path: opts.tsserver } : {}) } };
  } else if (kind === "python") {
    if (opts.pythonPath) settings.python = { pythonPath: opts.pythonPath };
    settings["python.analysis"] = { diagnosticMode: "openFilesOnly" };
    client = new Client(bin("pyright-langserver"), ["--stdio"], root, settings);
  } else throw new Error(`no language server for ${kind}`);
  const uri = pathToFileURL(root).href;
  await client.request("initialize", {
    processId: process.pid,
    rootUri: uri,
    workspaceFolders: [{ uri, name: path.basename(root) }],
    capabilities: { textDocument: { definition: { linkSupport: false }, synchronization: {} }, workspace: { configuration: true, workspaceFolders: true } },
    initializationOptions: init,
  });
  client.notify("initialized", {});
  await client.settle();
  return client;
}

// Runs async jobs with a bounded number in flight.
export async function pool(items, n, job) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: n }, async () => {
    while (next < items.length) { const i = next++; out[i] = await job(items[i], i); }
  }));
  return out;
}
