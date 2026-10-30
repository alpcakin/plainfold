// The sequence diagram of one story: what is visible (folding rules), where it goes (layout), and the SVG.
// Reading rules are in CLAUDE.md; everything here is derived from the model's structure, never from names.
import type { Sheet } from "./canvas";
import { SIGN } from "./changes";
import { segs } from "./english";
import type { CallItem, Item, Mark, Participant, ReturnItem, Said, SelfItem, Story, Was } from "./model";
import { alsoWords, refStory, touchGroups, walk } from "./model";
import { esc, fitMiddle, tw } from "./ui";

// Text sizes in the diagram; the CSS uses the same numbers (styles.css, "diagram").
const LABEL = 12, EXIT = 11, NAME = 12, SUB = 10.5, FRAME_T = 11, NUM = 11;
/** Room between a label and the lifelines on either side of it. */
const PAD = 14;
/** The widest a column's box gets; a longer name is cut in the middle and shown whole in the panel. */
const BOX_MAX = 190;
/** A story opens with each function as a column of its own when that keeps the columns at most this many. */
const MAX_COLS = 6;
/** Side work is reached by at least this many stories, and at least this share of them (see `isSide`). */
const SIDE_MIN = 3, SIDE_SHARE = 0.2;

/** Where code runs, named for the one column that stands for all of the project's functions there. */
/** `code`: the project's code where nothing shows which place runs it. */
const PLACE: Record<string, string> = { device: "App", browser: "Browser", server: "Server", code: "Code" };
/** Participants that are the project's own code; the others (packages, services, databases) are always a column each. */
const OWN = new Set<string>(["screen", "component", "endpoint", "function"]);

/** What the model's index says about a participant: how many stories and tests reach it, and which tests. */
export interface Reach {
  stories: (id: string) => number;
  tests: (id: string) => string[];
  /** stories in the project that are not tests */
  total: number;
}
const NO_REACH: Reach = { stories: () => 0, tests: () => [], total: 0 };

/** The two levels of detail: one column per place where code runs, or one per function. */
export type Level = "parts" | "functions";

/** A row or block the diagram shows (the view's own shape, built from the model). */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type N = any;

export interface Exits { callId: string; exits: ReturnItem[] }

/** A branch as a row names it: a condition by itself, a catch by the word and what it handles. */
const branchOf = (f: { kind: string; label: string }) => (f.kind === "catch" ? `catch ${f.label}`.trim() : f.label);

export class StoryView {
  S: Story;
  P: Participant[];
  PI: Record<string, Participant>;
  BY: Record<string, Item> = {};
  allCalls: CallItem[];
  open = new Set<string>();
  every = false;
  ops: N[] = [];
  /** the top of each element of the last drawing that keeps its key between drawings (`data-r`) */
  ys = new Map<string, number>();
  sel: string | null;
  reach: Reach;
  /** places (device, server…) whose functions are columns of their own; any other place is one column */
  expanded = new Set<string>();
  /** the step number shown in front of each drawn item */
  numOf = new Map<string, string>();
  /** a main return and the early exits of the same call ("· or 401, 500") */
  exitsOf = new Map<string, Exits>();
  /** the branch condition an early exit sits under */
  condOf = new Map<string, string>();
  /** exits whose condition is that a branch before them did not end the function: no frame shows it, their row does */
  afterBranch = new Set<string>();
  /** what changed since the user last looked (changes.ts): a mark per step, a changed step as it was, and the calls
   *  that hold a changed step somewhere inside */
  mark: Map<string, Mark>;
  was: Map<string, Was>;
  /** changed steps shown even where the diagram would hide them */
  named: Set<string>;
  inside = new Set<string>();
  /** the steps in plain English (english.ts), or null to show the code */
  english: ((it: Item) => Said | undefined) | null = null;
  /** the model that wrote them */
  englishBy = "";
  /** another story's name, by its id: a step that refers to a story says which */
  storyName: (id: string) => string | undefined = () => undefined;

  /** What a person would notice is shown; what only the code holds is not, unless it changed: what the screen holds
   *  on the way (`setLoading(true)`, but not what it holds as a way ends, the message before a return), a helper of
   *  the project's own that works a value out (`normalize(email) → string`), side work. Only in a story that reaches
   *  past its own code: in one that does not, those are all it does. "Every step" shows them. */
  private quietCache: boolean | undefined;
  get quiet(): boolean {
    if (this.every) return false;
    if (this.quietCache === undefined) {
      let crosses = false;
      walk(this.S.items, (i) => {
        if (i.t === "call" && (i.table || refStory(i) || (i.from !== i.to && this.PI[i.to] && this.PI[i.to].kind !== "function" && this.PI[i.to].kind !== "component"))) crosses = true;
      });
      this.quietCache = crosses;
    }
    return this.quietCache;
  }
  /** A step left out at the first level (`quiet`). */
  private hush = (it: Item) => this.quiet && !this.mark.get(it.id) && !this.named.has(it.id);

  constructor(story: Story, mark = new Map<string, Mark>(), was = new Map<string, Was>(), named = new Set<string>(), reach = NO_REACH) {
    this.S = story;
    this.mark = mark;
    this.was = was;
    this.named = named;
    this.reach = reach;
    this.P = story.participants;
    this.PI = Object.fromEntries(this.P.map((p) => [p.id, p]));
    walk(story.items, (i) => (this.BY[i.id] = i));
    // the condition each early exit sits under: the nearest branch around it in its own function. What a function it
    // calls gives back is not an exit of its own.
    const conds = (items: Item[], cond: string | null, fn: string | null) => {
      // after branches that end the function (`if (ok) return;`), what follows runs only when none of them holds
      const after: string[] = [];
      for (const it of items) {
        const c = after.length ? after.join(", ") : cond;
        if (it.t === "return" && c !== null && this.mine(it, fn)) {
          this.condOf.set(it.id, c);
          if (after.length) this.afterBranch.add(it.id);
        }
        const last = it.t === "frame" && it.kind === "alt" && !it.else ? it.items[it.items.length - 1] : undefined;
        if (it.t === "frame" && last?.t === "return" && this.mine(last, fn)) after.push(`else (${it.label})`);
        if (it.t === "call" && it.body) conds(it.body, null, it.to);
        // what runs later sits under no branch of its own
        if (it.t === "frame" && it.kind === "effect") conds(it.items, null, fn);
        else if (it.t === "frame") {
          conds(it.items, branchOf(it), fn);
          conds(it.else || [], `else (${it.label})`, fn);
        }
      }
    };
    conds(story.items, null, null);
    this.allCalls = Object.values(this.BY).filter((i): i is CallItem => i.t === "call" && !!i.body);
    // every call starts closed: a story opens as its first level only, and a call opens on click.
    // A changed step is visible from the start: the calls around it open. What an added or removed call holds stays
    // closed; its arrow already carries the mark.
    const show = (items: Item[], calls: string[]) => {
      for (const it of items) {
        const m = this.mark.get(it.id);
        if (m) {
          for (const c of calls) {
            this.open.add(c);
            this.inside.add(c);
          }
          if (m !== "changed") continue;
        }
        if (it.t === "call" && it.body) show(it.body, [...calls, it.id]);
        if (it.t === "frame") {
          show(it.items, calls);
          show(it.else || [], calls);
        }
      }
    };
    show(story.items, []);
    this.sel = (Object.values(this.BY).find((i) => i.t === "call") || story.items[0])?.id ?? null;
  }

  pname = (id: string) => this.PI[id]?.name ?? id;
  /** A return that ends the function `fn` itself (null: the story's own function, whose answer stays on its lifeline,
   *  or, in a model written before that, goes to a sender that is no column). */
  private mine = (r: ReturnItem, fn: string | null) => (fn ? r.from === fn : r.from === r.to || !this.PI[r.to]);

  /** A step in plain English: the text of the step itself (a row may stand for a copy of it), unless English is off
   *  or its words changed since the user last looked; then the code shows what changed, as git diff does. */
  saidOf(it: Item): Said | undefined {
    const o = this.BY[it.id];
    if (!o || !this.english || this.was.has(o.id) || this.mark.get(o.id) === "removed") return undefined;
    return this.english(o);
  }

  // ---------- columns ----------
  private isOwn = (p: Participant | undefined): boolean => !!p && OWN.has(p.kind) && p.group in PLACE;
  /** The column a participant is drawn in: its own, or the one for the place it runs in while that place is closed. */
  col = (id: string): string => {
    const p = this.PI[id];
    return this.isOwn(p) && !this.expanded.has(p.group) ? "g:" + p.group : id;
  };
  /** The places this story has functions in. */
  places = (): string[] => [...new Set(this.P.filter(this.isOwn).map((p) => p.group))];
  /** The level every column is at, or null when some places are open and others are not. */
  level(): Level | null {
    const all = this.places();
    if (all.length && all.every((g) => this.expanded.has(g))) return "functions";
    return this.expanded.size ? null : "parts";
  }
  setLevel(l: Level): void {
    this.expanded = new Set(l === "functions" ? this.places() : []);
  }
  /** The finest level whose columns still read: every function a column of its own, unless that makes too many. */
  autoLevel(): Level {
    this.setLevel("functions");
    return this.columns().length <= MAX_COLS ? "functions" : "parts";
  }
  /** How many columns a closed place would open into. */
  opensInto(g: string): number {
    const keep = this.expanded;
    this.expanded = new Set([...keep, g]);
    const n = this.columns().filter((c) => this.PI[c]?.group === g).length;
    this.expanded = keep;
    return n;
  }

  /** Every row as if every call and every exit were open. The columns and the room between them come from these, so
   *  opening and closing a call never moves a column. */
  private fullRows(): N[] {
    return this.flat(this.fullNodes());
  }
  /** While the layout is worked out, an open call also gives the rows it shows when closed (its return folded onto
   *  it, what a hook goes on to touch), so that closing it needs no more room than it already has. */
  private layout = false;
  private fullNodes(): N[] {
    const open = this.open, ex = new Map(this.exitsOf);
    this.open = new Set(this.allCalls.map((c) => c.id));
    this.layout = true;
    const nodes = this.vis(this.S.items, null);
    this.layout = false;
    [this.open, this.exitsOf] = [open, ex];
    return nodes;
  }
  /** Each row's number: its place in the story with every call open, as an editor that folds keeps its line numbers.
   *  A closed call is followed by a jump (`2`, then `10`) that opening it fills, and no number changes. */
  private numbering(): Map<string, number> {
    const open = this.open, ex = new Map(this.exitsOf);
    this.open = new Set(this.allCalls.map((c) => c.id));
    const rows = this.flat(this.vis(this.S.items, null));
    [this.open, this.exitsOf] = [open, ex];
    const at = new Map<string, number>();
    for (const o of rows) if (o.type !== "divider" && !o.ghostOf && !o.back && !at.has(this.key(o))) at.set(this.key(o), at.size + 1);
    return at;
  }
  private flat(ns: N[], out: N[] = []): N[] {
    for (const n of ns) {
      if (n.type === "frame") {
        this.flat(n.kids, out);
        this.flat(n.els, out);
      } else out.push(n);
    }
    return out;
  }
  private colsOf(rows: N[]): Set<string> {
    const used = new Set<string>();
    for (const r of rows) {
      if (r.type === "msg") {
        used.add(this.col(r.it.from));
        used.add(this.col(r.it.to));
      }
      if (r.type === "self") used.add(this.col(r.it.at));
    }
    return used;
  }
  /** The columns, in order: a place's columns together, places in the order the story first reaches them. */
  columns(rows = this.fullRows()): string[] {
    const used = this.colsOf(rows);
    if (!used.size && this.P[0]) used.add(this.col(this.P[0].id));
    const first = new Map<string, number>(), groupFirst = new Map<string, number>();
    this.P.forEach((p, i) => {
      const c = this.col(p.id);
      if (!first.has(c)) first.set(c, i);
      if (!groupFirst.has(p.group)) groupFirst.set(p.group, i);
    });
    const g = (c: string) => groupFirst.get(this.groupOf(c)) ?? 0;
    return [...used].sort((a, b) => g(a) - g(b) || (first.get(a) ?? 0) - (first.get(b) ?? 0));
  }
  groupOf = (c: string): string => (c.startsWith("g:") ? c.slice(2) : this.PI[c]?.group ?? "");

  // ---------- what is visible ----------

  /** How an exit is named in a tag: its status (`403`), else its code up to its arguments (`Denied`). */
  private code3 = (l: string) => (/^(\d{3}|redirect)\b/.exec(l) || [l.replace(/\(.*$/s, "")])[0];

  /** Side work: a call of the project's own function whose result the caller does not use, that reaches nothing the
   *  project owns (no table, no endpoint, no other screen), and that many stories reach. Logging and analytics are
   *  the usual case. It is one quiet row and never a column; "every step" shows it as a call. */
  isSide(c: Item): boolean {
    if (this.every || c.t !== "call" || !c.unused || !c.body || c.from === c.to || c.ends?.length) return false;
    if (this.PI[c.to]?.kind !== "function") return false;
    const n = this.reach.stories(c.to);
    if (n < SIDE_MIN || n < SIDE_SHARE * this.reach.total) return false;
    let owns = false;
    walk(c.body, (i) => {
      if (i.t === "call" && (i.table || i.effect || refStory(i) || (this.isOwn(this.PI[i.to]) && this.PI[i.to].kind !== "function"))) owns = true;
    });
    return !owns;
  }
  /** The project's own functions the story calls on its main path (not side work, nor what side work holds), each
   *  once, in the order first called. */
  mainCalls(): CallItem[] {
    const out = new Map<string, CallItem>();
    const w = (items: Item[]) => {
      for (const it of items) {
        if (it.t === "call") {
          if (this.isSide(it)) continue;
          if (this.isOwn(this.PI[it.to]) && it.from !== it.to && !out.has(it.to)) out.set(it.to, it);
          if (it.body) w(it.body);
        } else if (it.t === "frame") {
          w(it.items);
          w(it.else || []);
        }
      }
    };
    w(this.S.items);
    return [...out.values()];
  }

  /** What a closed call reaches outside the project's code, somewhere inside it: written on its row, so a call that
   *  ends at an outside service does not read like a plain helper. A call that changes data says that instead
   *  (`dataOf`); the panel lists both. */
  private outside = (c: CallItem): string => (this.dataOf(c) ? "" : this.sideTargets(c).join(", "));
  /** What a closed call changes in the data, somewhere inside it, by verb: "deletes items, tags". */
  dataOf = (c: CallItem): string =>
    touchGroups(c.touches ?? [])
      .filter(([verb]) => verb !== "reads")
      .map(([verb, names]) => `${verb} ${names.join(", ")}`)
      .join(" · ");

  /** What side work reaches outside the project's code. */
  sideTargets(c: CallItem): string[] {
    const s = new Set<string>();
    walk(c.body ?? [], (i) => {
      if (i.t === "call" && this.PI[i.to] && !OWN.has(this.PI[i.to].kind)) s.add(this.PI[i.to].name);
    });
    return [...s];
  }

  /** The callee's own returns: the main one (the last that ends it at its end) and the early exits (a rule each). */
  private returnsOf(body: Item[], callee: string) {
    let main: ReturnItem | null = null;
    const exits: ReturnItem[] = [];
    const w = (items: Item[]) => {
      for (const i of items) {
        if (i.t === "return" && i.from === callee) {
          if (i.early || i.throws) exits.push(i);
          else main = i;
        } else if (i.t === "frame" && i.kind !== "effect") {
          w(i.items);
          w(i.else || []);
        }
      }
    };
    w(body);
    return { main: main as ReturnItem | null, exits };
  }

  /** How a call with a body ends: its main return, its other exits, and, short, every way it can end besides the main
   *  return ("or 403, Denied"): its early exits and what is thrown in what it calls. A function that gives nothing
   *  back gives the caller the same when it ends early: only what it throws is named. */
  private ends(c: CallItem): { main: ReturnItem | null; exits: ReturnItem[]; tag: string } {
    const { main, exits } = this.returnsOf(c.body!, c.to);
    const named = main ? exits : exits.filter((e) => e.throws);
    // a rethrow is no new way to end: it passes on what it caught
    const all = [...named.flatMap((e) => e.passes ?? [e.label]), ...(c.ends ?? [])].map(this.code3);
    return { main, exits, tag: all.length ? "or " + [...new Set(all)].join(", ") : "" };
  }

  /** Where the story's work can stop, grouped by how it ends (`401`, `Denied`), in the order written, each once (a
   *  function called twice has its rules once). A throw stops the work up to whatever catches it; a rethrow only passes
   *  one on. An early return stops only its own function, so it counts where it ends the work of a place: in the
   *  story's own function, or one another place calls (an endpoint's answer). Other early returns are rows of the
   *  calls they are in. */
  rules(): { end: string; throws: boolean; rules: ReturnItem[] }[] {
    const out: { end: string; throws: boolean; rules: ReturnItem[] }[] = [];
    const seen = new Set<string>();
    walk(this.S.items, (i) => {
      if (i.t !== "return" || i.passes) return;
      const place = this.mine(i, null) || this.PI[i.from]?.group !== this.PI[i.to]?.group;
      if (!i.throws && !(i.early && place)) return;
      const k = `${i.ref.file}:${i.ref.from}`;
      if (seen.has(k)) return;
      seen.add(k);
      const end = this.code3(i.label);
      const g = out.find((x) => x.end === end && x.throws === !!i.throws);
      if (g) g.rules.push(i);
      else out.push({ end, throws: !!i.throws, rules: [i] });
    });
    return out;
  }
  /** What a rule says besides how it ends (`"Not found"` of `404 "Not found"`). */
  ruleSays = (r: ReturnItem) => r.label.slice(this.code3(r.label).length).trim();

  // A call to a plain function in the same process that touches nothing outside (no body, no effect), together with
  // its return, folds into one row on the caller: "name(args) → result". Only when everything between the two belongs
  // to the callee.
  private ownedBy = (i: Item, id: string): boolean =>
    i.t === "self" ? i.at === id
    : i.t === "return" ? i.from === id
    : i.t === "call" ? i.from === id
    : i.items.every((x) => this.ownedBy(x, id)) && (i.else || []).every((x) => this.ownedBy(x, id));

  private plainCall(items: Item[], idx: number) {
    const c = items[idx];
    if (this.every || c.t !== "call" || c.body || c.effect || c.table || c.from === c.to) return null;
    const to = this.PI[c.to], from = this.PI[c.from];
    if (!to || !from || to.kind !== "function" || to.group !== from.group) return null;
    for (let j = idx + 1; j < items.length; j++) {
      const x = items[j];
      if (x.t === "return" && x.from === c.to && x.to === c.from) return { ret: x, next: j };
      if (!this.ownedBy(x, c.to)) return null;
    }
    return null;
  }

  private vis(items: Item[], scope: N, depth = 0, par: string | null = null): N[] {
    const out: N[] = [];
    for (let idx = 0; idx < items.length; idx++) {
      const it = items[idx];
      const pc = this.plainCall(items, idx);
      if (pc && it.t === "call") {
        if (this.hush(it) && !it.changes?.length) {
          idx = pc.next;
          continue;
        }
        out.push({ type: "self", it: { t: "self", id: it.id, at: it.from, kind: "pure", label: it.label, ref: it.ref }, merged: [it], text: `${it.label} → ${pc.ret.label}`, gives: pc.ret.label, changes: it.changes });
        idx = pc.next;
        continue;
      }
      if (it.t === "self") {
        if (!this.every && !this.named.has(it.id) && (it.kind === "local" || (it.kind === "state" && (it.label.startsWith("useState:") || (scope && scope.lane))))) continue;
        // what the screen holds as a way ends (the message it shows before it stops) is kept
        const ends = idx === items.length - 1 || items[idx + 1]?.t === "return";
        if (it.kind === "state" && !ends && this.hush(it)) continue;
        const prev = out[out.length - 1];
        // what changes, one after another, is one row: what the screen holds, or what the function changes
        if (!this.every && (it.kind === "state" || it.kind === "changes") && prev && prev.type === "self" && prev.it.kind === it.kind && !prev.cond && this.mark.get(prev.it.id) === this.mark.get(it.id)) {
          prev.merged.push(it);
          continue;
        }
        // the trigger names the other ways the same story starts
        const also = it.kind === "trigger" && !scope && this.S.also?.length ? alsoWords(this.S).join(" · ") : undefined;
        out.push({ type: "self", it, merged: [it], also });
      } else if (it.t === "call" && this.isSide(it)) {
        if (this.hush(it)) continue;
        out.push({ type: "self", it: { t: "self", id: it.id, at: it.from, kind: "side", label: it.label, ref: it.ref }, merged: [it], side: this.sideTargets(it) });
      } else if (it.t === "call" && refStory(it) && !it.body) {
        // another story starts here, or is handed work here, told there: a row on this lifeline that goes to it, not a
        // column of its own (`opens`: that story's id)
        out.push({ type: "self", it: { t: "self", id: it.id, at: it.from, kind: "ref", label: it.label, ref: it.ref }, merged: [it], opens: refStory(it) });
      } else if (it.t === "call" && !it.body) {
        // calls to the same place, one after another with nothing between and nothing kept, read as one row
        const prev = out[out.length - 1], same = (a: string, b: string) => this.col(a) === this.col(b);
        if (!this.every && prev && prev.type === "msg" && prev.it.t === "call" && !prev.it.body && !prev.cond && !prev.ghostOf && prev.it.unused && !prev.it.effect && !it.effect &&
          same(prev.it.from, it.from) && same(prev.it.to, it.to) && !same(it.from, it.to) && this.mark.get(prev.it.id) === this.mark.get(it.id) && !this.was.has(it.id) && !this.was.has(prev.it.id)) {
          prev.merged.push(it);
          // the receiver they share is written once: "session.add(item) · .commit() · .refresh(item)"
          const at = (l: string) => l.slice(0, l.indexOf("(") < 0 ? 0 : l.lastIndexOf(".", l.indexOf("(")) + 1);
          const first = at(prev.merged[0].label);
          prev.text = prev.merged.map((c: CallItem, i: number) => (i && first && at(c.label) === first ? "." + this.shortCall(c.label).slice(first.length) : this.shortCall(c.label))).join(" · ");
          continue;
        }
        out.push({ type: "msg", it, merged: [it], fold: "" });
        // what the caller keeps comes back: a return arrow, unless the model draws one itself
        const next = items[idx + 1];
        if (!it.unused && !refStory(it) && !same(it.from, it.to) && !(next && next.t === "return" && next.from === it.to && next.to === it.from)) {
          out.push({ type: "msg", it: { t: "return", id: it.id + ":back", from: it.to, to: it.from, label: it.into ?? "", ref: it.ref }, back: it.id });
        }
      } else if (it.t === "call") {
        const same = this.col(it.from) === this.col(it.to);
        const open = this.open.has(it.id);
        const row: N = { type: "msg", it, fold: open ? "▾" : "▸" };
        out.push(row);
        const r = this.ends(it);
        const lane = it.from === it.to;
        if (r.exits.length) this.exitsOf.set(it.id, { callId: it.id, exits: r.exits });
        // a call that stays in its column has no return arrow: what it gives back rides on the call itself, and,
        // closed, how else it can end
        if (same && !lane) {
          if (r.main) row.ret = r.main.label;
          if (!open) row.tag = r.tag;
        }
        // closed, it says what it changes in the data and what it reaches outside the project's code
        if (!lane) {
          row.data = this.dataOf(it);
          row.reach = this.outside(it);
        }
        if (open) {
          out.push(...this.vis(it.body!, { callId: it.id, fn: it.to, lane: lane || (scope && scope.lane), same, main: r.main }, depth + 1, it.id));
          if (this.layout) out.push(...this.closedForm(it, lane, same, row));
        } else if (lane) {
          // closed: one arrow from the screen to what the hook goes on to touch ("reads items, books")
          const targets = new Set<string>();
          walk(it.body!, (i) => {
            if (i.t === "call" && i.table) targets.add(i.to);
          });
          if (targets.size === 1) {
            const sum = touchGroups(it.touches || []).map(([e, n]) => `${e} ${n.join(", ")}`).join(" · ");
            out[out.length - 1] = { type: "msg", it: { ...it, to: [...targets][0], label: it.label.replace(/\(.*$/, "") }, sum, fold: "▸" };
          } else out[out.length - 1].tag = (it.touches || []).join(", ");
        } else if (same) {
          // nothing
        } else if (r.main) {
          // closed: what it gives back, and how else it can end ("· or 403, 404")
          out.push({ type: "msg", it: r.main, synth: it, tag: r.tag });
        } else row.tag = r.tag;
      } else if (it.t === "return") {
        // an early exit ends its function here: a rule. What a function it calls gives back is not one.
        const exit = this.mine(it, scope ? scope.fn : null) && !!(it.early || it.throws);
        const cond = exit && this.afterBranch.has(it.id) ? this.condOf.get(it.id) : undefined;
        // a model written before answers stayed on their lifeline names the request's sender, which is no column
        const away = !this.PI[it.to];
        if (this.col(it.from) === this.col(it.to) || away) {
          // a return to the same column draws no arrow. An early exit stays on its lifeline, as a row that stops there;
          // so does what the story's own function answers, at the top, as a row that leaves it.
          if (exit) out.push({ type: "self", it: { t: "self", id: it.id, at: it.from, kind: "exit", label: it.label, ref: it.ref }, merged: [it], exit: true, throws: !!it.throws, cond });
          else if (!scope && (it.from === it.to || away)) out.push({ type: "self", it: { t: "self", id: it.id, at: it.from, kind: "reply", label: it.label, ref: it.ref }, merged: [it], exit: true, reply: true });
          continue;
        }
        out.push({ type: "msg", it, exit: exit && !(scope && it === scope.main), throws: !!it.throws, cond });
      } else if (it.kind === "effect") {
        // what runs later: a thin divider over its rows, from the column they run in, not a box
        const kids = this.vis(it.items, scope, depth, par);
        out.push({ type: "divider", it, rows: kids, home: this.homeOf(kids) ?? scope?.fn }, ...kids);
      } else {
        const kids = this.vis(it.items, scope, depth, par), els = it.else ? this.vis(it.else, scope, depth, par) : [];
        // a branch with a single visible step is one row with its condition in front, not a box
        const one = (it.kind === "alt" || it.kind === "catch") && !els.length && kids.length && kids[0].type !== "frame" && kids[0].type !== "divider" && !kids[0].cond;
        if (one && kids.length === 1) out.push({ ...kids[0], cond: branchOf(it), frame: it });
        else if (kids.length || els.length) {
          out.push({ type: "frame", it, kids, els });
          // the layout also takes the room of the one row the branch becomes when a call in it is closed
          if (this.layout && one) out.push({ ...kids[0], cond: branchOf(it), frame: it });
        }
      }
    }
    // every row knows which open call it sits under (par) and how deep (depth); rows spliced in from a deeper call are already tagged
    for (const n of out) if (n.depth === undefined) { n.depth = depth; n.par = par; }
    return out.flatMap((n) => {
      const g = this.ghost(n);
      return g ? [g, n] : [n];
    });
  }

  /** The column the first of these rows runs in. */
  private homeOf(ns: N[]): string | undefined {
    for (const n of ns) {
      if (n.type === "self") return n.it.at;
      if (n.type === "msg") return n.it.from;
      if (n.type === "frame") {
        const h = this.homeOf([...n.kids, ...n.els]);
        if (h) return h;
      }
    }
    return undefined;
  }
  /** A divider says when what follows runs ("after first render"), as the core words it; the code is in the panel. */
  private divText = (o: N) => {
    const t: string = o.it.note ?? o.it.label;
    return t.length > 64 ? t.slice(0, 63) + "…" : t;
  };

  /** The rows a call with a body shows when it is closed, besides its own arrow: for the layout. */
  private closedForm(it: CallItem, lane: boolean, same: boolean, row?: N): N[] {
    const r = this.ends(it);
    // the call's own row as it reads closed, with what it reaches outside and how else it can end
    const own: N[] = row?.reach || row?.data || r.tag ? [{ ...row, fold: "▸", depth: 0, tag: r.main && !same ? row?.tag : r.tag }] : [];
    if (lane) {
      const targets = new Set<string>();
      walk(it.body!, (i) => {
        if (i.t === "call" && i.table) targets.add(i.to);
      });
      if (targets.size !== 1) return [];
      return [{ type: "msg", it: { ...it, to: [...targets][0], label: it.label.replace(/\(.*$/, "") }, sum: (it.touches || []).join(" · "), fold: "▸", depth: 0 }];
    }
    if (same || !r.main) return own;
    return [...own, { type: "msg", it: r.main, synth: it, tag: r.tag, depth: 0 }];
  }

  /** A changed row whose words changed, as it was: drawn right above it like a removed line in git diff. It has no
   *  number of its own and selects the row it was. */
  private ghost(n: N): N | null {
    // rows spliced in from a deeper call already have theirs
    if ((n.type !== "msg" && n.type !== "self") || n.ghostOf || n.ghosted || n.back) return null;
    // only when the row would read differently: an arrow elides long arguments, and the panel shows both whole
    const shown = (l: string) => (n.it.t === "call" && !this.isSelfCall(n) ? this.shortCall(l) : l);
    const w = this.was.get(n.it.id), f = n.frame && this.was.get(n.frame.id);
    const label = w && shown(w.label) !== shown(n.it.label) ? w.label : null, cond = f && f.label !== n.frame.label ? branchOf({ kind: n.frame.kind, label: f.label }) : null;
    if (!label && !cond) return null;
    n.ghosted = true;
    const it = { ...n.it, id: "was:" + n.it.id, label: label ?? n.it.label };
    return {
      ...n, it, ghostOf: n.it.id, merged: [it], cond: cond ?? n.cond, fold: "", tag: "", sum: undefined, synth: undefined, ret: undefined,
      text: n.text && label ? n.text.replace(n.it.label, label) : n.text,
    };
  }

  // ---------- labels ----------
  /** How a drawn row changed: its own step, the branch folded into it, or a step merged into it. */
  rowMark = (n: N): Mark | undefined =>
    n.ghostOf ? "removed" : this.mark.get(n.it.id) ?? (n.frame && this.mark.get(n.frame.id)) ?? n.merged?.map((i: Item) => this.mark.get(i.id)).find(Boolean);
  /** The condition a changed branch had before, struck after the new one in the frame's title. */
  private wasText(n: N): string {
    const f = n.type === "frame" && this.was.get(n.it.id);
    return f && f.label !== n.it.label ? `  ${f.label}` : "";
  }
  /** The condition in front of a row that is a branch of its own, cut past 30 characters; the panel shows it whole. */
  private condText = (n: N) => {
    if (!n.cond) return "";
    const c = (n.frame && this.saidOf(n.frame)?.text) || n.cond;
    return `[${c.length > 30 ? c.slice(0, 29) + "…" : c}] `;
  };
  /** A row's words in plain English, as segments, when every step it stands for has them; a row that stands for
   *  several steps joins them as the code does. */
  private rowSaid(n: N): [string, string][] | null {
    if (!this.english || n.ghostOf || n.back || n.synth || n.exit || (n.type === "msg" && n.it.t === "return")) return null;
    const items: Item[] = n.merged?.length ? n.merged : [n.it];
    const all = items.map((i) => this.saidOf(i));
    if (!all.length || all.some((s) => !s)) return null;
    const out: [string, string][] = [];
    all.forEach((s, k) => {
      if (k) out.push([" · ", ""]);
      out.push(...segs(s!, items.length > 1 ? 28 : 44));
    });
    if (n.gives !== undefined) out.push([` → ${n.gives}`, "tag"]);
    return out;
  }
  /** A frame's keyword and title: with English on, "if" and "for" and the condition in plain words when there is a
   *  text for it. */
  private frameParts(o: N): { kw: string; words: [string, string][] } {
    const k = o.it.kind;
    const s = this.saidOf(o.it);
    // a catch is "if" only with words that could follow it
    const plain = !!this.english && (k === "loop" || (k === "alt" && !o.it.note) || (k === "catch" && !!s));
    return { kw: plain ? (k === "loop" ? "for" : "if") : k, words: s ? segs(s, 60) : [[o.it.label, ""]] };
  }
  private frameTitle = (o: N) => {
    const f = this.frameParts(o);
    return `${f.kw} ${f.words.map((w) => w[0]).join("")}`.trim();
  };
  private isSelfCall = (n: N) => n.type === "msg" && n.it.t === "call" && this.col(n.it.from) === this.col(n.it.to);
  private shortArgs = (l: string) => l.replace(/\((.{11,})\)$/, "(…)");
  /** On an arrow, a long argument list at the end of a call is elided; the details panel shows it whole. A label the
   *  core cut short ("…") inside an argument list counts as one. */
  private shortCall(l: string): string {
    if (l.length > 32 && l.endsWith("…")) {
      const open: number[] = [];
      for (let i = 0; i < l.length; i++) {
        if (l[i] === "(") open.push(i);
        else if (l[i] === ")") open.pop();
      }
      if (open.length && l.length - open[0] - 2 > 10) return l.slice(0, open[0]) + "(…)";
    }
    if (l.length <= 32 || !l.endsWith(")")) return l;
    let depth = 0;
    for (let i = l.length - 1; i >= 0; i--) {
      if (l[i] === ")") depth++;
      else if (l[i] === "(" && --depth === 0) return l.length - i - 2 > 10 ? l.slice(0, i) + "(…)" : l;
    }
    return l;
  }
  /** A step that stays in its column, past 44 characters, keeps its name and loses its arguments; the panel shows it
   *  whole. */
  private cap = (l: string) => (l.length > 44 ? (this.shortCall(l).length <= 44 ? this.shortCall(l) : l.slice(0, 43) + "…") : l);
  private selfLabel = (n: N) => {
    const said = this.rowSaid(n);
    if (said) return this.condText(n) + said.map((s) => s[0]).join("") + (n.opens ? this.refText(n.opens) : "");
    return this.condText(n) + (n.text ? n.text : n.merged.length > 1 ? n.merged.map((i: SelfItem) => this.shortArgs(i.label)).join(" · ") : (n.it.kind === "render" ? "render " : "") + this.cap(n.it.label)) + this.changesText(n);
  };
  /** What a step folded into one row changes in what it is given. */
  private changesText = (n: N) => (n.changes?.length ? ` · ${this.touchText("changes " + n.changes.join(", "))}` : "") + (n.opens ? this.refText(n.opens) : "");
  /** A step that refers to another story names it after its mark, cut past 40 characters. */
  private refText = (id: string) => {
    const name = this.storyName(id);
    return " ↗" + (name ? " " + this.touchText(name) : "");
  };
  private alsoText = (n: N) => (n.also ? ` · ${this.touchText(n.also)}` : "");
  private sideText = (n: N) => (n.side?.length ? `  → ${n.side[0]}${n.side.length > 1 ? ` +${n.side.length - 1}` : ""}` : "");
  /** What a closed call goes on to touch, cut past 40 characters; the panel lists it whole. */
  private touchText = (t: string) => (t.length > 40 ? t.slice(0, 39) + "…" : t);
  private msgSegs(n: N): [string, string][] {
    const out: [string, string][] = [];
    if (n.cond) out.push([this.condText(n), "cond"]);
    const said = this.rowSaid(n);
    if (said) out.push(...said);
    else out.push([n.text ?? (n.it.t === "call" ? (this.isSelfCall(n) ? this.cap(n.it.label) : this.shortCall(n.it.label)) : n.it.label), ""]);
    if (n.ret) out.push([` → ${n.ret}`, "tag"]);
    if (n.sum) out.push([` · ${this.touchText(n.sum)}`, "tag"]);
    if (n.it.effect && n.it.effect !== "reads") out.push([`  ${n.it.effect}`, "rk"]);
    if (n.it.t === "call" && n.it.changes?.length && !n.synth) out.push([` · ${this.touchText("changes " + n.it.changes.join(", "))}`, "rk"]);
    if (n.data && n.fold === "▸") out.push([` · ${this.touchText(n.data)}`, "rk"]);
    if (n.tag) out.push([` · ${this.touchText(n.tag)}`, "tag"]);
    if (n.reach && n.fold === "▸") out.push([` · ${this.touchText(n.reach)}`, "out"]);
    if (n.it.t === "call" && n.it.touches && n.fold === "▾" && n.it.from === n.it.to) out.push([` · ${this.touchText(n.it.touches.join(", "))}`, "tag"]);
    if (n.synth) out.push([" ▸", "fold"]);
    else if (n.fold) out.push([` ${n.fold}`, n.fold === "▾" ? "fold on" : "fold"]);
    if (n.fold === "▸" && this.inside.has(n.it.id)) out.push([" ●", "din"]);
    return out;
  }
  private msgText = (n: N) => this.msgSegs(n).map((s) => s[0]).join("");
  /** The width a row's words take next to its column (a step that stays) or on its arrow. */
  private rowWidth = (n: N) =>
    n.type === "self" ? tw(this.selfLabel(n) + this.alsoText(n) + this.sideText(n), n.it.kind === "side" || n.exit ? EXIT : LABEL)
    : tw(this.msgText(n), n.exit || n.back ? EXIT : LABEL);
  /** Which part of the row the key of its label is: a row is drawn once, but the same step can be an arrow and its
   *  return. */
  private key = (n: N) => n.it.id + (n.back ? ":back" : n.exit ? ":exit" : "");

  /** The call a row is about, for the test mark in front of it: a call of the project's own function. */
  private testCall(o: N): CallItem | null {
    const c = o.type === "msg" && o.it.t === "call" ? o.it : o.type === "self" && !o.side && o.merged?.[0]?.t === "call" ? o.merged[0] : null;
    return c && OWN.has(this.PI[c.to]?.kind ?? "") && c.from !== c.to ? c : null;
  }

  /** The first drawn step that changed since the user last looked: the view starts there. */
  firstMark(): string | null {
    if (!this.mark.size) return null;
    const o = this.ops.find((o) => o.it && o.type !== "frame" && o.type !== "divider" && this.rowMark(o));
    return o ? o.it.id : null;
  }

  // ---------- draw ----------
  render(): Sheet {
    const sel = this.sel;
    // Layout: the columns are worked out from every row as if every call were open, so none appears or goes while
    // reading; the room between them fits only the rows shown, so a closed call's insides take none. No label may
    // cross a lifeline other than its own.
    const fullN = this.fullNodes(), full = this.flat(fullN);
    this.exitsOf.clear();
    const nodes = this.vis(this.S.items, null);
    const rows = this.flat(nodes);
    let depthMax = 0;
    const deepest = (ns: N[], d: number) => {
      for (const n of ns) {
        depthMax = Math.max(depthMax, d);
        if (n.type === "frame") {
          deepest(n.kids, d + 1);
          deepest(n.els, d + 1);
        }
      }
    };
    deepest(fullN, 0);
    const cols = this.columns(full);
    const shown = this.colsOf(rows);
    const ix: Record<string, number> = Object.fromEntries(cols.map((c, i) => [c, i]));
    const isPlace = (c: string) => c.startsWith("g:");
    const nameOf = (c: string) => (isPlace(c) ? PLACE[c.slice(2)] : this.pname(c));
    const opens = new Map<string, number>();
    for (const c of cols) if (isPlace(c)) opens.set(c, this.opensInto(c.slice(2)));
    const subOf = (c: string) => {
      if (isPlace(c)) return c.slice(2);
      const p = this.PI[c];
      return p.sub && p.sub.length > 26 ? p.sub.slice(0, 25) + "…" : p.sub || p.kind;
    };
    const more = (c: string) => (opens.get(c) ?? 0) > 1 ? `▸ ${opens.get(c)} functions` : "";
    const boxName = (c: string) => fitMiddle(nameOf(c), NAME, BOX_MAX - 24);
    const bw = (c: string) => Math.min(BOX_MAX, Math.max(tw(boxName(c), NAME), tw(subOf(c), SUB), tw(more(c), SUB)) + 24);
    const boxH = cols.some((c) => more(c)) ? 58 : 44;
    const TOP = 30 + boxH + 24;

    // the room between neighbouring columns: enough for their boxes, for the words of every step that stays in the
    // left one, and for every arrow's label, which sits in the widest gap its arrow crosses
    const n = cols.length;
    const gap = cols.slice(1).map((c, k) => Math.max(110, (bw(cols[k]) + bw(c)) / 2 + 20));
    let tail = bw(cols[n - 1]) / 2 + 12;
    const room = (k: number, w: number) => {
      if (k < n - 1) gap[k] = Math.max(gap[k], w);
      else tail = Math.max(tail, w);
    };
    const seg = new Map<string, number>();
    const constrain = (list: N[]) => {
      const labels: [number, number, number, string][] = [];
      for (const r of list) {
        // a divider's words start at the column its rows run in and must not reach the next one
        if (r.type === "divider") {
          if (r.home) room(ix[this.col(r.home)], tw(this.divText(r), EXIT) + PAD);
          continue;
        }
        if (r.type === "self" || this.isSelfCall(r)) room(ix[this.col(r.type === "self" ? r.it.at : r.it.from)], 32 + this.rowWidth(r) + PAD);
        else if (r.type === "msg" && !seg.has(this.key(r))) {
          const a = ix[this.col(r.it.from)], b = ix[this.col(r.it.to)];
          labels.push([Math.min(a, b), Math.max(a, b), this.rowWidth(r), this.key(r)]);
        }
      }
      labels.sort((x, y) => x[1] - x[0] - (y[1] - y[0]));
      for (const [lo, hi, w, key] of labels) {
        let k = lo;
        for (let j = lo; j < hi; j++) if (gap[j] > gap[k]) k = j;
        if (k < n - 1) gap[k] = Math.max(gap[k], w + 2 * PAD);
        seg.set(key, k);
      }
    };
    constrain(rows);
    // the left gutter: a column for the change signs, then each row's test mark and its number, right-aligned. It is
    // drawn on its own, as it stays at the left edge when the diagram is moved across.
    const NUM0 = this.mark.size ? 18 : 6;
    const numAt = this.numbering();
    const numEnd = NUM0 + 10 + tw(String(numAt.size), NUM), GUT = numEnd + 6;
    const NUMX = Math.max(52, numEnd);
    const X: Record<string, number> = {};
    let x = Math.max(NUMX + 6 + bw(cols[0]) / 2, NUMX + 6 + 14 * (depthMax + 1));
    cols.forEach((c, i) => {
      if (i) x += gap[i - 1];
      X[c] = x;
    });
    const xOf = (id: string) => X[this.col(id)];
    /** where an arrow's label is centred: the middle of its gap */
    const mid = (o: N) => {
      const k = seg.get(this.key(o));
      const a = xOf(o.it.from), b = xOf(o.it.to);
      return k !== undefined && cols[k + 1] ? (X[cols[k]] + X[cols[k + 1]]) / 2 : (a + b) / 2;
    };

    const ext = (o: N): [number, number] => {
      if (o.type === "self") { const x0 = xOf(o.it.at); return [x0 - 4, x0 + 34 + this.rowWidth(o)]; }
      if (this.isSelfCall(o)) { const x0 = xOf(o.it.from); return [x0 - 4, x0 + 34 + this.rowWidth(o)]; }
      if (o.type === "msg") {
        const a = xOf(o.it.from), b = xOf(o.it.to), m = mid(o), w = this.rowWidth(o);
        return [Math.min(a, b, m - w / 2), Math.max(a, b, m + w / 2)];
      }
      if (o.type === "divider") {
        const x0 = o.home ? xOf(o.home) : X[cols[0]];
        let l = x0 - 4, r = l + tw(this.divText(o), EXIT);
        for (const c of o.rows) if (c.type !== "divider") { const [a, b] = ext(c); l = Math.min(l, a); r = Math.max(r, b); }
        return [l, r];
      }
      let l = Infinity, r = -Infinity;
      for (const c of [...o.kids, ...o.els]) { const [a, b] = ext(c); l = Math.min(l, a); r = Math.max(r, b); }
      l -= 12; r += 12;
      return [l, Math.max(r, l + tw(`${this.frameTitle(o)}${this.wasText(o)}`, FRAME_T) + 20)];
    };

    const ops: N[] = [];
    this.ops = ops;
    this.numOf.clear();
    let y = TOP;
    const rowH = (o: N) => (o.type === "self" ? (o.side || o.exit ? 26 : 30) : o.exit || o.back ? 26 : 34);
    const place = (ns: N[]) => {
      for (const o of ns) {
        if (o.type === "frame") {
          const [l, r] = ext(o);
          const op = { ...o, l, r, y0: y };
          ops.push(op);
          y += 24;
          place(o.kids);
          if (o.els.length) { op.elseY = y; y += 22; place(o.els); }
          op.y1 = y + 2;
          y += 10;
        } else if (o.type === "divider") { const [dl, dr] = ext(o); ops.push({ ...o, y, dl, dr }); y += 30; }
        else if (o.ghostOf || o.back) { ops.push({ ...o, y, nl: "" }); y += rowH(o); }
        else {
          const nl = String(numAt.get(this.key(o)) ?? "");
          this.numOf.set(o.it.id, nl);
          if (o.synth) this.numOf.set(o.synth.id, nl);
          ops.push({ ...o, y, nl });
          y += rowH(o);
        }
      }
    };
    place(nodes);
    const H = y + 8;
    let W = X[cols[n - 1]] + tail;
    for (const o of ops) W = Math.max(W, (o.type === "frame" ? o.r : o.type === "divider" ? o.dr : ext(o)[1]) + 12);

    const markOf = (o: N): Mark | undefined => (o.type === "frame" || o.type === "divider" ? this.mark.get(o.it.id) : this.rowMark(o));
    // what keeps its place between drawings has a key, and its top is kept, so a row that moved can slide
    const ys = new Map<string, number>();
    this.ys = ys;
    const keep = (k: string, y: number, body: string) => {
      ys.set(k, y);
      return `<g data-r="${esc(k)}">${body}</g>`;
    };
    const rk = (o: N) => `${o.type}:${o.synth ? "s:" : ""}${this.key(o)}`;
    const dcls = (o: N) => (markOf(o) ? ` d-${markOf(o)}` : "");
    const sign = (o: N) => (markOf(o) ? `<tspan class="dsg ${markOf(o)}">${SIGN[markOf(o)!]} </tspan>` : "");
    const wasSpan = (o: N) => (this.wasText(o) ? `<tspan class="was">${esc(this.wasText(o))}</tspan>` : "");
    // the old words are struck by a line of their own: a text decoration would be painted over the glyphs with the halo
    const strike = (x: number, baseline: number, before: string, size: number, was: string) => {
      if (!was) return "";
      const x0 = x + tw(before, size) + tw(was.match(/^\s*/)![0], size), x1 = x + tw(before + was, size), y0 = baseline - size * 0.3;
      return `<line class="strike" x1="${x0}" y1="${y0}" x2="${x1}" y2="${y0}"/>`;
    };
    // a row's number, and the change sign in its own column at the left edge; what a closed call holds is told by the
    // jump in the numbers after it, never by a color behind it
    const num = (o: N, ty: number) =>
      (markOf(o) ? `<text class="num" x="5" y="${ty}">${sign(o)}</text>` : "") +
      (o.nl ? `<text class="num${o.par ? " sub" : ""}" x="${numEnd}" y="${ty}" text-anchor="end">${o.nl}</text>` : "");
    // whether tests reach the function a row calls: a mark just before its number
    const testMark = (o: N, ty: number) => {
      const c = this.testCall(o);
      if (!c || o.ghostOf || !o.nl) return "";
      const t = this.reach.tests(c.to).length;
      return `<circle class="tdot${t ? " on" : ""}" cx="${NUM0 + 3}" cy="${ty - 4}" r="3"><title>${t ? `${t} test${t > 1 ? "s" : ""} reach ${esc(this.pname(c.to))}` : `No test reaches ${esc(this.pname(c.to))}`}</title></circle>`;
    };
    // a call that opens has a small button over the mark at the end of its words: soft while closed, filled when open
    const chip = (o: N, x: number, middle: boolean, ty: number, size: number) => {
      const segs = this.msgSegs(o), i = segs.findIndex((g) => g[1].startsWith("fold"));
      if (i < 0) return "";
      const before = segs.slice(0, i).map((g) => g[0]).join("") + " ";
      const x0 = (middle ? x - tw(segs.map((g) => g[0]).join(""), size) / 2 : x) + tw(before, size), w = tw(segs[i][0].trim(), size);
      return `<rect class="fbtn${segs[i][1] === "fold on" ? " on" : ""}" x="${x0 - 4}" y="${ty - size + 1}" width="${w + 8}" height="${size + 3}" rx="4"/>`;
    };
    let svg = "", gut = "";
    // the numbers go on a layer of their own (see `num`), each kept with its row
    const pin = (o: N, y: number, body: string) => {
      if (body) gut += keep(rk(o), y, body);
    };
    // what changed since the user last looked: a band behind each changed row, as git diff tints a line; rows that
    // follow each other make one band
    for (const o of ops) {
      const m = markOf(o);
      if (!m) continue;
      const [y0, h] = o.type === "frame" ? [o.y0, 24] : [o.y, o.type === "divider" ? 30 : rowH(o)];
      svg += keep(`db:${rk(o)}`, y0, `<rect class="db ${m}" x="0" y="${y0}" width="${W}" height="${h}"/>`);
      // the band goes on under the numbers while they stand at the left edge
      gut += keep(`db:${rk(o)}`, y0, `<rect class="gdb ${m}" x="0" y="${y0}" width="${GUT}" height="${h}"/>`);
    }
    // the column boxes and the places above them are also drawn on their own, for the top of the viewport
    let head = "";
    // the place a run of columns is in; an opened place can be closed from there
    for (let i = 0; i < n;) {
      const g = this.groupOf(cols[i]);
      let j = i;
      while (j + 1 < n && this.groupOf(cols[j + 1]) === g) j++;
      const l = X[cols[i]] - bw(cols[i]) / 2, r = X[cols[j]] + bw(cols[j]) / 2;
      // an opened place can be shown as one column again, when it has more than one
      const closable = this.expanded.has(g) && g in PLACE && j > i;
      head += closable
        ? `<g class="grp on" data-g="${esc(g)}" tabindex="0" role="button" aria-label="Show ${esc(PLACE[g])} as one column"><text x="${l}" y="12">${esc(g)} <tspan class="gx">▾ one column</tspan></text><line x1="${l}" y1="20" x2="${r}" y2="20"/></g>`
        : `<g class="grp"><text x="${l}" y="12">${esc(g)}</text><line x1="${l}" y1="20" x2="${r}" y2="20"/></g>`;
      i = j + 1;
    }
    svg += head;
    ops.forEach((o, k) => {
      if (o.type !== "frame") return;
      const fp = this.frameParts(o);
      pin(o, o.y0, num(o, o.y0 + 15));
      svg += keep(rk(o), o.y0,
        `<g class="fr${dcls(o)}${sel === o.it.id ? " sel" : ""}" data-k="${k}" tabindex="0" role="button" aria-label="${esc(this.frameTitle(o))}">` +
        `<rect class="bx" x="${o.l}" y="${o.y0}" width="${o.r - o.l}" height="${o.y1 - o.y0}" rx="5"/>` +
        `<text x="${o.l + 8}" y="${o.y0 + 15}"><tspan class="kw">${esc(fp.kw)}</tspan> <tspan class="lbl">${fp.words.map(([t, c]) => (c ? `<tspan class="${c}">${esc(t)}</tspan>` : esc(t))).join("")}</tspan>${wasSpan(o)}</text>` +
        strike(o.l + 8, o.y0 + 15, this.frameTitle(o), FRAME_T, this.wasText(o)) +
        (o.elseY ? `<line class="sep" x1="${o.l}" y1="${o.elseY + 4}" x2="${o.r}" y2="${o.elseY + 4}"/><text x="${o.l + 8}" y="${o.elseY + 17}">[else]</text>` : "") + `</g>`);
    });
    const sideOf = new Set<string>();
    const s = sel ? this.BY[sel] : null;
    if (s && (s.t === "call" || s.t === "return")) { sideOf.add(this.col(s.from)); sideOf.add(this.col(s.to)); }
    if (s && s.t === "self") sideOf.add(this.col(s.at));
    const headH = 30 + boxH + 10;
    for (const c of cols) {
      const x0 = X[c], w = bw(c), m = more(c);
      // a column nothing shown reaches is inside a closed call: it stays where it is, quieter
      const on = (sel === "p:" + c ? " sel" : sideOf.has(c) ? " end" : "") + (shown.has(c) ? "" : " idle") + (isPlace(c) ? " place" : "");
      const life = (y2: number) => `<line class="life${shown.has(c) ? "" : " idle"}" x1="${x0}" y1="${30 + boxH + 2}" x2="${x0}" y2="${y2}"/>`;
      const box =
        `<g class="pt${on}" data-p="${esc(c)}" tabindex="0" role="button" aria-label="${esc(nameOf(c))}${m ? ": open into its functions" : ""}"><rect class="bx" x="${x0 - w / 2}" y="30" width="${w}" height="${boxH}" rx="7"/>` +
        (boxName(c) !== nameOf(c) ? `<title>${esc(nameOf(c))}</title>` : "") +
        `<text class="lb" x="${x0}" y="49" text-anchor="middle">${esc(boxName(c))}</text><text class="sb" x="${x0}" y="64" text-anchor="middle">${esc(subOf(c))}</text>` +
        (m ? `<text class="xp" x="${x0}" y="80" text-anchor="middle">${esc(m)}</text>` : "") + `</g>`;
      svg += life(H) + box;
      head += life(headH) + box;
    }
    ops.forEach((o, k) => {
      const it = o.it, on = sel === it.id || (o.synth && sel === o.synth.id) || (o.back && sel === o.back) ? " sel" : "";
      if (o.type === "divider") {
        pin(o, o.y, num(o, o.y + 12));
        svg += keep(rk(o), o.y, `<g class="dv${dcls(o)}${on}" data-k="${k}" tabindex="0" role="button" aria-label="${esc(it.note || it.label)}"><text x="${o.dl}" y="${o.y + 12}">${esc(this.divText(o))}</text><line x1="${o.dl}" y1="${o.y + 19}" x2="${o.dr}" y2="${o.y + 19}"/></g>`);
      } else if (o.type === "self") {
        const small = o.side || o.exit;
        const x0 = xOf(it.at), ty = o.y + (small ? 16 : 18), lo = small ? 4 : 6, hi = small ? 17 : 20;
        const kw = (it.kind === "render" || it.kind === "trigger" || it.kind === "jump") && o.merged.length === 1;
        // a step that comes back to its lifeline, or (an early exit) one that stops there
        const shape = o.reply
          ? `<path class="ln" d="M${x0} ${o.y + 11} H${x0 + 18} M${x0 + 13} ${o.y + 7} L${x0 + 18} ${o.y + 11} L${x0 + 13} ${o.y + 15}"/>`
          : o.exit
          ? `<path class="ln" d="M${x0} ${o.y + 11} H${x0 + 18}"/><path class="stop" d="M${x0 + 18} ${o.y + 5} V${o.y + 17}"/>`
          : `<path class="ln" d="M${x0} ${o.y + lo} H${x0 + 24} V${o.y + hi} H${x0 + 7}"/><path class="ah" d="M${x0 + 1} ${o.y + hi} L${x0 + 8} ${o.y + hi - 3.5} L${x0 + 8} ${o.y + hi + 3.5} Z"/>`;
        const shownLabel = this.cap(it.label);
        const head = it.kind === "render" ? "render " : shownLabel.split(" ")[0] + " ";
        const said = this.rowSaid(o);
        const body = said ? said.map(([t, c]) => (c ? `<tspan class="${c}">${esc(t)}</tspan>` : esc(t))).join("") + (o.opens ? esc(this.refText(o.opens)) : "")
          : kw ? `<tspan class="kd">${esc(head)}</tspan>${esc(it.kind === "render" ? shownLabel : shownLabel.slice(head.length))}` : esc(this.selfLabel(o).slice(this.condText(o).length));
        pin(o, o.y, num(o, ty) + testMark(o, ty));
        svg += keep(rk(o), o.y, `<g class="it ${it.kind}${o.throws ? " throw" : ""}${dcls(o)}${on}" data-k="${k}" tabindex="0" role="button" aria-label="${esc(this.selfLabel(o) + this.alsoText(o) + this.sideText(o))}">` +
          `<path class="hit" d="M${x0} ${o.y + lo} H${x0 + 26} V${o.y + hi} H${x0}"/>` + shape +
          `<text x="${x0 + 32}" y="${ty}">${o.cond ? `<tspan class="cond">${esc(this.condText(o))}</tspan>` : ""}${body}${o.also ? `<tspan class="tag">${esc(this.alsoText(o))}</tspan>` : ""}${o.side?.length ? `<tspan class="tag">${esc(this.sideText(o))}</tspan>` : ""}</text></g>`);
      } else if (this.isSelfCall(o)) {
        const x0 = xOf(it.from), ty = o.y + 18;
        const label = this.msgSegs(o).map(([t, c]) => (c ? `<tspan class="${c}">${esc(t)}</tspan>` : esc(t))).join("");
        pin(o, o.y, num(o, ty) + testMark(o, ty));
        svg += keep(rk(o), o.y, `<g class="it${dcls(o)}${on}" data-k="${k}" tabindex="0" role="button" aria-label="${esc(this.msgText(o))}">` +
          `<path class="hit" d="M${x0} ${o.y + 6} H${x0 + 26} V${o.y + 20} H${x0}"/>` +
          `<path class="ln" d="M${x0} ${o.y + 6} H${x0 + 24} V${o.y + 20} H${x0 + 7}"/><path class="ah" d="M${x0 + 1} ${o.y + 20} L${x0 + 8} ${o.y + 16.5} L${x0 + 8} ${o.y + 23.5} Z"/>` +
          chip(o, x0 + 32, false, ty, LABEL) + `<text x="${x0 + 32}" y="${ty}">${label}</text></g>`);
      } else if (o.type === "msg") {
        const small = o.exit || o.back;
        const a = xOf(it.from), b = xOf(it.to), ly = o.y + (small ? 18 : 22), dir = b > a ? 1 : -1, ret = it.t === "return", end = b - dir;
        const head = ret ? `<path class="ln" d="M${end - dir * 7} ${ly - 4} L${end} ${ly} L${end - dir * 7} ${ly + 4}"/>` : `<path class="ah" d="M${end} ${ly} L${end - dir * 8} ${ly - 4} L${end - dir * 8} ${ly + 4} Z"/>`;
        const label = this.msgSegs(o).map(([t, c]) => (c ? `<tspan class="${c}">${esc(t)}</tspan>` : esc(t))).join("");
        const ty = o.y + (small ? 11 : 14);
        pin(o, o.y, num(o, ty) + testMark(o, ty + 4));
        svg += keep(rk(o), o.y,
          `<g class="it${ret ? " ret" : ""}${small ? " exit" : ""}${o.throws ? " throw" : ""}${it.link && it.link.kind === "inferred" ? " inf" : ""}${dcls(o)}${on}" data-k="${k}" tabindex="0" role="button" aria-label="${esc(this.pname(it.from) + " to " + this.pname(it.to) + ": " + this.msgText(o))}">` +
          `<line class="hit" x1="${a}" y1="${ly - 6}" x2="${b}" y2="${ly - 6}"/><line class="ln" x1="${a}" y1="${ly}" x2="${end - dir * (ret ? 0 : 7)}" y2="${ly}"/>${head}` +
          chip(o, mid(o), true, ty, small ? EXIT : LABEL) + `<text x="${mid(o)}" y="${ty}" text-anchor="middle">${label}</text></g>`);
      }
    });
    return {
      svg: `<svg class="seq" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-label="Sequence diagram: ${esc(this.S.title)}">${svg}</svg>`,
      head, gut, gutW: GUT, gutTop: TOP - 6, W, H, headH, ys,
    };
  }

  // ---------- interaction ----------
  /** A click on a drawn step: opens or closes what it folds, and selects it. */
  act(k: number): void {
    const o = this.ops[k];
    if (!o) return;
    if (o.ghostOf) {
      this.sel = o.ghostOf;
      return;
    }
    if (o.back) {
      this.sel = o.back;
      return;
    }
    if (o.type === "msg" && o.it.t === "call" && o.it.body) {
      if (this.open.has(o.it.id)) this.open.delete(o.it.id);
      else this.open.add(o.it.id);
      this.sel = o.it.id;
    } else if (o.synth) {
      this.open.add(o.synth.id);
      this.sel = o.synth.id;
    } else this.sel = o.it.id;
  }

  toggle(id: string): void {
    if (this.open.has(id)) this.open.delete(id);
    else this.open.add(id);
  }

  /** Makes a step visible: opens every call it sits inside. */
  reveal(id: string): void {
    const path: string[] = [];
    const find = (items: Item[], trail: string[]): boolean => {
      for (const it of items) {
        if (it.id === id) {
          path.push(...trail);
          return true;
        }
        if (it.t === "frame" && (find(it.items, trail) || find(it.else || [], trail))) return true;
        if (it.t === "call" && it.body && find(it.body, [...trail, it.id])) return true;
      }
      return false;
    };
    find(this.S.items, []);
    for (const c of path) this.open.add(c);
    this.sel = id;
  }
}
