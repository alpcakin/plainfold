// What the view asks of the shell. Inside Tauri these are commands in src-tauri/src/main.rs.
// In a plain browser (`npm run dev`, then open the page) a preview stands in for the shell, so the view can be
// developed and screenshotted without the app: `?preview=/@fs/<absolute path to a model.json>`.
import type { AppState, Change, EnglishBatch, EnglishEnd, KeyState, Need, Opened, Progress, ProjectModel, Said, ServerSettings, StoryDiff } from "./model";

export const inApp = "__TAURI_INTERNALS__" in window;
const params = new URLSearchParams(location.search);

async function tauri() {
  const [{ invoke }, { listen }] = await Promise.all([import("@tauri-apps/api/core"), import("@tauri-apps/api/event")]);
  return { invoke, listen };
}

export async function startup(): Promise<AppState> {
  if (inApp) return (await tauri()).invoke<AppState>("startup");
  const model = params.get("preview");
  return { recent: model ? [{ path: model, name: previewName(model), stories: 0, adapters: [] }] : [], showWelcome: true, autoDownload: false, keepServers: params.get("settings") !== "off", english: params.get("english") !== "off" };
}

/** This window's language servers, as the settings show them. */
export async function serverSettings(): Promise<ServerSettings> {
  if (inApp) return (await tauri()).invoke<ServerSettings>("server_settings");
  // preview: `&settings=1` running, `&settings=closed` closed after a while, `&settings=off` turned off
  const now = Math.floor(Date.now() / 1000), s = params.get("settings");
  if (s === "off") return { keep: false, project: null, running: [], closed: ["turned off in Settings", now - 120] };
  if (s === "closed") return { keep: true, project: null, running: [], closed: ["not used for 20 minutes", now - 300] };
  return { keep: true, project: params.get("preview"), running: [{ kind: "typescript", name: "typescript-language-server", bytes: 906_000_000, since: now - 1500, reads: 3 }], closed: null };
}

export async function setKeepServers(on: boolean): Promise<void> {
  if (inApp) await (await tauri()).invoke("set_keep_servers", { on });
}

/** The window leaves its project (back to the welcome screen): its language servers stop. */
export async function closeProject(): Promise<void> {
  if (inApp) await (await tauri()).invoke("close_project");
}

/** The languages a project uses and the state of each one's language server. */
export async function checkServers(path: string): Promise<Need[]> {
  if (inApp) return (await tauri()).invoke<Need[]>("check_servers", { path });
  // preview: `&ask=1` shows the question asked when a server has to be downloaded, `&ask=node` with Node.js too
  if (!params.get("ask")) return [];
  const server = { label: "TypeScript language server", detail: "typescript-language-server 6.0.1, typescript 5.9.3", from: "the npm registry", downloadMb: 5, diskMb: 25 };
  const node = { label: "Node.js 24.21.0", detail: "node-v24.21.0-darwin-arm64.tar.gz", why: "It runs the language server: this machine has Node.js 20.11.0; the TypeScript language server needs 22.22 or newer.", from: "nodejs.org", downloadMb: 53, diskMb: 122 };
  const parts = params.get("ask") === "node" ? [server, node] : [server];
  return [{ kind: "typescript", state: "download", detail: parts.map((p) => p.detail).join(", "), label: server.label, parts }];
}

export async function installServers(id: number, kinds: string[], onProgress: (p: Progress) => void): Promise<void> {
  if (!inApp) return;
  const { invoke, listen } = await tauri();
  const stop = await listen<Progress & { id: number }>("progress", (e) => {
    if (e.payload.id === id) onProgress(e.payload);
  });
  try {
    await invoke("install_servers", { id, kinds });
  } finally {
    stop();
  }
}

/** Stops waiting for a language server in read `id` (`kind` "*": the whole read). */
export async function stopWaiting(id: number, kind: string): Promise<void> {
  if (inApp) await (await tauri()).invoke("stop_waiting", { id, kind });
}

export async function setAutoDownload(on: boolean): Promise<void> {
  if (inApp) await (await tauri()).invoke("set_auto_download", { on });
}

const previewName = (p: string) => p.split("/").pop()!.replace(/\.model\.json$|\.json$/, "");

export async function openProject(id: number, path: string, onProgress: (p: Progress) => void): Promise<Opened> {
  if (inApp) {
    const { invoke, listen } = await tauri();
    const stop = await listen<Progress & { id: number }>("progress", (e) => {
      if (e.payload.id === id) onProgress(e.payload);
    });
    try {
      return await invoke<Opened>("open_project", { id, path });
    } finally {
      stop();
    }
  }
  const model: ProjectModel = await (await fetch(path)).json();
  onProgress({ step: "loaded", name: model.name, files: Object.keys(model.files).length });
  const missing = (model.missing ?? []).map((m) => m.package);
  // preview: `&diff=/@fs/<absolute path to the JSON of plainfold-core diff --json>` shows what changed
  const d = params.get("diff");
  const diff: StoryDiff[] = d ? await (await fetch(d)).json() : [];
  return { path, model, adapters: [], servers: [{ kind: "preview", name: "a saved model", with: "", readiness: "" }], failed: [], missing, diff };
}

/** The user has seen the changes: the next read compares with the model as it is now. */
export async function markSeen(path: string): Promise<void> {
  if (inApp) await (await tauri()).invoke("mark_seen", { path });
}

/** Calls `f` whenever the files read `id` built its stories from change: every file that differs now (none: back as read). */
export async function onChanges(f: (id: number, changes: Change[]) => void): Promise<void> {
  if (!inApp) return;
  await (await tauri()).listen<{ id: number; changes: Change[] }>("changes", (e) => f(e.payload.id, e.payload.changes));
}

// ---------- English ----------
// The steps in plain English (core/src/english). In the browser preview, `&english=/@fs/<absolute path to the output
// of plainfold-core english>` stands for the model; `&english=off` starts with English off, `&key=none` as if no key
// were saved.

let englishBatch: ((b: EnglishBatch) => void) | null = null;
let englishEnd: ((e: EnglishEnd) => void) | null = null;

/** Calls `batch` as texts arrive, and `end` when a run is done. */
export async function onEnglish(batch: (b: EnglishBatch) => void, end: (e: EnglishEnd) => void): Promise<void> {
  englishBatch = batch;
  englishEnd = end;
  if (!inApp) return;
  const { listen } = await tauri();
  await listen<EnglishBatch>("english", (e) => batch(e.payload));
  await listen<EnglishEnd>("english-end", (e) => end(e.payload));
}

export async function englishKey(): Promise<KeyState> {
  if (inApp) return (await tauri()).invoke<KeyState>("english_key");
  return { from: params.get("key") === "none" || !params.get("english") ? null : "keychain", model: "gpt-4o-mini" };
}

/** Saves the key in the system's keychain, or removes it (null). */
export async function setEnglishKey(key: string | null): Promise<KeyState> {
  if (inApp) return (await tauri()).invoke<KeyState>("set_english_key", { key });
  return { from: key ? "keychain" : null, model: "gpt-4o-mini" };
}

export async function setEnglish(on: boolean): Promise<void> {
  if (inApp) await (await tauri()).invoke("set_english", { on });
}

/** Starts the English of read `id`'s steps; returns the model that writes it. */
export async function describe(id: number, path: string): Promise<string> {
  if (inApp) return (await tauri()).invoke<string>("describe", { id, path });
  const p = params.get("english");
  if (!p || p === "off") throw new Error("no API key");
  const t: { model: string; texts: Record<string, Said> } = await (await fetch(p)).json();
  const n = Object.keys(t.texts).length;
  setTimeout(() => {
    englishBatch?.({ id, texts: t.texts, done: n, total: n, tokensIn: 0, tokensOut: 0 });
    englishEnd?.({ id, steps: n, described: n, rejected: {}, requests: 0, kept: 0, tokensIn: 0, tokensOut: 0, error: null, stopped: false });
  });
  return t.model;
}

export async function setShowWelcome(show: boolean): Promise<void> {
  if (inApp) await (await tauri()).invoke("set_show_welcome", { show });
}

export async function openInEditor(root: string, file: string, line: number): Promise<string> {
  if (!inApp) return "nothing (browser preview)";
  return (await tauri()).invoke<string>("open_in_editor", { root, file, line });
}

export async function pickFolder(): Promise<string | null> {
  if (!inApp) return params.get("preview");
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true, multiple: false, title: "Open a project folder" });
  return typeof r === "string" ? r : null;
}

/** Calls `f` with the first folder dropped on the window. */
export async function onDrop(f: (path: string) => void): Promise<void> {
  if (!inApp) return;
  const { getCurrentWebview } = await import("@tauri-apps/api/webview");
  await getCurrentWebview().onDragDropEvent((e) => {
    if (e.payload.type === "drop" && e.payload.paths.length) f(e.payload.paths[0]);
  });
}

/** The welcome window is small and fixed; a project gets a large, resizable one. */
export async function windowFor(kind: "welcome" | "project"): Promise<void> {
  if (!inApp) return;
  const { getCurrentWindow, LogicalSize, currentMonitor } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (kind === "welcome") {
    await w.setMinSize(null);
    await w.setResizable(true);
    await w.setSize(new LogicalSize(800, 480));
    await w.setResizable(false);
    await w.setMaximizable(false);
  } else {
    const m = await currentMonitor();
    const sw = m ? m.size.width / m.scaleFactor : 1440, sh = m ? m.size.height / m.scaleFactor : 900;
    await w.setResizable(true);
    await w.setMaximizable(true);
    await w.setMinSize(new LogicalSize(880, 560));
    await w.setSize(new LogicalSize(Math.min(1440, sw - 80), Math.min(900, sh - 80)));
  }
  await w.center();
}

export async function windowControl(what: "min" | "max" | "close"): Promise<void> {
  if (!inApp) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (what === "min") await w.minimize();
  else if (what === "max") await w.toggleMaximize();
  else await w.close();
}
