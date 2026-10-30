// The plainfold window: welcome, reading a project, and the story view (sidebar, diagram, details, status bar).
import * as backend from "./backend";
import { Canvas } from "./canvas";
import { SIGN, withChanges } from "./changes";
import { StoryView, type Level, type Reach } from "./diagram";
import { siteKey } from "./english";
import { panel, type PanelState } from "./inspector";
import type { AppState, Change, EnglishBatch, EnglishEnd, Finding, Item, KeyState, Need, NotInstalled, Opened, Progress, ReturnItem, Said, ServerSettings, Story, StoryDiff, Thing, Use, UseKind } from "./model";
import { alsoWords, effectsOf, flowName, nameStories, refStory, stepCount } from "./model";
import { outcome, placesOf, type Part, type Place } from "./places";
import { entries, filter as palFilter, list as palList, type Entry } from "./palette";
import { $, APP_ICON, ICON, esc } from "./ui";

const params = new URLSearchParams(location.search);
const platform = params.get("platform") ?? (/Windows/.test(navigator.userAgent) ? "windows" : /Mac/.test(navigator.userAgent) ? "mac" : "linux");
document.documentElement.classList.add(platform);
const MOD = platform === "mac" ? "⌘" : "Ctrl+";

// ---------- state ----------
interface Step {
  key: string;
  title: string;
  detail: string;
  status: "done" | "busy" | "todo" | "warn";
  /** a language whose server the user may stop waiting for */
  skip?: string;
}

let appState: AppState = { recent: [], showWelcome: true, autoDownload: false, keepServers: true, english: true };
/** the settings, while they are open */
let settings: ServerSettings | null = null;
let settingsTimer = 0;
let screen: "welcome" | "opening" | "story" = "welcome";
let opened: Opened | null = null;
let view: StoryView | null = null;
let storyIdx = 0;
const back: number[] = [], fwd: number[] = [];
let tab: "stories" | "tests" | "data" | "setup" = "stories";
/** the Tests tab's own page: what the tests reach, story by story (instead of a story) */
let testsPage = false;
/** the level of detail the user chose for the columns; "auto" lets each story pick */
let level: "auto" | Level = "auto";
/** the thing the story list is narrowed to: only the stories that reach it (an id in the model's index) */
let focus: string | null = null;
let hideSide = false, hideInsp = false;
let every = false;
/** the diagram's viewport, kept between stories. The next drawing goes to the default view: for a new story at its
 *  first change, when the columns moved at the step selected. */
const canvas = new Canvas();
let homeNext: "" | "story" | "columns" = "";
let findings: (Finding & { kind: string })[] = [];
const panelState: PanelState = { expanded: new Set() };
let pal: { q: string; at: number; all: Entry[]; shown: Entry[] } | null = null;
let reading: { id: number; path: string; name: string; steps: Step[]; error?: string; ask?: { needs: Need[]; answer: (yes: boolean) => void } } | null = null;
let readId = 0;
/** the read `opened` came from, and the files that changed since (told by the shell's watch) */
let openedId = 0;
let changed: Change[] = [];
/** changes told for a read whose result has not arrived yet */
let early: { id: number; changes: Change[] } | null = null;
/** The steps in plain English for the open read (english.ts): the texts by step, the model writing them, how far it
 *  is, and where its key comes from. */
const eng = {
  texts: new Map<string, Said>(),
  id: 0,
  model: "",
  done: 0,
  total: 0,
  running: false,
  tokens: 0,
  error: "",
  key: null as KeyState | null,
  /** steps left as code when the run ended, by why their text did not pass */
  rejected: {} as Record<string, number>,
};

// ---------- skeleton ----------
$("root").innerHTML = `
<div class="welcome" id="welcome" hidden>
  <div class="wl-main" data-tauri-drag-region>
    <div class="wl-caption"><button type="button" class="cap" data-act="win-min" aria-label="Minimize">${ICON.winMin()}</button><button type="button" class="cap close" data-act="win-close" aria-label="Close">${ICON.winClose()}</button></div>
    ${APP_ICON(84)}
    <h1 data-tauri-drag-region>plainfold</h1>
    <p class="wl-sub" data-tauri-drag-region>See what your code does, as stories.</p>
    <button type="button" class="wl-btn" data-act="open-folder"><span class="acc">${ICON.folder(18)}</span><span class="grow">Open Folder…</span><kbd>${MOD}O</kbd></button>
    <p class="wl-hint" data-tauri-drag-region>Or drop a project folder on this window.</p>
    <div class="wl-check"><label><input type="checkbox" id="show-welcome"> Show this window when plainfold opens</label><button type="button" class="link" data-act="settings">Settings…</button></div>
  </div>
  <aside class="wl-recent" aria-label="Recent projects">
    <div class="wl-h" data-tauri-drag-region>Recent</div>
    <div class="wl-list" id="recent"></div>
  </aside>
</div>
<div class="app" id="app" hidden>
  <div class="wintitle" data-tauri-drag-region>
    <div class="wt-l" data-tauri-drag-region><span class="wt-mark">${ICON.mark()}</span><span data-tauri-drag-region>plainfold</span></div>
    <button type="button" class="wt-search" data-act="palette">${ICON.search(14)}<span class="grow">Go to a story, participant or file</span><kbd>Ctrl+K</kbd></button>
    <div class="wt-r"><button type="button" class="cap" data-act="win-min" aria-label="Minimize">${ICON.winMin()}</button><button type="button" class="cap" data-act="win-max" aria-label="Maximize">${ICON.winMax()}</button><button type="button" class="cap close" data-act="win-close" aria-label="Close">${ICON.winClose()}</button></div>
  </div>
  <div class="frame">
    <aside class="side" aria-label="Project">
      <div class="lights" data-tauri-drag-region><button type="button" class="ib" data-act="side" aria-label="Hide sidebar">${ICON.sidebar()}</button></div>
      <div class="proj-w"><button type="button" class="proj" data-act="switch" title="Open another project">
        <span class="proj-ic">${ICON.folder()}</span><span class="grow"><span class="proj-n" id="proj-name"></span><span class="proj-s" id="proj-sub"></span></span>${ICON.updown()}
      </button></div>
      <div class="seg" role="tablist" aria-label="Sections">
        <button type="button" role="tab" data-act="tab" data-tab="stories" id="tab-stories">Stories</button>
        <button type="button" role="tab" data-act="tab" data-tab="tests" id="tab-tests">Tests</button>
        <button type="button" role="tab" data-act="tab" data-tab="data" id="tab-data">Data</button>
        <button type="button" role="tab" data-act="tab" data-tab="setup" id="tab-setup">Setup</button>
      </div>
      <label class="filter" id="filter">${ICON.search(13)}<input id="q" type="search" placeholder="Filter stories" aria-label="Filter stories" autocomplete="off" spellcheck="false"></label>
      <nav class="stories" id="stories" aria-label="Stories"></nav>
    </aside>
    <div class="main">
      <div class="toolbar" id="toolbar" data-tauri-drag-region></div>
      <div class="bar" id="bar"><div class="bar-in" id="bar-in"></div></div>
      <div class="body">
        <section class="doc" id="doc" aria-label="Story"></section>
        <aside class="insp" id="insp" aria-label="Step details"></aside>
      </div>
      <div class="status" id="status"></div>
    </div>
  </div>
  <div class="palette" id="palette" hidden>
    <div class="scrim" data-act="pal-close"></div>
    <div class="pal" role="dialog" aria-label="Go to">
      <label class="pal-q">${ICON.search()}<input id="pal-q" type="text" placeholder="Go to a story, participant or file" aria-label="Go to a story, participant or file" autocomplete="off" spellcheck="false"><kbd>esc</kbd></label>
      <div class="pal-list" id="pal-list"></div>
      <div class="pal-foot"><span>↑↓ move</span><span>↵ open</span><span>${MOD}↵ open in editor</span><span class="grow"></span><span>esc close</span></div>
    </div>
  </div>
</div>
<div class="settings" id="settings" hidden>
  <div class="scrim" data-act="set-close"></div>
  <div class="dlg" role="dialog" aria-modal="true" aria-label="Settings">
    <div class="dlg-h" data-tauri-drag-region><span class="grow">Settings</span><button type="button" class="ib" data-act="set-close" aria-label="Close">${ICON.winClose()}</button></div>
    <div class="dlg-b" id="settings-body"></div>
  </div>
</div>
<div class="toast" id="toast" hidden></div>`;

// ---------- screens ----------
function show(s: typeof screen) {
  screen = s;
  $("welcome").hidden = s !== "welcome";
  $("app").hidden = s === "welcome";
  document.documentElement.classList.toggle("is-welcome", s === "welcome");
}

function renderWelcome() {
  ($("show-welcome") as HTMLInputElement).checked = appState.showWelcome;
  $("recent").innerHTML =
    appState.recent
      .map(
        (r) =>
          `<button type="button" class="wl-item" data-act="open-path" data-path="${esc(r.path)}"><span class="wl-ic">${ICON.folder(16)}</span><span class="grow">` +
          `<span class="wl-n">${esc(r.name)}</span><span class="wl-p">${esc(tilde(r.path))}</span>` +
          `<span class="wl-d">${esc([...r.adapters, plural(r.stories, "story", "stories")].join(" · "))}</span></span></button>`,
      )
      .join("") || `<p class="wl-empty">Projects you open appear here.</p>`;
}

/** The places a story can start in, as the list names them, in the order the diagram's columns take. */
const PLACE: Record<string, string> = { device: "Device", browser: "Browser", server: "Server", supabase: "Supabase", outside: "Outside" };
const placeOrder = (p: string) => (p === "device" || p === "browser" ? 0 : p === "server" ? 1 : p === "supabase" ? 2 : p ? 3 : 4);

/** A story's icon in the lists: what starts it. */
const storyIcon = (st: Story): string => {
  const k = st.trigger.kind;
  return k === "user-action" ? ICON.action() : k === "screen-open" ? ICON.screen() : k === "endpoint" ? ICON.arrow() : k === "schedule" ? ICON.clock() : k === "lifecycle" ? ICON.power() : k === "test" ? ICON.check() : ICON.box();
};

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** What starts a story, in words: the trigger kind from the model, plus what was used for a user action, or how
 * something outside the project reaches a function, as far as the code shows. */
function triggerWords(st: Story): string {
  const t = st.trigger, sub = flowName(st).sub;
  if (t.kind === "screen-open") return "screen opens";
  if (t.kind === "schedule" || t.kind === "lifecycle") return t.when ?? t.kind;
  if (t.kind === "user-action") return "user action" + (sub ? " · " + sub : "");
  if (t.kind === "endpoint") {
    // one the project's own stories call says so; a request from outside starts the others, as far as the code shows
    const by = reachedFrom(st);
    return (by ? plural(by.n, "story reaches it", "stories reach it") : "request") + (t.handler ? ` · handled by ${t.handler}` : "");
  }
  if (t.kind === "file" || (t.kind === "test" && !t.how)) return (t.kind === "test" ? "test · " : "") + "top-level code of the file";
  if (t.how) {
    const how =
      t.how === "decorated" ? t.by ?? "decorated"
      : t.how === "passed" ? `passed to ${t.by ?? "a call"}`
      : t.how === "value" ? `used as a value${t.by ? ` (${t.by})` : ""}`
      : "nothing in the project calls it";
    const ext = t.extends?.length ? ` · its class extends ${t.extends.join(", ")}` : "";
    return (t.kind === "test" ? "test · " : "") + how + ext;
  }
  return t.kind.replace(/-/g, " ");
}

const tilde = (p: string) => p.replace(/^\/Users\/[^/]+|^\/home\/[^/]+/, "~");

// ---------- the story list: the places a person meets ----------
/** Places the user opened or closed in the list, by their story's id; the others follow `placeOpen`. */
const folded = new Map<string, boolean>();

/** A screen's name: the function it is, as its story draws it, else its address. */
function screenName(st: Story): string {
  const fn = st.participants.find((p) => p.kind === "screen");
  return fn?.name ?? st.trigger.route;
}

/** The stories tab: the app's start and layouts, the screens in the order a person reaches them with the controls on
 *  each, then the server and what no adapter places (`places.ts`). */
function placeList(q: string): string {
  const m = opened!.model, P = placesOf(m), ambient = new Set(P.ambient);
  const byId = new Map(m.stories.map((s) => [s.id, s]));
  const goesTo = (id: string) => {
    const s = byId.get(id);
    return s ? (s.trigger.kind === "screen-open" ? screenName(s) : flowName(s).main) : undefined;
  };
  const matches = (i: number) => {
    const st = m.stories[i];
    return !q || (st.title + " " + flowName(st).main + " " + st.trigger.route + " " + (st.trigger.kind === "screen-open" ? screenName(st) : "")).toLowerCase().includes(q);
  };
  const changedIn = (ids: number[]) => ids.some((i) => diffOf(m.stories[i].id));
  const fx = (st: Story) =>
    outcome(st, ambient, goesTo)
      .map((o) => (o.tone ? `<span class="${o.tone}">${esc(o.text)}</span>` : esc(o.text)))
      .join(" · ");
  const row = (i: number, sub?: string) => {
    const st = m.stories[i], on = i === storyIdx && !testsPage, d = diffOf(st.id);
    const name = st.trigger.kind === "screen-open" ? screenName(st) : flowName(st).main;
    const lines = [sub ?? "", ...(st.trigger.kind === "user-action" ? alsoWords(st).map(esc) : []), fx(st)].filter(Boolean).join(" · ");
    return `<button type="button" class="story${on ? " on" : ""}" data-act="story" data-id="${i}"${on ? ' aria-current="true"' : ""}>` +
      `<span class="story-ic">${storyIcon(st)}</span>` +
      `<span class="grow"><span class="story-n">${esc(name)}</span>${lines ? `<span class="story-s">${lines}</span>` : ""}</span>` +
      (d ? `<span class="dsign ${d.mark}" title="${esc(diffWords(d))} since you last looked">${SIGN[d.mark]}</span>` : "") + `</button>`;
  };
  const all = (p: { actions: number[]; parts: Part[] }): number[] => [...p.actions, ...p.parts.flatMap(all)];
  // a part that holds controls, under its name, and the parts it draws; one with no controls of its own only passes
  // what it draws on, so the list stays as shallow as the parts that hold controls. With a filter, only what matches.
  const part = (p: Part, on = 1): string => {
    const acts = p.actions.filter(matches);
    const inner = p.parts.map((x) => part(x, acts.length ? p.on : on)).join("");
    if (!acts.length) return inner;
    const shared = p.on > 1 && p.on !== on ? `<span class="n">on ${p.on} screens</span>` : "";
    return `<div class="ppart"><div class="ppart-h" title="${esc(p.name)}"><span class="grow">${esc(p.name)}</span>${shared}</div>${acts.map((i) => row(i)).join("")}${inner}</div>`;
  };
  const place = (pl: Place, sub: string) => {
    const st = m.stories[pl.story], key = st.id, inside = all(pl);
    const hit = matches(pl.story), shown = q ? inside.filter(matches) : inside;
    if (q && !hit && !shown.length) return "";
    const auto = inside.includes(storyIdx) || changedIn(inside) || (!!q && shown.length > 0);
    const open = inside.length > 0 && (folded.get(key) ?? auto);
    const body = open
      ? `<div class="pbody">${pl.actions.filter((i) => !q || hit || matches(i)).map((i) => row(i)).join("")}${pl.parts.map((p) => part(p)).join("")}</div>`
      : "";
    const chev = inside.length
      ? `<button type="button" class="pchev" data-act="fold" data-key="${esc(key)}" aria-expanded="${open}" aria-label="${open ? "Close" : "Open"} ${esc(screenName(st))}" title="${inside.length} control${inside.length === 1 ? "" : "s"}">${ICON.forward()}</button>`
      : `<span class="pchev"></span>`;
    return `<div class="place${open ? " open" : ""}"><div class="prow">${chev}${row(pl.story, sub)}</div>${body}</div>`;
  };
  const sect = (title: string, n: number, why = "") => `<h3 class="sect"${why ? ` title="${esc(why)}"` : ""}><span class="grow">${esc(title)}</span><span class="n">${n}</span></h3>`;
  let h = "";
  const app = P.app.map((pl) => place(pl, m.stories[pl.story].trigger.when ?? "")).join("");
  if (app) h += sect("App", P.app.length, "What runs as the app starts, and the layouts drawn around its screens") + app;
  const route = (st: Story) => (st.trigger.route !== screenName(st) ? st.trigger.route : "");
  const screens = P.screens.map((pl) => place(pl, [route(m.stories[pl.story]), pl.reached ? "" : "nothing found opens it"].filter(Boolean).join(" · "))).join("");
  if (screens) {
    const amb = P.ambient.map((e) => m.index?.find((t) => t.id === e)?.name ?? e);
    h += sect("Screens", P.screens.length, "In the order a person reaches them from the start of the app") +
      (amb.length ? `<p class="pnote">Almost everything here also calls ${amb.map(esc).join(", ")}: left out of the lines below.</p>` : "") + screens;
  }
  const loose = P.loose.map((p) => part(p)).join("");
  if (loose) h += sect("Not found on a screen", P.loose.reduce((n, p) => n + p.actions.length, 0), "Controls in parts that no screen is found to draw: what draws them is not followed yet") + loose;
  const eps = P.endpoints.map(([file, list]) => {
    const rows = list.filter(matches).map((i) => row(i, triggerWords(m.stories[i]))).join("");
    return rows ? `<h4>${esc(file)}</h4>${rows}` : "";
  }).join("");
  const own = P.own.filter(matches).map((i) => row(i, triggerWords(m.stories[i]))).join("");
  if (eps || own) h += sect("Server", P.endpoints.reduce((n, [, l]) => n + l.length, 0) + P.own.length) + eps + (own ? `<h4>Runs by itself</h4>${own}` : "");
  const other = P.other.filter(matches).map((i) => row(i, triggerWords(m.stories[i]))).join("");
  if (other) h += sect("Other entry points", P.other.length, "Functions nothing in the project calls, and top-level code. The code does not show where they run.") + other;
  return h;
}

// ---------- reading a project ----------
async function openPath(path: string) {
  if (reading) backend.stopWaiting(reading.id, "*"); // a read now waits as long as its servers need: end the old one
  const id = ++readId;
  const name = path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
  reading = { id, path, name, steps: firstSteps() };
  if (opened?.path !== path) focus = null;
  if (screen === "welcome") await backend.windowFor("project");
  show("opening");
  renderOpening();
  try {
    await prepareServers(id, path);
    if (reading?.id !== id) return;
    const o = await backend.openProject(id, path, (p) => {
      if (reading?.id !== id) return;
      progress(p);
      renderOpening();
    });
    if (reading?.id !== id) return;
    // reading the same project again keeps the story that was open, if it is still there
    const keep = opened?.path === o.path ? storyAt(storyIdx)?.id : undefined;
    reading = null;
    nameStories(o.model.stories);
    opened = o;
    openedId = id;
    changed = early?.id === id ? early.changes : [];
    early = null;
    back.length = fwd.length = 0;
    panelState.expanded.clear();
    tab = "stories";
    ($("q") as HTMLInputElement).value = "";
    show("story");
    if (o.model.stories.length || o.diff.length) loadStory(Math.max(0, keep ? indexOf(keep) : 0), false);
    else renderStory();
    backend.startup().then((s) => (appState = { ...s, open: null }));
    void startEnglish(id, o.path);
  } catch (e) {
    // a failure after the read was shown (while drawing) is the view's own error: do not let it vanish
    if (!reading && readId === id) console.error(e);
    if (reading?.id !== id) return;
    reading.error = String(e);
    renderOpening();
  }
}

const elapsed = (s: number) => (s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`);
const mb = (bytes: number) => (bytes / 1e6).toFixed(1);
const notShown = (kind: string) => `Stories that start in ${kind} code are not shown: plainfold shows nothing rather than guessing.`;
/** What a download for a language brings, in words: its server, or only what runs the server. */
const getting = (n: Need) => (n.parts.some((p) => p.label === n.label) ? `the ${n.label}` : `${n.parts.map((p) => p.label).join(", ")} for the ${n.label}`);

/** Finds the language servers the project needs; asks before downloading any (unless told not to ask). */
async function prepareServers(id: number, path: string) {
  const needs = await backend.checkServers(path);
  if (reading?.id !== id) return;
  const steps = reading.steps;
  const before = () => steps.findIndex((s) => s.key === "build");
  for (const n of needs.filter((n) => n.state === "unavailable")) {
    steps.splice(before(), 0, { key: "no:" + n.kind, title: `No ${n.label}`, detail: `${n.detail}. ${notShown(n.kind)}`, status: "warn" });
  }
  const get = needs.filter((n) => n.state === "download");
  if (!get.length) return renderOpening();
  const first = steps[0];
  const yes = appState.autoDownload || (await new Promise<boolean>((answer) => {
    first.status = "todo"; // nothing is read while the question is open
    reading!.ask = { needs: get, answer };
    renderOpening();
  }));
  if (reading?.id !== id) return;
  reading.ask = undefined;
  first.status = "busy";
  for (const n of get) {
    steps.splice(before(), 0, yes
      ? { key: "dl:" + n.kind, title: `Downloading ${getting(n)}`, detail: "", status: "busy" }
      : { key: "no:" + n.kind, title: `${n.label}: not downloaded`, detail: notShown(n.kind), status: "warn" });
  }
  renderOpening();
  if (!yes) return;
  for (const n of get) {
    const step = steps.find((s) => s.key === "dl:" + n.kind)!;
    try {
      await backend.installServers(id, [n.kind], (p) => {
        if (reading?.id !== id || p.step !== "downloading") return;
        step.detail = (n.parts.length > 1 ? `${p.label}: ` : "") + (p.total ? `${mb(p.done)} of ${mb(p.total)} MB` : `${mb(p.done)} MB`);
        renderOpening();
      });
      Object.assign(step, { title: `Downloaded ${getting(n)}`, detail: n.detail, status: "done" });
    } catch (e) {
      Object.assign(step, { title: `Could not download ${getting(n)}`, detail: `${e}. ${notShown(n.kind)}`, status: "warn" });
    }
    renderOpening();
  }
}

const firstSteps = (): Step[] => [
  { key: "load", title: "Reading the files", detail: "", status: "busy" },
  { key: "adapters", title: "Matching adapters", detail: "", status: "todo" },
  { key: "servers", title: "Starting language servers", detail: "", status: "todo" },
  { key: "build", title: "Building stories", detail: "", status: "todo" },
];

function progress(p: Progress) {
  const steps = reading!.steps;
  const at = (k: string) => steps.find((s) => s.key === k);
  const set = (k: string, s: Partial<Step>) => Object.assign(at(k) ?? steps[steps.push({ key: k, title: "", detail: "", status: "todo" }) - 1], s);
  if (p.step === "loaded") {
    reading!.name = p.name;
    set("load", { title: "Project found", detail: p.files === 1 ? "1 source file" : `${p.files} source files`, status: "done" });
    set("adapters", { status: "busy" });
  } else if (p.step === "adapters") {
    set("adapters", p.names.length ? { title: "Adapters matched", detail: p.names.join(" · "), status: "done" } : { title: "No adapter matched", detail: "Stories start where an adapter finds an entry point, so this project may show none yet.", status: "warn" });
  } else if (p.step === "server-starting") {
    const i = steps.findIndex((s) => s.key === "servers");
    const step: Step = { key: "srv:" + p.kind, title: `Waiting for the ${p.kind} language server`, detail: "Its answers are used only once they stop changing, however long that takes.", status: "busy" };
    if (i >= 0) steps.splice(i, 1, step);
    else steps.splice(steps.findIndex((s) => s.key === "build"), 0, step);
  } else if (p.step === "server-kept") {
    const i = steps.findIndex((s) => s.key === "servers");
    const step: Step = { key: "srv:" + p.kind, title: `Using the ${p.name} kept from the last read`, detail: p.changed ? `${plural(p.changed, "file", "files")} changed since` : "Nothing changed since", status: "busy" };
    if (i >= 0) steps.splice(i, 1, step);
    else steps.splice(steps.findIndex((s) => s.key === "build"), 0, step);
  } else if (p.step === "server-waiting") {
    set("srv:" + p.kind, { detail: `${p.doing || "Loading the project"} · ${elapsed(p.seconds)}`, skip: p.kind });
  } else if (p.step === "server-ready") {
    set("srv:" + p.kind, { title: `${p.name} ready`, detail: "", status: "done", skip: undefined });
  } else if (p.step === "server-failed") {
    const i = steps.findIndex((s) => s.key === "servers");
    if (i >= 0) steps.splice(i, 1);
    const w = steps.findIndex((s) => s.key === "srv:" + p.kind);
    if (w >= 0) steps.splice(w, 1);
    if (!at("no:" + p.kind)) {
      const step: Step = { key: "no:" + p.kind, title: `No ${p.kind} language server`, detail: `${p.why}. ${notShown(p.kind)}`, status: "warn" };
      steps.splice(steps.findIndex((s) => s.key === "build"), 0, step);
    }
  } else if (p.step === "building") {
    steps.forEach((s) => s.key === "servers" && (s.status = "done"));
    set("build", { status: "busy" });
  }
}

function renderOpening() {
  const r = reading!;
  const done = r.steps.filter((s) => s.status === "done" || s.status === "warn").length;
  $("proj-name").textContent = r.name;
  $("proj-sub").textContent = r.error ? "Could not be read" : "Reading…";
  $("toolbar").innerHTML = toolbarHead(r.name, r.error ? "Could not be read" : "Reading the project");
  ($("bar") as HTMLElement).hidden = !!r.error;
  ($("bar-in") as HTMLElement).style.width = `${Math.round((done / r.steps.length) * 100)}%`;
  $("stories").innerHTML = `<div class="skel"><i style="width:44px"></i><i style="width:72%;height:10px;margin-top:14px"></i><i style="width:48%"></i><i style="width:64px;margin-top:24px"></i><i style="width:58%;height:10px;margin-top:14px"></i><i style="width:40%"></i></div>`;
  $("filter").hidden = true;
  segment();
  const icon = (s: Step) => (s.status === "done" ? ICON.done() : s.status === "busy" ? ICON.busy() : s.status === "warn" ? ICON.warn(16) : ICON.todo());
  $("doc").classList.remove("split");
  $("doc").innerHTML =
    `<div class="opening"><div class="mono dim">${esc(tilde(r.path))}</div><h1>${r.error ? "Could not read" : "Reading"} ${esc(r.name)}</h1>` +
    (r.error
      ? `<p class="err">${esc(r.error)}</p><div class="acts"><button type="button" class="btn" data-act="open-folder">Choose another folder…</button><button type="button" class="btn" data-act="retry">Try again</button></div>`
      : `<p class="sub">plainfold reads the code. It never runs it.</p><div class="meter"><div style="width:${Math.round((done / r.steps.length) * 100)}%"></div></div>` +
        (r.ask ? askCard(r.ask.needs) : "") +
        `<ol class="steps">${r.steps.map((s) => `<li class="${s.status}"><span class="si">${icon(s)}</span><span><span class="st-t">${esc(s.title)}</span>${s.detail ? `<span class="st-d">${esc(s.detail)}</span>` : ""}` +
          (s.skip && s.status === "busy" ? `<button type="button" class="link st-skip" data-act="skip" data-kind="${esc(s.skip)}">Stop waiting (nothing in ${esc(s.skip)} code will be shown)</button>` : "") +
          `</span></li>`).join("")}</ol>`) +
    `</div>`;
  $("insp").hidden = true;
  $("status").innerHTML = r.steps.some((s) => s.key.startsWith("srv:") && s.status === "busy")
    ? `<span class="st"><span class="dot busy"></span>${esc(r.steps.find((s) => s.key.startsWith("srv:") && s.status === "busy")!.title.replace(/^Waiting for the /, ""))} · waiting for its answers to settle</span>`
    : "";
  layout();
}

function askCard(needs: Need[]) {
  // a part two languages need (Node.js) is downloaded once
  const parts = [...new Map(needs.flatMap((n) => n.parts).map((p) => [p.label, p])).values()];
  const total = parts.reduce((n, p) => n + p.downloadMb, 0);
  const from = [...new Set(parts.map((p) => p.from))].join(" and ");
  return (
    `<div class="ask"><div class="ask-t">To read this project, plainfold needs to download</div>` +
    `<ul>${parts.map((p) => `<li><span class="st-t">${esc(p.label)} · ${p.downloadMb} MB, ${p.diskMb} MB on disk</span><span class="st-d mono">${esc(p.detail)}</span>${p.why ? `<span class="st-d">${esc(p.why)}</span>` : ""}</li>`).join("")}</ul>` +
    `<p class="ask-d">Downloaded once from ${esc(from)} at exactly these versions, and checked before use. Without ${parts.length === 1 ? "it" : "them"}, stories in ${needs.length === 1 ? "that language" : "those languages"} are not shown: plainfold shows nothing rather than guessing.</p>` +
    `<label class="ask-c"><input type="checkbox" id="dl-always"> Download what a project needs without asking</label>` +
    `<div class="acts"><button type="button" class="btn primary" data-act="dl-yes">Download${total ? ` (${total} MB)` : ""}</button><button type="button" class="btn" data-act="dl-no">Not now</button></div></div>`
  );
}

// ---------- the story view ----------
// Stories are numbered as the model lists them; stories removed since the user last looked follow, as they were.
const removedStories = (): Story[] => (opened?.diff ?? []).filter((d) => d.mark === "removed" && d.story).map((d) => d.story!);
function storyAt(i: number): Story | undefined {
  const m = opened?.model;
  if (!m) return undefined;
  return i < m.stories.length ? m.stories[i] : removedStories()[i - m.stories.length];
}
function indexOf(id: string): number {
  const m = opened!.model, i = m.stories.findIndex((s) => s.id === id);
  if (i >= 0) return i;
  const r = removedStories().findIndex((s) => s.id === id);
  return r >= 0 ? m.stories.length + r : -1;
}
const diffOf = (id: string): StoryDiff | undefined => opened?.diff.find((d) => d.id === id);

/** "renamed from save · 2 added · 1 changed": what happened to where a changed story starts, then its steps by how
 *  they changed. */
function diffWords(d: StoryDiff): string {
  if (d.mark === "added") return "new story";
  if (d.mark === "removed") return "removed story";
  const n = (m: string) => d.steps.filter((s) => s.mark === m).length;
  const start = [d.renamed ? `renamed from ${d.renamed}` : "", d.moved ? `moved from ${d.moved}` : ""];
  return [...start, ...(["added", "removed", "changed"] as const).filter((m) => n(m)).map((m) => `${n(m)} ${m}`)].filter(Boolean).join(" · ");
}

function loadStory(i: number, push = true) {
  if (push && i !== storyIdx) {
    back.push(storyIdx);
    fwd.length = 0;
  }
  storyIdx = i;
  const st = storyAt(i)!;
  const marked = withChanges(st, diffOf(st.id));
  view = new StoryView(marked.story, marked.marks, marked.was, marked.named, reachOf(opened!.model));
  view.storyName = (id) => {
    const s = opened!.model.stories.find((x) => x.id === id);
    return s && flowName(s).main;
  };
  setEnglish(view);
  view.every = every;
  view.setLevel(level === "auto" ? view.autoLevel() : level);
  homeNext = "story";
  testsPage = false;
  findings = [...st.signals.map((f) => ({ ...f, kind: "signal" })), ...st.gaps.map((f) => ({ ...f, kind: "gap" }))];
  renderStory();
  $("doc").scrollTop = 0;
}

function renderStory() {
  const o = opened!;
  $("proj-name").textContent = o.model.name;
  $("proj-sub").textContent = o.adapters.join(" · ") || plural(o.model.stories.length, "story", "stories");
  ($("bar") as HTMLElement).hidden = true;
  $("filter").hidden = tab !== "stories" && tab !== "tests";
  ($("q") as HTMLInputElement).placeholder = tab === "tests" ? "Filter tests" : "Filter stories";
  $("insp").hidden = hideInsp || !view || testsPage;
  layout(); // before drawing: the diagram fits the width the panels leave
  segment();
  sidebar();
  toolbar();
  doc();
  draw();
  statusBar();
}

function layout() {
  const a = $("app");
  a.classList.toggle("no-side", hideSide);
  a.classList.toggle("no-insp", hideInsp || screen !== "story" || testsPage);
}

function segment() {
  $("tab-stories").setAttribute("aria-selected", String(tab === "stories"));
  $("tab-tests").setAttribute("aria-selected", String(tab === "tests"));
  $("tab-data").setAttribute("aria-selected", String(tab === "data"));
  $("tab-setup").setAttribute("aria-selected", String(tab === "setup"));
}

/** The packages that are not installed: where the project lists each, and the command its own tools would install
 *  them with, to copy. plainfold never runs it: installing writes into the project and runs the packages' own code. */
function notInstalled(list: NotInstalled[]): string {
  if (!list.length) return "";
  const where = (n: NotInstalled) =>
    (n.listed.length ? `listed in ${n.listed.join(", ")}` : "not listed under this name in a manifest plainfold reads") + (n.env ? ` · not in ${n.env}` : "");
  const cmds = [...new Set(list.flatMap((n) => n.install ?? []))];
  return `<h4>Not installed</h4>` +
    `<p>Calls into these are shown by name only, as not proven.</p>` +
    `<ul>${list.map((n) => `<li><span class="pkg">${esc(n.package)}</span><span class="k">${esc(where(n))}</span></li>`).join("")}</ul>` +
    (cmds.length
      ? `<p>To install what the manifests list, run this from the project's folder, then Reload. plainfold does not run it: installing writes into the project and runs the packages' own code.</p>` +
        cmds.map((c) => `<div class="cmd"><code>${esc(c)}</code><button type="button" class="btn sm" data-act="copy" data-text="${esc(c)}">${ICON.copy()}Copy</button></div>`).join("")
      : `<p>Install them with the project's own tools, then Reload. plainfold does not install them: that writes into the project and runs the packages' own code.</p>`);
}

function sidebar() {
  const m = opened!.model;
  if (tab === "setup") {
    const o = opened!;
    $("stories").innerHTML =
      `<div class="setup"><p class="dim">Where the app runs, how it ships, its settings and libraries: not built yet.</p>` +
      `<h4>How this project was read</h4><ul>` +
      (o.adapters.length ? `<li><span class="k">Adapters</span>${esc(o.adapters.join(", "))}</li>` : `<li><span class="k">Adapters</span>none matched</li>`) +
      o.servers.map((s) => `<li><span class="k">${esc(s.kind)}</span>${esc(s.name)} · ready${s.with ? ` · ${esc(s.with)}` : ""}</li>`).join("") +
      o.failed.map((f) => `<li class="warn"><span class="k">${esc(f.kind)}</span>No language server: ${esc(f.why)}. Its stories are not shown.</li>`).join("") +
      `</ul>` +
      (m.setup.length ? `<h4>What limits it</h4><ul>${m.setup.map((n) => `<li class="warn">${esc(n)}</li>`).join("")}</ul>` : "") +
      notInstalled(m.missing ?? []) +
      `</div>`;
    return;
  }
  if (tab === "data") {
    $("stories").innerHTML = dataList();
    return;
  }
  const q = ($("q") as HTMLInputElement).value.trim().toLowerCase();
  const t = focus ? thing(focus) : undefined;
  if (t) {
    const box = $("stories");
    box.innerHTML = focusList(t, q);
    // the selected story in view, else the top, where it says what the list is narrowed to
    const on = box.querySelector<HTMLElement>(".on");
    box.scrollTop = on && on.offsetTop > box.clientHeight - 40 ? Math.max(0, on.offsetTop - box.clientHeight / 2) : 0;
    return;
  }
  const diff = opened!.diff;
  const summary = diff.length
    ? `<div class="chg"><div class="chg-h"><span class="grow">Since you last looked</span>` +
      `<button type="button" class="btn sm" data-act="seen" title="Clears every mark. The next Reload compares with the code as it is now.">OK</button></div>` +
      diff
        .map((d) => {
          const i = indexOf(d.id), on = i === storyIdx;
          return `<button type="button" class="story${on ? " on" : ""}" data-act="story" data-id="${i}"><span class="dsign ${d.mark}">${SIGN[d.mark]}</span>` +
            `<span class="grow"><span class="story-n${d.mark === "removed" ? " gone" : ""}">${esc(flowName(storyAt(i)!).main)}</span><span class="story-s">${esc(diffWords(d))}</span></span></button>`;
        })
        .join("") +
      `</div>`
    : "";
  // the stories tab: the places a person meets
  if (tab === "stories") {
    const none = m.stories.length ? "No story matches." : opened!.failed.length ? "No stories: a language server is missing." : "No stories: nothing in this project reaches outside its own code yet.";
    $("stories").innerHTML = summary + (placeList(q) || `<p class="none">${none}</p>`);
    const on = $("stories").querySelector<HTMLElement>(".on");
    const box = $("stories");
    if (on && (on.offsetTop < box.scrollTop || on.offsetTop > box.scrollTop + box.clientHeight - 40)) box.scrollTop = Math.max(0, on.offsetTop - box.clientHeight / 2);
    return;
  }
  // tests, grouped by the file they are written in
  const sections = new Map<string, Map<string, [Story, number][]>>();
  m.stories.forEach((st, i) => {
    if ((st.trigger.kind === "test") !== (tab === "tests")) return;
    if (q && !(st.title + " " + flowName(st).main + " " + st.trigger.route).toLowerCase().includes(q)) return;
    const place = st.trigger.place ?? "";
    if (!sections.has(place)) sections.set(place, new Map());
    const groups = sections.get(place)!;
    if (!groups.has(st.trigger.route)) groups.set(st.trigger.route, []);
    groups.get(st.trigger.route)!.push([st, i]);
  });
  const row = ([st, i]: [Story, number]) => {
    const fx = [...effectsOf(st)], nm = flowName(st), on = i === storyIdx && !testsPage;
    const sub = [nm.sub || (st.trigger.kind === "user-action" || st.trigger.kind === "test" ? "" : triggerWords(st)), ...fx.map((f) => (f === "reads" ? f : `<span class="warm">${esc(f)}</span>`))].filter(Boolean);
    return `<button type="button" class="story${on ? " on" : ""}" data-act="story" data-id="${i}"${on ? ' aria-current="true"' : ""}>` +
      `<span class="story-ic">${storyIcon(st)}</span>` +
      `<span class="grow"><span class="story-n">${esc(nm.main)}</span>${sub.length ? `<span class="story-s">${sub.map((s) => (s.startsWith("<") ? s : esc(s))).join(" · ")}</span>` : ""}</span>` +
      (diffOf(st.id) ? `<span class="dsign ${diffOf(st.id)!.mark}" title="${esc(diffWords(diffOf(st.id)!))} since you last looked">${SIGN[diffOf(st.id)!.mark]}</span>` : "") + `</button>`;
  };
  const block = (g: Map<string, [Story, number][]>) => [...g].map(([route, list]) => `<h4>${esc(route)}</h4>` + list.map(row).join("")).join("");
  // a heading for each place only when there is more than one
  const placed = [...sections].sort(([a], [b]) => placeOrder(a) - placeOrder(b));
  const listed = placed
    .map(([place, g]) => {
      const n = [...g.values()].reduce((k, l) => k + l.length, 0);
      const h = place
        ? `<h3 class="sect"><span class="grow">${esc(PLACE[place] ?? place)}</span><span class="n">${n}</span></h3>`
        : `<h3 class="sect" title="Functions nothing in the project calls, and top-level code. The code does not show where they run."><span class="grow">Other entry points</span><span class="n">${n}</span></h3>`;
      return (sections.size > 1 ? h : "") + block(g);
    })
    .join("");
  // the Tests tab starts with its own page: what the tests reach, when there are tests
  const overview = !q && m.stories.some((st) => st.trigger.kind === "test")
    ? `<button type="button" class="story${testsPage ? " on" : ""}" data-act="tests-page"><span class="story-ic">${ICON.check()}</span><span class="grow"><span class="story-n">What the tests reach</span><span class="story-s">story by story</span></span></button>`
    : "";
  const none = q ? "No test matches." : "No tests in this project.";
  $("stories").innerHTML = overview + listed || `<p class="none">${none}</p>`;
  const on = $("stories").querySelector<HTMLElement>(".on");
  const box = $("stories");
  if (on && (on.offsetTop < box.scrollTop || on.offsetTop > box.scrollTop + box.clientHeight - 40)) box.scrollTop = Math.max(0, on.offsetTop - box.clientHeight / 2);
}

// ---------- what uses what ----------
let reachCache: { model: Opened["model"]; reach: Reach } | null = null;
/** How many stories reach each thing, and which tests, from the model's index (tests counted apart). */
function reachOf(m: Opened["model"]): Reach {
  if (reachCache?.model === m) return reachCache.reach;
  const tests = new Set(m.stories.filter((s) => s.trigger.kind === "test").map((s) => s.id));
  const stories = new Map<string, number>(), byTest = new Map<string, string[]>();
  for (const t of m.index ?? []) {
    const ids = new Set(t.uses.map((u) => u.story));
    stories.set(t.id, [...ids].filter((id) => !tests.has(id)).length);
    byTest.set(t.id, [...ids].filter((id) => tests.has(id)));
  }
  const reach: Reach = { stories: (id) => stories.get(id) ?? 0, tests: (id) => byTest.get(id) ?? [], total: m.stories.length - tests.size };
  reachCache = { model: m, reach };
  return reach;
}

let startCache: { model: Opened["model"]; at: Map<string, Thing> } | null = null;
/** How many other stories (tests apart) reach what a story starts in, such as its endpoint or screen, with that
 * thing's id in the index. */
function reachedFrom(st: Story): { id: string; name: string; n: number } | undefined {
  const m = opened!.model;
  if (startCache?.model !== m) {
    const at = new Map<string, Thing>();
    for (const t of m.index ?? []) for (const u of t.uses) if (u.how === "starts") at.set(u.story, t);
    startCache = { model: m, at };
  }
  const t = startCache.at.get(st.id);
  const n = t ? reachOf(m).stories(t.id) - (st.trigger.kind === "test" ? 0 : 1) : 0;
  return t && n > 0 ? { id: t.id, name: t.name, n } : undefined;
}

/** How many of the functions a story calls on its main path tests reach. */
function testsOf(v: StoryView): { hit: number; of: number } {
  const calls = v.mainCalls();
  return { hit: calls.filter((c) => v.reach.tests(c.to).length).length, of: calls.length };
}
let indexCache: { model: Opened["model"]; by: Map<string, Thing> } | null = null;
function thing(id: string): Thing | undefined {
  const m = opened?.model;
  if (!m) return undefined;
  if (indexCache?.model !== m) indexCache = { model: m, by: new Map((m.index ?? []).map((t) => [t.id, t])) };
  return indexCache.by.get(id);
}

/** Narrows the story list to the stories that reach a thing (nothing happens for one the index does not know). */
function focusOn(id: string) {
  if (!thing(id)) return;
  focus = id;
  tab = "stories";
  segment();
  sidebar();
}

const STRENGTH: Record<UseKind, number> = { deletes: 0, writes: 1, reads: 2, calls: 3, draws: 4, starts: 5 };
const BY_HOW: Record<UseKind, string> = { deletes: "Delete it", writes: "Write it", reads: "Read it", calls: "Call it", draws: "Draw it", starts: "Start in it" };

function findItem(items: Item[], id: string): Item | undefined {
  for (const it of items) {
    if (it.id === id) return it;
    const inner = it.t === "call" ? it.body : it.t === "frame" ? [...it.items, ...(it.else ?? [])] : undefined;
    const f = inner && findItem(inner, id);
    if (f) return f;
  }
  return undefined;
}

/** What a thing is, in words. */
function thingKind(t: Thing): string {
  if (t.in) return t.uses.every((u) => u.how === "calls") ? "database function" : "table";
  if (t.sub === "package") return "package";
  return t.kind;
}

/** The stories that reach a thing, each with where: grouped by how for a table, else by where they start. */
function focusList(t: Thing, q: string): string {
  const m = opened!.model;
  const by = new Map<string, Use[]>();
  for (const u of t.uses) by.set(u.story, [...(by.get(u.story) ?? []), u]);
  const groups = new Map<string, [Story, number, Use[]][]>(), tests: [Story, number, Use[]][] = [];
  const order: string[] = [];
  m.stories.forEach((st, i) => {
    const uses = by.get(st.id);
    if (!uses) return;
    if (q && !(st.title + " " + flowName(st).main + " " + st.trigger.route).toLowerCase().includes(q)) return;
    if (st.trigger.kind === "test") return void tests.push([st, i, uses]);
    const strongest = uses.reduce<UseKind>((a, u) => (STRENGTH[u.how] < STRENGTH[a] ? u.how : a), "starts");
    const key = t.in ? BY_HOW[strongest] : st.trigger.route;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push([st, i, uses]);
    if (!order.includes(key)) order.push(key);
  });
  if (t.in) order.sort((a, b) => Object.values(BY_HOW).indexOf(a) - Object.values(BY_HOW).indexOf(b));
  // the use a row stands for: the strongest (a delete before a read), one a step draws before one it only holds
  const main = (uses: Use[]) => [...uses].sort((a, b) => STRENGTH[a.how] - STRENGTH[b.how] || Number(b.drawn) - Number(a.drawn) || Number(!!b.step) - Number(!!a.step))[0];
  const cut = (s: string) => (s.length > 34 ? s.slice(0, 33) + "…" : s);
  const where = (st: Story, uses: Use[]) => {
    const u = main(uses);
    const it = u.step ? findItem(st.items, u.step) : undefined;
    const at = u.how === "starts" ? "starts here" : it ? `${u.drawn ? "at" : "inside"} ${cut(it.label)}` : `${u.ref.file.split("/").pop()}:${u.ref.from}`;
    return at + (uses.length > 1 ? ` · ${uses.length} places` : "");
  };
  const row = ([st, i, uses]: [Story, number, Use[]]) => {
    const nm = flowName(st), on = i === storyIdx, step = main(uses).step;
    return `<button type="button" class="story${on ? " on" : ""}" data-act="story" data-id="${i}"${step ? ` data-step="${esc(step)}"` : ""}${on ? ' aria-current="true"' : ""}>` +
      `<span class="story-ic">${storyIcon(st)}</span>` +
      `<span class="grow"><span class="story-n">${esc(nm.main)}</span><span class="story-s">${esc(where(st, uses))}</span></span></button>`;
  };
  const nStories = [...groups.values()].reduce((n, l) => n + l.length, 0);
  const head =
    `<div class="focus"><div class="focus-h"><span class="grow"><span class="focus-k">${esc(thingKind(t))}</span><span class="focus-n">${esc(t.name)}</span></span>` +
    `<button type="button" class="ib" data-act="unfocus" aria-label="Show every story" title="Show every story (esc)">${ICON.winClose()}</button></div>` +
    `<div class="focus-s" title="Calls that could not be resolved are not counted, so more stories may reach it.">${plural(nStories, "story", "stories")}${tests.length ? ` and ${plural(tests.length, "test", "tests")}` : ""} reach it, at least</div></div>`;
  const body = order.map((k) => `<h4>${esc(k)}</h4>` + groups.get(k)!.map(row).join("")).join("") +
    (tests.length ? `<div class="grp-t static"><span class="grow">Tests</span><span class="n">${tests.length}</span></div>` + tests.map(row).join("") : "");
  return head + (body || `<p class="none">${q ? "No story matches." : "No story reaches it."}</p>`);
}

/** The tables the stories touch, with how many read, write and delete each, and what outside the code they reach. */
function dataList(): string {
  const m = opened!.model, idx = m.index ?? [];
  const testIds = new Set(m.stories.filter((s) => s.trigger.kind === "test").map((s) => s.id));
  const count = (t: Thing, how?: UseKind) => new Set(t.uses.filter((u) => !testIds.has(u.story) && (!how || u.how === how)).map((u) => u.story)).size;
  const byCount = (a: Thing, b: Thing) => count(b) - count(a) || a.name.localeCompare(b.name);
  const tables = idx.filter((t) => t.in && t.uses.some((u) => u.how !== "calls")).sort(byCount);
  const fns = idx.filter((t) => t.in && t.uses.every((u) => u.how === "calls")).sort(byCount);
  const holders = new Set(idx.filter((t) => t.in).map((t) => t.in));
  const outside = idx.filter((t) => !t.in && !holders.has(t.id) && (t.group === "outside" || t.group === "supabase")).sort(byCount);
  const row = (t: Thing, sub: string, icon: string) =>
    `<button type="button" class="story" data-act="focus" data-thing="${esc(t.id)}"><span class="story-ic">${icon}</span>` +
    `<span class="grow"><span class="story-n">${esc(t.name)}</span><span class="story-s">${sub}</span></span></button>`;
  const fx = (t: Thing) =>
    (["reads", "writes", "deletes"] as const)
      .map((h) => [h, count(t, h)] as const)
      .filter(([, n]) => n)
      .map(([h, n]) => (h === "reads" ? `${n} ${h.replace(/s$/, "")}` : `<span class="warm">${n} ${h.replace(/s$/, "")}</span>`))
      .join(" · ") || "tests only";
  const used = (t: Thing) => { const n = count(t); return n ? plural(n, "story", "stories") : "tests only"; };
  const part = (title: string, list: Thing[], sub: (t: Thing) => string, icon: string) => (list.length ? `<h4>${esc(title)}</h4>` + list.map((t) => row(t, sub(t), icon)).join("") : "");
  const html =
    part("Tables", tables, fx, ICON.database()) +
    part("Database functions", fns, used, ICON.database()) +
    part("Outside the code", outside, (t) => `${esc(thingKind(t))} · ${used(t)}`, ICON.box());
  return html || `<p class="none">${m.index ? "The stories reach no table and nothing outside the code." : "This model was read before plainfold kept an index of what uses what. Reload to read it again."}</p>`;
}

function toolbarHead(title: string, sub: string) {
  return (
    `<button type="button" class="ib show-side" data-act="side" aria-label="Show sidebar">${ICON.sidebar()}</button>` +
    `<button type="button" class="ib" data-act="back" aria-label="Back" ${back.length && screen === "story" ? "" : "disabled"}>${ICON.back()}</button>` +
    `<button type="button" class="ib" data-act="fwd" aria-label="Forward" ${fwd.length && screen === "story" ? "" : "disabled"}>${ICON.forward()}</button>` +
    `<div class="tb-title" data-tauri-drag-region><span class="tb-t" data-tauri-drag-region>${esc(title)}</span><span class="tb-s" data-tauri-drag-region>${esc(sub)}</span></div>` +
    `<span class="grow" data-tauri-drag-region></span>`
  );
}

/** Shown once a file the stories were built from changes; the project is read again only when the user asks. */
function changesButton() {
  if (!changed.length) return "";
  const files = changed.map((c) => `${c.file} (${c.how})`).join("\n");
  return `<button type="button" class="tb-btn changes" data-act="reload" title="${esc(files)}">` +
    `<span class="dot"></span>${plural(changed.length, "file", "files")} changed<span class="tb-dot">·</span><span class="tb-act">Reload</span><kbd>${MOD}R</kbd></button>`;
}

function toolbar() {
  if (!view) {
    $("toolbar").innerHTML = toolbarHead(opened!.model.name, "") + changesButton();
    return;
  }
  const n = flowName(view.S);
  $("toolbar").innerHTML =
    (testsPage ? toolbarHead("What the tests reach", opened!.model.name) : toolbarHead(n.main, [n.sub, view.S.trigger.route].filter(Boolean).join(" · "))) +
    changesButton() +
    (testsPage ? "" :
      `<div class="lvl" role="group" aria-label="Columns">` +
      (["parts", "functions"] as const).map((l) => `<button type="button" data-act="level" data-level="${l}" aria-pressed="${view!.level() === l}" title="${l === "parts" ? "One column for each place the code runs" : "One column for each function"}">${l === "parts" ? "Parts" : "Functions"}</button>`).join("") +
      `</div>` +
      `<button type="button" class="tb-btn" data-act="english" aria-pressed="${appState.english}" title="${esc(englishTitle())}"><span class="sw${appState.english ? " on" : ""}"><i></i></span>English</button>` +
      `<button type="button" class="tb-btn" data-act="every" aria-pressed="${every}"><span class="sw${every ? " on" : ""}"><i></i></span>Every step</button>` +
      `<span class="vsep"></span>`) +
    `<button type="button" class="ib mac-only" data-act="palette" aria-label="Go to" title="Go to (${MOD}K)">${ICON.search()}</button>` +
    `<button type="button" class="ib${hideInsp ? "" : " on"}" data-act="insp" aria-label="${hideInsp ? "Show" : "Hide"} details" aria-pressed="${!hideInsp}">${ICON.inspector()}</button>`;
}

function doc() {
  $("doc").classList.toggle("split", !!view && !testsPage);
  if (!view) {
    const failed = opened!.failed;
    $("doc").innerHTML = failed.length
      ? `<div class="empty"><h1>No stories to show</h1><p>plainfold shows nothing in a language whose language server is missing, rather than guessing.</p>` +
        `<ul class="finds">${failed.map((f) => `<li class="signal"><span class="fi">${ICON.warn()}</span><span>No ${esc(f.kind)} language server: ${esc(f.why)}.</span></li>`).join("")}</ul>` +
        `<div class="acts"><button type="button" class="btn" data-act="reload">Read again</button></div></div>`
      : `<div class="empty"><h1>No stories yet</h1><p>A story starts at an entry point (a screen, an endpoint, a job, a function nothing in the project calls) and is shown once it reaches something outside the code: a package, a database, the network. Nothing in this project does yet.</p></div>`;
    return;
  }
  if (testsPage) {
    $("doc").innerHTML = testsPageHtml();
    return;
  }
  // only what the diagram cannot show by itself: the tests, what changed, what the code warns of, what is not resolved
  const S = view.S, m = opened!.model, reach = reachOf(m), d = diffOf(S.id);
  const t = testsOf(view);
  const fn = (n: number) => (n === 1 ? "function" : "functions");
  const tests = m.stories.length === reach.total
    ? `<span class="chip">No tests in this project</span>`
    : !t.of ? ""
    : t.hit ? `<span class="chip ok" title="Reaching a function is not the same as checking it">Tests reach ${t.hit} of the ${t.of} ${fn(t.of)} this story calls</span>`
    : `<span class="chip warn" title="Reaching a function is not the same as checking it">No test reaches the ${t.of === 1 ? "function" : `${t.of} functions`} this story calls</span>`;
  const changes = d ? `<span class="chip"><span class="dsign ${d.mark}">${SIGN[d.mark]}</span> ${esc(d.mark === "removed" ? "Removed since you last looked; shown as it was" : diffWords(d) + " since you last looked")}</span>` : "";
  // who else reaches where this story starts: the diagram starts there, so it cannot show it
  const by = reachedFrom(S);
  const from = by ? `<button type="button" class="chip link" data-act="focus" data-thing="${esc(by.id)}" title="Calls that could not be resolved are not counted, so more stories may reach it.">${plural(by.n, "other story reaches", "other stories reach")} ${esc(by.name)} · list them</button>` : "";
  const loc = (r: { file: string; from: number; to: number }) => `${r.file}:${r.from}${r.to !== r.from ? "–" + r.to : ""}`;
  const at = (f: Finding, i: number) => (opened!.model.files[f.ref.file] ? ` · <button type="button" class="link mono" data-act="finding" data-f="${i}">${esc(loc(f.ref))}</button>` : "");
  const signals = findings.map((f, i) => ({ f, i })).filter(({ f }) => f.kind === "signal");
  const gaps = findings.map((f, i) => ({ f, i })).filter(({ f }) => f.kind === "gap");
  // where the story can stop, even inside calls that are closed: one row per way it ends, a line per condition, each a
  // way to its step. The same condition written in more places is one line with each place.
  const rules = view.rules().map((g) => {
    const lines = new Map<string, ReturnItem[]>();
    for (const r of g.rules) {
      const cond = view!.condOf.get(r.id), says = view!.ruleSays(r);
      const text = [cond ? `<span class="cond">[${esc(cond)}]</span>` : "", says ? esc(says) : ""].filter(Boolean).join(" ") || `line ${r.ref.from}`;
      lines.set(text, [...(lines.get(text) ?? []), r]);
    }
    const whens = [...lines].map(([text, rs]) => {
      const files = new Map<string, number[]>();
      for (const r of rs) files.set(r.ref.file, [...(files.get(r.ref.file) ?? []), r.ref.from]);
      const at = [...files].map(([f, lines]) => `${f.slice(f.lastIndexOf("/") + 1)}:${lines.join(", ")}`).join(" · ");
      return `<button type="button" class="when" data-act="reveal" data-id="${esc(rs[0].id)}" title="${esc(rs.map((r) => loc(r.ref)).join("\n"))}"><span class="w">${text}</span><span class="at">${esc(at)}</span></button>`;
    });
    return { lines: whens.length, html: `<div class="rule"><span class="end${g.throws ? " throw" : ""}">${esc(g.end)}</span><span class="whens">${whens.join("")}</span></div>` };
  });
  const lines = rules.reduce((n, r) => n + r.lines, 0);
  lowerTabs = [
    ...(rules.length ? [{ key: "stop", title: "Where it can stop", n: lines, html: `<div class="rules">${rules.map((r) => r.html).join("")}</div>` }] : []),
    ...(signals.length ? [{ key: "shows", title: "What the code shows", n: signals.length, html: `<ul class="finds">${signals.map(({ f, i }) => `<li class="signal"><span class="fi">${ICON.warn()}</span><span>${esc(f.text)}${at(f, i)}</span></li>`).join("")}</ul>` }] : []),
    ...(gaps.length ? [{ key: "gaps", title: "Not resolved", n: gaps.length, html: `<ul class="finds">${gaps.map(({ f, i }) => `<li><span class="fi">${ICON.todo()}</span><span>${esc(f.text)}${at(f, i)}</span></li>`).join("")}</ul>` }] : []),
    {
      key: "how", title: "How to read this", html: `<div class="how">` +
        `<p>Numbers follow the order the code is written in, not the order it runs. They count every step as if every call were open, so a jump (2, then 10) is what a closed call holds; opening it fills the gap. A dashed arrow with a label is what comes back; a dashed line without one is inferred, not proven.</p>` +
        `<p>Steps that only the code holds are left out until Every step is on: what the screen holds on the way (a loading flag), a helper that works a value out, side work. What the screen holds as a way ends, such as the message it shows before it stops, stays, and so does anything that changed.</p>` +
        `<p>Scroll or drag to move, pinch or ${MOD}scroll to zoom toward the pointer, ${MOD}0 for the default view.</p>` +
        `<ul class="legend"><li><span class="tdot on"></span>a test reaches the function this step calls</li><li><span class="tdot"></span>no test reaches it</li>` +
        `<li><span class="mono dim">· or 401</span>other ways the call can end; open the call to see the rules that end it</li>` +
        `<li><span class="mono warm">⊣ [cond] 403</span>a rule: when the condition holds, the work ends here (in this color when it throws)</li>` +
        `<li><span class="mono dim">continue</span>this round of the loop ends here; break leaves the loop</li>` +
        `<li><span class="mono warm">· changes cart.items</span>the call changes a value it is given, or the object it is called on</li>` +
        `<li><span class="mono warm">· deletes items</span>a closed call changes this in the data, somewhere inside it</li>` +
        `<li><span class="mono acc">↗</span>another story starts here; it is told there, and a click goes to it</li>` +
        `<li><span class="mono dim">→ package</span>side work, with Every step on: a call whose result is not used, that many stories make; it never gets a column</li>` +
        `<li><span class="mono acc">▸ 2 functions</span>a column that stands for a place; click it to see its functions</li>` +
        `<li><span class="fchip">▸</span><span class="fchip on">▾</span>a call that opens on a click, and one that is open; the numbers of what it does are in this color</li>` +
        (view.english ? `<li><span class="dim">≈</span>in plain English by ${esc(view.englishBy)}, checked against the code; names and values are copied from it</li>` : "") + `</ul>` +
        (rules.length ? `<p>Where it can stop: what is thrown, by what it ends with, and a return before the end of the story's own function or of an endpoint. A return that only ends a helper early is a row inside that call.</p>` : "") +
        (S.coverage.length ? `<p>Lines behind the steps</p><div class="cov">${S.coverage.map((c) => `<span>${esc(c.fn)} <span class="${c.missing.length ? "warm" : "ok"}">${c.covered}/${c.total}</span>${c.missing.length ? ` <span class="dim">missing ${c.missing.join(", ")}</span>` : ""}</span>`).join("")}</div>` : "") +
        `</div>`,
    },
  ];
  $("doc").innerHTML =
    (tests || changes || from ? `<div class="chips">${tests}${from}${changes}</div>` : "") +
    `<div id="dia-slot"></div><div class="lower" id="lower"></div>`;
  $("dia-slot").replaceWith(canvas.el);
  lowerPane();
}

/** The pane under the diagram: what the diagram cannot show, one tab at a time. It is resized by dragging its tabs'
 *  bar and can be closed down to that bar; both are kept for this viewer. */
const lower = { tab: "", h: 200, shut: false };
try {
  Object.assign(lower, JSON.parse(localStorage.getItem("plainfold.lower") ?? "{}"));
} catch {
  // nothing kept: the defaults
}
const keepLower = () => {
  try {
    localStorage.setItem("plainfold.lower", JSON.stringify(lower));
  } catch {
    // not kept; it still works for this window
  }
};
let lowerTabs: { key: string; title: string; n?: number; html: string }[] = [];

function lowerPane() {
  const el = document.getElementById("lower");
  if (!el || !lowerTabs.length) return;
  const on = lowerTabs.find((t) => t.key === lower.tab) ?? lowerTabs[0];
  el.classList.toggle("shut", lower.shut);
  el.style.height = lower.shut ? "" : `${lower.h}px`;
  el.innerHTML =
    `<div class="lw-bar" role="tablist" aria-label="About this story" title="Drag to resize">` +
    lowerTabs.map((t) => `<button type="button" role="tab" data-act="lw-tab" data-tab="${t.key}" aria-selected="${t === on && !lower.shut}">${esc(t.title)}${t.n ? `<span class="n">${t.n}</span>` : ""}</button>`).join("") +
    `<span class="grow"></span><button type="button" class="ib" data-act="lw-shut" aria-label="${lower.shut ? "Open the pane" : "Close the pane"}">${lower.shut ? ICON.chevUp() : ICON.chevDown()}</button></div>` +
    (lower.shut ? "" : `<div class="lw-body" role="tabpanel">${on.html}</div>`);
}

// the pane's bar resizes it when dragged, and opens or closes it on a double click
$("doc").addEventListener("pointerdown", (e) => {
  const t = e.target as Element;
  if (e.button !== 0 || !t.closest(".lw-bar") || t.closest("button")) return;
  const y0 = e.clientY, h0 = $("lower").offsetHeight, max = Math.max(120, $("doc").clientHeight - 160);
  let moved = false;
  const move = (m: PointerEvent) => {
    if (!moved && Math.abs(m.clientY - y0) < 3) return;
    if (!moved && lower.shut) {
      lower.shut = false;
      lowerPane();
    }
    moved = true;
    lower.h = Math.round(Math.min(max, Math.max(90, h0 + y0 - m.clientY)));
    $("lower").style.height = `${lower.h}px`;
  };
  const up = () => {
    removeEventListener("pointermove", move);
    removeEventListener("pointerup", up);
    if (moved) keepLower();
  };
  addEventListener("pointermove", move);
  addEventListener("pointerup", up);
});
$("doc").addEventListener("dblclick", (e) => {
  const t = e.target as Element;
  if (!t.closest(".lw-bar") || t.closest("button")) return;
  lower.shut = !lower.shut;
  keepLower();
  lowerPane();
});

/** The Tests tab's page: for each story, the functions it calls on its main path, and whether tests reach them. */
function testsPageHtml(): string {
  const m = opened!.model, reach = reachOf(m);
  const tests = m.stories.filter((s) => s.trigger.kind === "test");
  if (!tests.length) return `<div class="empty"><h1>No tests</h1><p>No test in this project was found. Tests are found by the test runners plainfold knows; their files are not stories.</p></div>`;
  const rows: { st: Story; i: number; calls: { name: string; hit: number }[] }[] = [];
  m.stories.forEach((st, i) => {
    if (st.trigger.kind === "test") return;
    const v = new StoryView(st, undefined, undefined, undefined, reach);
    const calls = v.mainCalls().map((c) => ({ name: v.pname(c.to), hit: reach.tests(c.to).length }));
    if (calls.length) rows.push({ st, i, calls });
  });
  const none = rows.filter((r) => !r.calls.some((c) => c.hit)).length;
  const groups = new Map<string, typeof rows>();
  for (const r of rows) groups.set(r.st.trigger.route, [...(groups.get(r.st.trigger.route) ?? []), r]);
  return `<div class="doc-in tests-page"><h1>What the tests reach</h1>` +
    `<div class="tv-sum"><div><b>${tests.length}</b><span>${tests.length === 1 ? "test" : "tests"} in ${plural(new Set(tests.map((t) => t.trigger.route)).size, "file", "files")}</span></div>` +
    `<div><b>${rows.length}</b><span>stories that call functions of the project</span></div>` +
    `<div><b>${none}</b><span>of them: no function reached by a test</span></div></div>` +
    `<p class="quiet">A filled square is a function the story calls that at least one test reaches. Reaching a function is not checking it, and side work is not counted.</p>` +
    [...groups].map(([route, list]) => `<h4>${esc(route)}</h4>` + list.map((r) => {
      const hit = r.calls.filter((c) => c.hit).length;
      return `<button type="button" class="tv-row" data-act="story" data-id="${r.i}"><span class="nm">${esc(flowName(r.st).main)}</span>` +
        `<span class="strip">${r.calls.map((c) => `<i class="${c.hit ? "y" : ""}" title="${esc(c.name)}: ${c.hit ? plural(c.hit, "test", "tests") : "no test"}"></i>`).join("")}</span>` +
        `<span class="rt${hit ? "" : " zero"}">${hit} / ${r.calls.length}</span></button>`;
    }).join("")).join("") +
    `</div>`;
}

/** Draws the story. The view stays where it looks: opening a call shows rows (`slide`: the rows below move down to
 *  make room, the new ones fade in) and can push the columns aside; the step `hold` (the one clicked) stays where it
 *  is on screen. */
function draw(slide = false, hold?: string) {
  if (!view || testsPage) return;
  const prev = slide ? view.ys : undefined;
  const x0 = hold ? leftOf(hold) : null;
  canvas.set(view.render(), prev);
  const x1 = hold ? leftOf(hold) : null;
  if (x0 !== null && x1 !== null) canvas.hold(x0 - x1);
  if (homeNext) {
    const at = homeNext === "story" ? view.firstMark() : view.sel, b = at ? box(at) : null;
    homeNext = "";
    canvas.home();
    if (b) canvas.reveal(b, false);
  }
  $("insp").innerHTML = panel(view, opened!.model, findings, panelState);
}

/** Where a step is drawn, in the diagram's own units. */
function box(id: string): DOMRect | null {
  const k = view?.ops.findIndex((o) => o.it?.id === id || o.synth?.id === id) ?? -1;
  const el = k >= 0 ? canvas.el.querySelector<SVGGraphicsElement>(`.sheet [data-k="${k}"]`) : null;
  return el ? el.getBBox() : null;
}

/** Where a step's row starts across: its leftmost lifeline, in the diagram's own units. */
function leftOf(id: string): number | null {
  const k = view?.ops.findIndex((o) => o.it?.id === id || o.synth?.id === id) ?? -1;
  const el = k >= 0 ? canvas.el.querySelector<SVGGraphicsElement>(`.sheet [data-k="${k}"] .hit`) : null;
  return el ? el.getBBox().x : null;
}

/** Opens what a step sits inside, selects it and brings it into view. */
function revealStep(id: string) {
  if (!view) return;
  view.reveal(id);
  redraw(true);
  const b = box(id);
  if (b) canvas.reveal(b);
}

function statusBar() {
  const o = opened!;
  const steps = o.model.stories.reduce((n, s) => n + stepCount(s), 0);
  $("status").innerHTML =
    o.servers.map((s) => `<button type="button" class="st" data-act="settings" title="Language servers: ${MOD},"><span class="dot ok"></span>${esc(s.name)} ready</button>`).join("") +
    o.failed.map((f) => `<button type="button" class="st warm" data-act="tab" data-tab="setup"><span class="dot warn"></span>no ${esc(f.kind)} language server</button>`).join("") +
    (o.missing.length ? `<button type="button" class="st warm" data-act="tab" data-tab="setup">${ICON.warn(12)}Dependencies not installed</button>` : "") +
    `<span class="grow"></span>` +
    `<span class="st">${plural(o.model.stories.length, "story", "stories")} · ${plural(steps, "step", "steps")} · read in ${(o.model.stats.ms / 1000).toFixed(1)} s</span>` +
    `<button type="button" class="st link" data-act="reload" title="Read the project again (${MOD}R)">Reload</button>` +
    englishStatus();
}

// ---------- English ----------
/** The steps in plain English, for the view: looked up as they arrive, so a redraw shows the newest. */
function setEnglish(v: StoryView) {
  v.english = appState.english ? (it: Item) => eng.texts.get(siteKey(it) ?? "") : null;
  v.englishBy = eng.model || eng.key?.model || "a model";
}

/** Starts writing the English of read `id`, unless it is off or there is no key for a model. */
async function startEnglish(id: number, path: string) {
  eng.texts.clear();
  Object.assign(eng, { id, done: 0, total: 0, tokens: 0, error: "", running: false, rejected: {} });
  if (!appState.english) return statusBar();
  eng.key = await backend.englishKey();
  if (!eng.key.from || id !== openedId) return statusBar();
  eng.running = true;
  statusBar();
  try {
    eng.model = await backend.describe(id, path);
  } catch (e) {
    eng.running = false;
    eng.error = String(e);
  }
  if (view) setEnglish(view);
  statusBar();
}

let englishFrame = 0;
backend.onEnglish(
  (b: EnglishBatch) => {
    if (b.id !== eng.id) return;
    for (const [k, v] of Object.entries(b.texts)) eng.texts.set(k, v);
    Object.assign(eng, { done: b.done, total: b.total, tokens: eng.tokens + b.tokensIn + b.tokensOut });
    // texts arrive request by request: draw them a few at a time
    cancelAnimationFrame(englishFrame);
    englishFrame = requestAnimationFrame(() => {
      if (screen !== "story" || !opened) return;
      statusBar();
      if (appState.english) draw();
    });
  },
  (e: EnglishEnd) => {
    if (e.id !== eng.id) return;
    eng.running = false;
    eng.error = e.stopped ? "" : e.error ?? "";
    eng.rejected = e.rejected;
    if (screen === "story" && opened) statusBar();
  },
);

function englishTitle(): string {
  return appState.english
    ? `Steps in plain English, written by ${eng.model || eng.key?.model || "a model"} from the code and checked against it. Off: every label is code text.`
    : "Every label is code text. On: steps in plain English, written by a model from the code.";
}

function englishStatus(): string {
  if (!appState.english) return `<span class="st">English off · every label is code text</span>`;
  if (eng.key && !eng.key.from) return `<button type="button" class="st link" data-act="settings">English: add an API key</button>`;
  if (eng.error) return `<button type="button" class="st warm" data-act="settings" title="${esc(eng.error)}">${ICON.warn(12)}English stopped: ${esc(eng.error.length > 60 ? eng.error.slice(0, 59) + "…" : eng.error)}</button>`;
  const by = esc(eng.model || "the model");
  const tokens = eng.tokens ? ` · ${eng.tokens >= 1000 ? `${Math.round(eng.tokens / 1000)}k` : eng.tokens} tokens` : "";
  if (eng.running) return `<span class="st" title="A step without its text yet shows its code"><span class="dot busy"></span>English · ${by} · writing ${eng.done} of ${eng.total || "…"} steps${tokens}</span>`;
  // done: the steps whose text did not pass a check keep their code, and why is in the tooltip
  const code = Math.max(0, eng.total - eng.texts.size);
  const why = Object.entries(eng.rejected).sort((a, b) => b[1] - a[1]).map(([w, n]) => `${n}: ${w}`).join("\n");
  const title = `Written by ${by} from the code and checked against it.` + (code ? ` A step whose text did not pass a check shows its code:\n${why}` : "");
  return `<span class="st" title="${esc(title)}">English · ${by} · ${plural(eng.texts.size, "step", "steps")} in words${code ? `, ${code} as code` : ""}${tokens}</span>`;
}

// ---------- go to ----------
function openPalette() {
  if (screen !== "story" || !opened) return;
  const all = entries(opened.model);
  pal = { q: "", at: 0, all, shown: palFilter(all, "") };
  $("palette").hidden = false;
  const input = $("pal-q") as HTMLInputElement;
  input.value = "";
  renderPalette();
  input.focus();
}

function renderPalette() {
  if (!pal) return;
  $("pal-list").innerHTML = palList(pal.shown, pal.q, pal.at);
  $("pal-list").querySelector(".on")?.scrollIntoView({ block: "nearest" });
}

function closePalette() {
  pal = null;
  $("palette").hidden = true;
}

function pick(e: Entry, inEditor: boolean) {
  closePalette();
  if (e.kind === "file" || inEditor) {
    const file = e.file ?? opened!.model.stories[e.story!].participants.find((p) => p.id === e.participant)?.ref?.file ?? opened!.model.stories[e.story!].trigger.ref.file;
    const line = e.kind === "participant" ? (opened!.model.stories[e.story!].participants.find((p) => p.id === e.participant)?.ref?.from ?? 1) : e.kind === "story" ? opened!.model.stories[e.story!].trigger.ref.from : 1;
    return edit(file, line);
  }
  loadStory(e.story!);
  if (e.kind === "participant") {
    view!.sel = "p:" + e.participant;
    focusOn(e.participant!);
    draw();
  }
}

// ---------- settings ----------
async function openSettings() {
  closePalette();
  [settings, eng.key] = await Promise.all([backend.serverSettings(), backend.englishKey()]);
  $("settings").hidden = false;
  renderSettings();
  // what the servers hold changes while they run
  clearInterval(settingsTimer);
  settingsTimer = window.setInterval(async () => {
    if (!settings) return clearInterval(settingsTimer);
    settings = await backend.serverSettings();
    renderSettings();
  }, 5000);
}

function closeSettings() {
  settings = null;
  clearInterval(settingsTimer);
  $("settings").hidden = true;
}

function renderSettings() {
  const s = settings;
  if (!s) return;
  const time = (secs: number) => new Date(secs * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const size = (b: number | null) => (b === null ? "not measured" : b >= 1e9 ? `${(b / 1e9).toFixed(1)} GB` : `${Math.round(b / 1e6)} MB`);
  const sw = (on: boolean) => `<span class="sw${on ? " on" : ""}"><i></i></span>`;
  const lowMemory = platform === "mac" ? ", and when the Mac is low on memory" : "";
  const running = s.running.length
    ? `<div class="set-g">Running for ${esc(s.project?.split("/").pop() ?? "this project")}</div>` +
      s.running.map((h) => `<div class="set-srv"><span class="grow"><span class="set-t">${esc(h.name)}</span><span class="set-d">since ${time(h.since)} · ${plural(h.reads, "read", "reads")}</span></span><span class="set-mb">${size(h.bytes)}</span></div>`).join("")
    : `<p class="set-d set-none">${s.keep ? "None running now." : "None kept."}${s.closed ? ` Closed at ${time(s.closed[1])}: ${esc(s.closed[0])}.` : ""}</p>`;
  // the key being typed survives the redraw every few seconds
  const typing = document.getElementById("key-in") as HTMLInputElement | null;
  const typed = typing?.value ?? "", focused = !!typing && document.activeElement === typing;
  const key = eng.key;
  const keyFrom = !key?.from ? "No key saved." : key.from === "keychain" ? "A key is saved in the keychain." : "Using OPENAI_API_KEY from the environment.";
  $("settings-body").innerHTML =
    `<h3>English</h3>` +
    `<button type="button" class="set-row" data-act="english" aria-pressed="${appState.english}">${sw(appState.english)}<span class="grow"><span class="set-t">Show steps in plain English</span>` +
    `<span class="set-d">Written by ${esc(key?.model ?? "a model")} from the code of each function a story shows, and checked against it: names and values are copied from the code, and a text that fails a check is not shown. Comments and documentation are never sent. Texts are kept on this computer, so the same code is not sent twice.</span></span></button>` +
    `<div class="set-key"><span class="set-t">OpenAI API key</span><span class="set-d">${keyFrom} It is sent only to OpenAI.</span>` +
    `<div class="key-row"><input id="key-in" type="password" placeholder="${key?.from === "keychain" ? "Replace the saved key" : "sk-…"}" autocomplete="off" spellcheck="false" aria-label="OpenAI API key">` +
    `<button type="button" class="btn" data-act="key-save">Save</button>` +
    (key?.from === "keychain" ? `<button type="button" class="btn" data-act="key-remove">Remove</button>` : "") +
    `</div></div>` +
    `<h3>Language servers</h3>` +
    `<button type="button" class="set-row" data-act="keep" aria-pressed="${s.keep}">${sw(s.keep)}<span class="grow"><span class="set-t">Keep them running between reads</span>` +
    `<span class="set-d">Reading the project again is faster: they already know it. They close when this window closes or opens another project, after 20 minutes away from this window${lowMemory}. Off: every read starts them again.</span></span></button>` +
    running +
    `<h3>Downloads</h3>` +
    `<button type="button" class="set-row" data-act="auto-dl" aria-pressed="${appState.autoDownload}">${sw(appState.autoDownload)}<span class="grow"><span class="set-t">Download the language servers a project needs without asking</span>` +
    `<span class="set-d">From the npm registry, and Node.js from nodejs.org when this machine has none that runs them; at the versions plainfold pins, checked before use.</span></span></button>`;
  const input = document.getElementById("key-in") as HTMLInputElement | null;
  if (input) {
    input.value = typed;
    if (focused) input.focus();
  }
}

async function saveKey(key: string | null) {
  try {
    eng.key = await backend.setEnglishKey(key);
    const input = document.getElementById("key-in") as HTMLInputElement | null;
    if (input) input.value = "";
    toast(key ? "Saved the key in the keychain" : "Removed the key");
    renderSettings();
    if (opened && screen === "story" && appState.english) void startEnglish(openedId, opened.path);
  } catch (e) {
    toast(String(e));
  }
}

// ---------- actions ----------
async function edit(file: string, line: number) {
  try {
    const by = await backend.openInEditor(opened!.path, file, line);
    toast(`Opened ${file}:${line} in ${by}`);
  } catch (e) {
    toast(String(e));
  }
}

let toastTimer = 0;
function toast(text: string) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (t.hidden = true), 2400);
}

/** Clears every mark: the next Reload compares with the code as it is now. */
async function markSeen() {
  if (!opened) return;
  await backend.markSeen(opened.path);
  const id = storyAt(storyIdx)?.id;
  opened.diff = [];
  const i = id ? indexOf(id) : -1;
  if (opened.model.stories.length) loadStory(Math.max(0, i), false);
  else { view = null; renderStory(); }
}

async function openFolder() {
  const p = await backend.pickFolder();
  if (p) openPath(p);
}

async function toWelcome() {
  if (reading) backend.stopWaiting(reading.id, "*");
  backend.closeProject();
  readId++;
  reading = null;
  appState = await backend.startup();
  renderWelcome();
  show("welcome");
  await backend.windowFor("welcome");
}

function redraw(slide = false, hold?: string) {
  if (screen !== "story" || !view) return;
  view.every = every;
  toolbar();
  draw(slide, hold);
}

/** The columns are about to move (another level, a place opened or closed, every step): a view the user has not moved
 *  goes to the default view again. */
const columnsMove = () => (homeNext = canvas.moved ? "" : "columns");

function handle(t: HTMLElement) {
  const d = t.dataset;
  if (d.k !== undefined && view) {
    // a step that starts another story is a reference to it: the click goes there
    const o = view.ops[+d.k];
    const opens = o && !o.ghostOf ? o.opens ?? (o.type === "msg" && o.it.t === "call" ? refStory(o.it) : undefined) : undefined;
    const i = opens ? opened!.model.stories.findIndex((s) => s.id === opens) : -1;
    if (i >= 0) return loadStory(i);
    const id = o?.synth?.id ?? o?.it?.id;
    view.act(+d.k);
    return redraw(true, id);
  }
  // a column that stands for a place opens into its functions; any other column shows what it is
  if (d.p !== undefined && view) {
    if (d.p.startsWith("g:")) {
      view.expanded.add(d.p.slice(2));
      columnsMove();
    } else view.sel = "p:" + d.p;
    return redraw();
  }
  if (d.g !== undefined && view) {
    view.expanded.delete(d.g);
    columnsMove();
    return redraw();
  }
  switch (d.act) {
    case "open-folder": return void openFolder();
    case "open-path": return void openPath(d.path!);
    case "switch": return void toWelcome();
    case "retry": return void (reading && openPath(reading.path));
    case "skip": return void (reading && backend.stopWaiting(reading.id, d.kind!));
    case "dl-yes":
    case "dl-no": {
      const always = ($("dl-always") as HTMLInputElement | null)?.checked;
      if (always && d.act === "dl-yes") {
        appState.autoDownload = true;
        backend.setAutoDownload(true);
      }
      return reading?.ask?.answer(d.act === "dl-yes");
    }
    case "reload": return void (opened && openPath(opened.path));
    case "seen": return void markSeen();
    case "story":
      loadStory(+d.id!);
      if (d.step) revealStep(d.step);
      return;
    case "focus": return focusOn(d.thing!);
    case "fold": {
      const key = d.key!, open = t.getAttribute("aria-expanded") === "true";
      folded.set(key, !open);
      return sidebar();
    }
    case "unfocus":
      focus = null;
      return sidebar();
    case "tests-page":
      testsPage = true;
      return renderStory();
    case "tab":
      tab = d.tab as typeof tab;
      // the Tests tab opens on its own page, unless a test is what is shown
      testsPage = tab === "tests" && storyAt(storyIdx)?.trigger.kind !== "test";
      return screen === "story" ? renderStory() : segment();
    case "side":
      hideSide = !hideSide;
      return layout(), redraw();
    case "insp":
      hideInsp = !hideInsp;
      $("insp").hidden = hideInsp;
      return layout(), redraw();
    case "back":
      if (back.length) { fwd.push(storyIdx); loadStory(back.pop()!, false); }
      return;
    case "fwd":
      if (fwd.length) { back.push(storyIdx); loadStory(fwd.pop()!, false); }
      return;
    case "every":
      every = !every;
      columnsMove();
      return redraw();
    case "english":
      appState.english = !appState.english;
      void backend.setEnglish(appState.english);
      // what was written is kept, so starting again shows it at once and asks only for the rest
      if (appState.english && opened && !eng.running) void startEnglish(openedId, opened.path);
      if (!appState.english) eng.running = false;
      if (view) setEnglish(view);
      if (settings) renderSettings();
      if (screen === "story") { redraw(); statusBar(); }
      return;
    case "key-save": return void saveKey((document.getElementById("key-in") as HTMLInputElement | null)?.value.trim() || null);
    case "key-remove": return void saveKey(null);
    case "level":
      level = d.level as Level;
      view?.setLevel(level);
      columnsMove();
      return redraw();
    case "zout": return canvas.zoomBy(1 / 1.2);
    case "zin": return canvas.zoomBy(1.2);
    case "zfit": return canvas.home(true);
    case "lw-tab":
      lower.tab = d.tab!;
      lower.shut = false;
      keepLower();
      return lowerPane();
    case "lw-shut":
      lower.shut = !lower.shut;
      keepLower();
      return lowerPane();
    case "toggle": view?.toggle(d.id!); return redraw(true, d.id);
    case "reveal": return revealStep(d.id!);
    case "finding": if (view) view.sel = "f:" + d.f; return redraw();
    case "more":
      if (panelState.expanded.has(d.key!)) panelState.expanded.delete(d.key!);
      else panelState.expanded.add(d.key!);
      return draw();
    case "edit": return void edit(d.file!, +d.line!);
    case "copy":
      navigator.clipboard?.writeText(d.text!).then(() => toast(`Copied ${d.text}`), () => toast("Could not copy"));
      return;
    case "palette": return openPalette();
    case "settings": return void openSettings();
    case "set-close": return closeSettings();
    case "keep":
      if (!settings) return;
      appState.keepServers = !settings.keep;
      return void backend.setKeepServers(appState.keepServers).then(openSettings);
    case "auto-dl":
      appState.autoDownload = !appState.autoDownload;
      backend.setAutoDownload(appState.autoDownload);
      return renderSettings();
    case "pal-close": return closePalette();
    case "pick": return pal ? pick(pal.shown[+d.i!], false) : undefined;
    case "win-min": return void backend.windowControl("min");
    case "win-max": return void backend.windowControl("max");
    case "win-close": return void backend.windowControl("close");
  }
}

document.addEventListener("click", (e) => {
  const t = (e.target as HTMLElement).closest<HTMLElement>("[data-act],[data-k],[data-p],[data-g]");
  if (t && !(t as HTMLButtonElement).disabled) handle(t);
});

document.addEventListener("keydown", (e) => {
  const mod = platform === "mac" ? e.metaKey : e.ctrlKey;
  if (settings && e.key === "Escape") { e.preventDefault(); return closeSettings(); }
  if (mod && e.key === ",") { e.preventDefault(); return settings ? closeSettings() : void openSettings(); }
  if (settings && e.key === "Enter" && (e.target as HTMLElement).id === "key-in") { e.preventDefault(); return act("key-save"); }
  if (settings) return;
  if (focus && !pal && e.key === "Escape") { e.preventDefault(); focus = null; return sidebar(); }
  if (pal) {
    if (e.key === "Escape") { e.preventDefault(); return closePalette(); }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      pal.at = (pal.at + (e.key === "ArrowDown" ? 1 : -1) + pal.shown.length) % Math.max(1, pal.shown.length);
      return renderPalette();
    }
    if (e.key === "Enter" && pal.shown[pal.at]) { e.preventDefault(); return pick(pal.shown[pal.at], mod); }
  }
  if (mod && !e.shiftKey && !e.altKey) {
    const k = e.key.toLowerCase();
    if (k === "k") { e.preventDefault(); return pal ? closePalette() : openPalette(); }
    if (k === "o") { e.preventDefault(); return void openFolder(); }
    if (k === "r") { e.preventDefault(); if (opened && screen === "story") openPath(opened.path); return; }
    if (screen !== "story") return;
    if (k === "f") { e.preventDefault(); tab = "stories"; renderStory(); return ($("q") as HTMLInputElement).focus(); }
    if (k === "[") { e.preventDefault(); return act("back"); }
    if (k === "]") { e.preventDefault(); return act("fwd"); }
    if (k === "=" || k === "+") { e.preventDefault(); return act("zin"); }
    if (k === "-") { e.preventDefault(); return act("zout"); }
    if (k === "0") { e.preventDefault(); return act("zfit"); }
  }
  // the arrow keys move the diagram, unless something that takes them has the focus
  const keys: Record<string, [number, number]> = { ArrowLeft: [1, 0], ArrowRight: [-1, 0], ArrowUp: [0, 1], ArrowDown: [0, -1] };
  if (keys[e.key] && !pal && screen === "story" && view && !testsPage && !(e.target as Element).closest("input, textarea, select, .stories, .insp, .lower")) {
    e.preventDefault();
    const step = e.shiftKey ? 240 : 80;
    return canvas.panBy(keys[e.key][0] * step, keys[e.key][1] * step);
  }
  if ((e.key === "Enter" || e.key === " ") && !pal) {
    const t = (e.target as HTMLElement).closest?.<HTMLElement>("[data-k],[data-p],[data-g]");
    if (t) { e.preventDefault(); handle(t); }
  }
});

function act(a: string) {
  const b = document.createElement("button");
  b.dataset.act = a;
  handle(b);
}

$("q").addEventListener("input", () => sidebar());
$("pal-q").addEventListener("input", (e) => {
  if (!pal) return;
  pal.q = (e.target as HTMLInputElement).value;
  pal.shown = palFilter(pal.all, pal.q);
  pal.at = 0;
  renderPalette();
});
$("show-welcome").addEventListener("change", (e) => {
  appState.showWelcome = (e.target as HTMLInputElement).checked;
  backend.setShowWelcome(appState.showWelcome);
});
backend.onDrop((p) => openPath(p));
backend.onChanges((id, list) => {
  if (id === openedId && opened) {
    changed = list;
    if (screen === "story") toolbar();
  } else if (id === readId) early = { id, changes: list }; // told before the read's result arrived
});

// ---------- start ----------
(async () => {
  appState = await backend.startup();
  const preview = params.get("preview");
  const start = params.get("screen");
  if (preview && start !== "welcome") {
    if (start === "opening") {
      // the reading screen as it looks while a language server settles (browser preview only)
      reading = { id: 0, path: preview, name: "bookshelf", steps: firstSteps() };
      show("opening");
      progress({ step: "loaded", name: reading.name, files: 12 });
      progress({ step: "adapters", names: ["React", "Next.js", "Supabase", "web platform"] });
      progress({ step: "server-starting", kind: "typescript" });
      progress({ step: "server-waiting", kind: "typescript", seconds: 74, doing: "Initializing JS/TS language features…" });
      return renderOpening();
    }
    await openPath(preview);
    if (params.get("story")) {
      // `&story=<id>`: that story, as if picked in the list
      const i = (opened as Opened | null)?.model.stories.findIndex((st) => st.id === params.get("story")) ?? -1;
      if (i >= 0) loadStory(i);
    }
    if (params.get("changed")) {
      // `&changed=a.ts,b.ts`: those files shown as changed since the read
      changed = params.get("changed")!.split(",").map((file) => ({ file, how: "changed" }));
      toolbar();
    }
    if (params.get("settings")) void openSettings();
    // `&focus=<id>`: the list narrowed to the stories that reach it; `&tab=data`: the Data tab
    if (params.get("focus")) focusOn(params.get("focus")!);
    if (params.get("tab") === "data") { tab = "data"; renderStory(); }
    if (params.get("tab") === "setup") { tab = "setup"; renderStory(); }
    // `&tab=tests`: the Tests tab and its page; `&level=parts|functions`: the columns at that level
    if (params.get("tab") === "tests") { tab = "tests"; testsPage = true; renderStory(); }
    const v = view as StoryView | null;
    if (params.get("level") && v) { level = params.get("level") as Level; v.setLevel(level); columnsMove(); redraw(); }
    // `&open=all`: every call opened; `&select=<step id>`: that step selected, made visible
    if (params.get("open") === "all" && v) { v.open = new Set(v.allCalls.map((c) => c.id)); redraw(); }
    if (params.get("select") && v) revealStep(params.get("select")!);
    if (params.get("palette")) {
      openPalette();
      ($("pal-q") as HTMLInputElement).value = params.get("palette")!;
      $("pal-q").dispatchEvent(new Event("input"));
    }
    return;
  }
  if (start === "welcome" && params.get("settings")) void openSettings();
  if (appState.open) return openPath(appState.open);
  if (!appState.showWelcome && appState.recent[0]) return openPath(appState.recent[0].path);
  renderWelcome();
  show("welcome");
})();
