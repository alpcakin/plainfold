// The story list as the places a person meets (DECISIONS §4): what runs as the app starts and the layouts around its
// screens, then each screen in the order a person reaches it from the start, holding what it does when it opens and
// the controls drawn on it, nested in the parts of the screen they are written in; then what runs on a server, and
// what no adapter places. Made from the model only: stories, what starts in what, what draws what, and where a step
// goes (its reference). Ids are keys here, never read.
import type { CallItem, Item, ProjectModel, Story } from "./model";
import { refStory, walk } from "./model";

/** A part of a screen that holds controls: a component, with the parts it draws that hold controls too. */
export interface Part {
  id: string;
  name: string;
  /** user actions whose control is written in this part (story indexes) */
  actions: number[];
  parts: Part[];
  /** how many screens and layouts draw it (1: only this one) */
  on: number;
}
/** A screen, or what runs around screens or as the app starts: its story and what is drawn in it. */
export interface Place {
  story: number;
  actions: number[];
  parts: Part[];
  /** a screen the start of the app leads to, along the screens and controls before it */
  reached: boolean;
}
export interface Places {
  /** what runs as the app starts, and the layouts around its screens */
  app: Place[];
  /** in the order a person reaches them; screens nothing leads to last, in the model's order */
  screens: Place[];
  /** parts with controls that no screen or layout is found to draw */
  loose: Part[];
  /** endpoints by the file that answers them */
  endpoints: [string, number[]][];
  /** what starts by itself on a server: jobs, its start and stop, what runs before each request */
  own: number[];
  /** entry points no adapter places */
  other: number[];
  /** endpoints most of what a person does reaches (a session refreshed before each request): said once */
  ambient: string[];
}

const UI = new Set(["device", "browser"]);

const cache = new WeakMap<ProjectModel, Places>();
export function placesOf(m: ProjectModel): Places {
  const known = cache.get(m);
  if (known) return known;
  const stories = m.stories;
  const at = new Map<string, number>(stories.map((s, i) => [s.id, i]));
  // the function each story starts in
  const start = new Map<string, string>();
  for (const t of m.index ?? []) for (const u of t.uses) if (u.how === "starts") start.set(u.story, t.id);
  // what draws what: drawn by the code of `from`
  const kids = new Map<string, Set<string>>(), names = new Map<string, string>();
  for (const t of m.index ?? []) {
    names.set(t.id, t.name);
    for (const u of t.uses) if (u.how === "draws" && u.from && u.from !== t.id) (kids.get(u.from) ?? kids.set(u.from, new Set()).get(u.from)!).add(t.id);
  }
  const kind = (s: Story) => s.trigger.kind;
  const screens = stories.map((s, i) => [s, i] as const).filter(([s]) => kind(s) === "screen-open");
  const app = stories
    .map((s, i) => [s, i] as const)
    .filter(([s]) => kind(s) === "lifecycle" && (s.trigger.around !== undefined || UI.has(s.trigger.place ?? "")))
    .sort(([a], [b]) => Number(a.trigger.around !== undefined) - Number(b.trigger.around !== undefined) || (a.trigger.around ?? "").length - (b.trigger.around ?? "").length || a.trigger.route.split("/").length - b.trigger.route.split("/").length);
  // the roots: each screen's function, and each layout's or start's
  const root = new Map<string, number>();
  for (const [s, i] of [...screens, ...app]) {
    const fn = start.get(s.id);
    if (fn && !root.has(fn)) root.set(fn, i);
  }
  // from each root down, the first way found to every part it draws, never into another root
  const under = new Map<number, Map<string, string | null>>();
  for (const [fn, i] of root) {
    const seen = new Map<string, string | null>([[fn, null]]);
    const q = [fn];
    while (q.length) {
      const x = q.shift()!;
      for (const k of kids.get(x) ?? []) {
        if (seen.has(k) || (root.has(k) && root.get(k) !== i)) continue;
        seen.set(k, x);
        q.push(k);
      }
    }
    under.set(i, seen);
  }
  // user actions by the function their control is written in
  const byHome = new Map<string, number[]>();
  stories.forEach((s, i) => {
    if (kind(s) !== "user-action") return;
    const t = s.items.find((x) => x.t === "self" && x.kind === "trigger");
    const home = t && t.t === "self" ? t.at : undefined;
    if (home) (byHome.get(home) ?? byHome.set(home, []).get(home)!).push(i);
  });
  const placed = new Set<number>();
  const onCount = (id: string) => [...under.values()].filter((t) => t.has(id)).length;
  const build = (i: number): Place => {
    const tree = under.get(i)!;
    const fn = [...tree].find(([, p]) => p === null)?.[0];
    const holds = new Map<string, boolean>();
    const has = (x: string): boolean => {
      if (holds.has(x)) return holds.get(x)!;
      holds.set(x, false);
      const v = !!byHome.get(x)?.length || [...tree].some(([y, p]) => p === x && has(y));
      holds.set(x, v);
      return v;
    };
    const part = (x: string): Part => {
      const acts = byHome.get(x) ?? [];
      acts.forEach((a) => placed.add(a));
      return { id: x, name: names.get(x) ?? x, actions: acts, parts: [...tree].filter(([y, p]) => p === x && !root.has(y) && has(y)).map(([y]) => part(y)), on: onCount(x) };
    };
    const top = fn ? part(fn) : { actions: [], parts: [] };
    return { story: i, actions: top.actions, parts: top.parts, reached: true };
  };
  const appPlaces = app.map(([, i]) => build(i));
  // where each screen and control leads: the screens its steps refer to
  const refs = (s: Story): string[] => {
    const out: string[] = [];
    walk(s.items, (it) => {
      if (it.t === "call") {
        const r = refStory(it as CallItem);
        if (r) out.push(r);
      }
    });
    return out;
  };
  const screenIds = new Set(screens.map(([s]) => s.id));
  const actionsIn = (p: { actions: number[]; parts: Part[] }): number[] => [...p.actions, ...p.parts.flatMap(actionsIn)];
  const built = new Map<number, Place>(screens.map(([, i]) => [i, build(i)]));
  const leads = new Map<number, number[]>();
  for (const [s, i] of screens) {
    const to = [...refs(s), ...actionsIn(built.get(i)!).flatMap((a) => refs(stories[a]))].filter((r) => screenIds.has(r) && r !== s.id);
    leads.set(i, [...new Set(to)].map((r) => at.get(r)!));
  }
  // the start: where the app's start and its layouts lead, else the screen at the root address, else the screens
  // nothing leads to
  let first = appPlaces.flatMap((p) => [p.story, ...actionsIn(p)]).flatMap((i) => refs(stories[i])).filter((r) => screenIds.has(r)).map((r) => at.get(r)!);
  const home = screens.find(([s]) => s.trigger.route === "/");
  if (home) first.push(home[1]);
  if (!first.length) {
    const led = new Set([...leads.values()].flat());
    first = screens.map(([, i]) => i).filter((i) => !led.has(i));
  }
  const order: number[] = [], seen = new Set<number>();
  const q = [...first];
  while (q.length) {
    const i = q.shift()!;
    if (seen.has(i)) continue;
    seen.add(i);
    order.push(i);
    q.push(...(leads.get(i) ?? []));
  }
  const screenPlaces = [...order.map((i) => built.get(i)!), ...screens.filter(([, i]) => !seen.has(i)).map(([, i]) => ({ ...built.get(i)!, reached: false }))];
  // parts with controls no root draws. One a step of a place goes to, or what draws it (a sheet a control opens, a
  // screen opened by its function), is in that place; the others are not found on a screen.
  const loose: Part[] = [];
  const drawnBy = new Map<string, string[]>();
  for (const [p, ks] of kids) for (const k of ks) (drawnBy.get(k) ?? drawnBy.set(k, []).get(k)!).push(p);
  const ofStory = new Map<number, Place>();
  for (const pl of [...appPlaces, ...screenPlaces]) for (const i of [pl.story, ...actionsIn(pl)]) if (!ofStory.has(i)) ofStory.set(i, pl);
  const callsTo = new Map<string, number>();
  for (const [i] of ofStory)
    walk(stories[i].items, (it) => {
      if (it.t === "call" && it.from !== it.to && !callsTo.has(it.to)) callsTo.set(it.to, i);
    });
  for (const h of [...byHome.keys()].filter((h) => byHome.get(h)!.some((a) => !placed.has(a)))) {
    const p: Part = { id: h, name: names.get(h) ?? stories[byHome.get(h)![0]].trigger.route, actions: byHome.get(h)!.filter((a) => !placed.has(a)), parts: [], on: 1 };
    // the nearest of it and what draws it that a placed story reaches
    const seen = new Set([h]), q = [h];
    let at: number | undefined;
    while (q.length && at === undefined) {
      const x = q.shift()!;
      at = callsTo.get(x);
      for (const y of drawnBy.get(x) ?? []) if (!seen.has(y)) (seen.add(y), q.push(y));
    }
    const pl = at !== undefined ? ofStory.get(at) : undefined;
    if (pl) {
      pl.parts.push(p);
      p.actions.forEach((a) => placed.add(a));
    } else loose.push(p);
  }
  // the server, and the rest
  const endpoints = new Map<string, number[]>();
  const own: number[] = [], other: number[] = [];
  const inApp = new Set(app.map(([, i]) => i));
  stories.forEach((s, i) => {
    const k = kind(s);
    if (k === "endpoint") (endpoints.get(s.trigger.route) ?? endpoints.set(s.trigger.route, []).get(s.trigger.route)!).push(i);
    else if ((k === "lifecycle" || k === "schedule") && !inApp.has(i)) own.push(i);
    else if ((k === "function" || k === "file") && s.trigger.place === "server") own.push(i);
    else if (k === "function" || k === "file") other.push(i);
  });
  // endpoints at least half of what a person does reaches
  const ui = stories.filter((s) => kind(s) === "screen-open" || kind(s) === "user-action");
  const n = new Map<string, number>();
  for (const s of ui) {
    const hit = new Set<string>();
    walk(s.items, (it) => {
      if (it.t === "call" && s.participants.find((p) => p.id === it.to)?.kind === "endpoint") hit.add(it.to);
    });
    for (const e of hit) n.set(e, (n.get(e) ?? 0) + 1);
  }
  const ambient = ui.length >= 4 ? [...n].filter(([, k]) => k >= ui.length / 2).map(([e]) => e) : [];
  const out: Places = { app: appPlaces, screens: screenPlaces, loose, endpoints: [...endpoints], own, other, ambient };
  cache.set(m, out);
  return out;
}

/** What a person would notice a story do, in a few words: what it changes in the data, the endpoints and sign-in it
 *  calls, the screens it goes to, else what it reads. `screen`: names a story a step goes to. */
export function outcome(s: Story, ambient: Set<string>, screen: (id: string) => string | undefined, max = 3): { text: string; tone: "" | "warm" | "link" }[] {
  const changes = new Set<string>(), reads = new Set<string>(), calls = new Set<string>(), goes = new Set<string>();
  const runs = new Set<string>();
  const part = (id: string) => s.participants.find((p) => p.id === id);
  // what an endpoint said once above the screens does is said there too, so its insides are left out here
  const visit = (items: Item[], f: (it: Item) => void) => {
    for (const it of items) {
      f(it);
      if (it.t === "call" && it.body && !ambient.has(it.to)) visit(it.body, f);
      if (it.t === "frame") visit([...it.items, ...(it.else ?? [])], f);
    }
  };
  visit(s.items, (it) => {
    if (it.t !== "call") return;
    const p = part(it.to);
    if (it.table) {
      if (it.effect === "reads") reads.add(it.table);
      else if (it.effect) changes.add(`${it.effect} ${it.table}`);
      else runs.add(`runs ${it.table}`);
    } else if (p?.kind === "endpoint" && !ambient.has(it.to)) calls.add(p.name);
    // a query whose table is only known at runtime still reaches the data
    else if (p?.kind === "database") calls.add(p.name);
    else if (p?.kind === "auth") calls.add(p.name);
    const r = refStory(it);
    const to = r && screen(r);
    if (to && r !== s.id) goes.add(to);
  });
  const out: { text: string; tone: "" | "warm" | "link" }[] = [];
  for (const c of changes) out.push({ text: c, tone: "warm" });
  for (const c of [...calls, ...runs]) out.push({ text: c, tone: "" });
  for (const g of goes) out.push({ text: `→\u00a0${g}`, tone: "link" });
  if (!changes.size && reads.size) {
    const r = [...reads];
    out.push({ text: `reads ${r.slice(0, 2).join(", ")}${r.length > 2 ? ` +${r.length - 2}` : ""}`, tone: "" });
  }
  // a few, then how many more: the panel and the diagram say the rest
  return out.length > max + 1 ? [...out.slice(0, max), { text: `+${out.length - max} more`, tone: "" }] : out;
}
