// The story model as the core writes it (core/src/model.rs), plus what the shell adds around it.

export interface Ref { file: string; from: number; to: number }

export interface Participant {
  id: string;
  name: string;
  kind: "screen" | "component" | "endpoint" | "function" | "database" | "auth" | "service";
  group: string;
  sub?: string;
  ref?: Ref;
}

export interface Link { kind: "exact" | "inferred"; how: string }

export interface CallItem {
  t: "call";
  id: string;
  from: string;
  to: string;
  label: string;
  detail?: string;
  link: Link;
  effect?: "reads" | "writes" | "deletes";
  ref: Ref;
  target?: Ref;
  body?: Item[];
  note?: string;
  opens?: string;
  /** the story this step hands work to, told there (a job given to a scheduler): its id */
  story?: string;
  table?: string;
  touches?: string[];
  /** the caller does nothing with what the call gives back */
  unused?: boolean;
  /** the name the caller gives what the call gives back */
  into?: string;
  /** how the call can end besides returning: what is thrown in it or in what it calls, and not caught on the way */
  ends?: string[];
  /** the callee's parameters whose values it changes, as it names them */
  changes?: string[];
}

export interface ReturnItem {
  t: "return";
  id: string;
  from: string;
  to: string;
  label: string;
  detail?: string;
  ref: Ref;
  /** the function ends by throwing: the label says what, the detail is the statement */
  throws?: boolean;
  /** the function ends here before its end (code after it would run otherwise, or a loop stops): a rule */
  early?: boolean;
  /** a throw in a catch that throws what it caught again: no new way to end. What it passes on (empty: nothing named) */
  passes?: string[];
}

export interface SelfItem {
  t: "self";
  id: string;
  at: string;
  /** `jump`: a `break` or `continue`, which ends this round of the loop it is in */
  kind: "local" | "state" | "render" | "reads" | "trigger" | "changes" | "jump";
  label: string;
  detail?: string;
  ref: Ref;
}

export interface FrameItem {
  t: "frame";
  id: string;
  kind: "alt" | "loop" | "effect" | "catch";
  label: string;
  note?: string;
  ref: Ref;
  items: Item[];
  else?: Item[];
}

export type Item = CallItem | ReturnItem | SelfItem | FrameItem;

export interface Finding { text: string; ref: Ref }

export interface Coverage { fn: string; file: string; total: number; covered: number; missing: number[] }

export interface Trigger {
  kind: string; // "screen-open" | "user-action" | "endpoint" | "schedule" | "lifecycle" | "function" | "file" | "test"
  route: string;
  ref: Ref;
  event?: string;
  control?: string;
  text?: string;
  handler?: string;
  /** the place it starts in, as its adapter gives it or where the code that hands it over runs ("device" | "browser" | "server" | …) */
  place?: string;
  /** schedule, lifecycle: when it runs ("every 5 minutes", "when the app starts") */
  when?: string;
  /** lifecycle: a layout, drawn around the screens under this address ("/docs") */
  around?: string;
  /** function, test: how something outside the project reaches it ("decorated" | "passed" | "value" | "uncalled") */
  how?: string;
  /** the code that shows it: the decorator, the call it is passed to, where it is used as a value */
  by?: string;
  byRef?: Ref;
  /** classes outside the project that its class extends */
  extends?: string[];
}

export interface Story {
  id: string;
  title: string;
  trigger: Trigger;
  /** other ways the same story starts: triggers on the same screen whose steps are the same */
  also?: Trigger[];
  participants: Participant[];
  items: Item[];
  signals: Finding[];
  gaps: Finding[];
  coverage: Coverage[];
}

/** How a story reaches a thing (core/src/story/reach.rs). */
export type UseKind = "starts" | "calls" | "reads" | "writes" | "deletes" | "draws";

/** One place a story reaches a thing. */
export interface Use {
  story: string;
  /** the step that draws it, or (not `drawn`) the step it is inside */
  step?: string;
  drawn: boolean;
  how: UseKind;
  ref: Ref;
  from?: string;
}

/** A function, package, service, endpoint or table, with every place the stories reach it. A lower bound: what the
 *  stories cannot follow is not counted. */
export interface Thing {
  /** a participant id, or `<participant id>/<name>` for a table or database function in one */
  id: string;
  name: string;
  kind: Participant["kind"];
  group: string;
  sub?: string;
  ref?: Ref;
  /** for a table or database function: the participant it is in */
  in?: string;
  uses: Use[];
}

export interface ProjectModel {
  name: string;
  stories: Story[];
  /** what the stories reach, and where (absent in models written before it existed) */
  index?: Thing[];
  files: Record<string, string>;
  stats: { ms: number; llmCalls: number };
  setup: string[];
  /** imported packages that are not installed where the language server looks */
  missing?: NotInstalled[];
}

/** A package that is not installed: where the project lists it, and how its own tools would install it (core/src/deps.rs). */
export interface NotInstalled {
  package: string;
  /** the language server of the files that import it: "python" | "typescript" | "dart" */
  language: string;
  /** the manifests that list it under this name */
  listed: string[];
  /** where it was looked for: the project's Python environment, when it is inside the project */
  env?: string;
  /** commands to run from the project's folder; the user runs them, never plainfold */
  install?: string[];
}

/** A step in plain English, written by a model and checked against the code (core/src/english). */
export interface Said {
  text: string;
  /** where the names and values copied from the code are in `text` */
  names: [number, number][];
  /** names in the code that read like instructions, which the model was not shown */
  masked?: string[];
}

/** Texts that arrived for a read, and how far the work is. */
export interface EnglishBatch { id: number; texts: Record<string, Said>; done: number; total: number; tokensIn: number; tokensOut: number }

/** The English for a read is done (or stopped). */
export interface EnglishEnd {
  id: number;
  steps: number;
  described: number;
  rejected: Record<string, number>;
  requests: number;
  /** requests answered from what was kept */
  kept: number;
  tokensIn: number;
  tokensOut: number;
  error: string | null;
  stopped: boolean;
}

/** Where the key for the model comes from, and the model. */
export interface KeyState { from: "keychain" | "environment" | null; model: string }

/** A language server that was used: always ready, since an unready one never is. */
export interface ServerState { kind: string; name: string; with: string; readiness: string }

/** What the shell returns for an opened project. */
export interface Opened {
  path: string;
  model: ProjectModel;
  adapters: string[];
  servers: ServerState[];
  /** languages that had no server; their stories are left out */
  failed: { kind: string; why: string }[];
  missing: string[];
  /** what changed since the user last marked the changes as seen (core/src/diff.rs) */
  diff: StoryDiff[];
}

export type Mark = "added" | "removed" | "changed";
export interface Was { label: string; detail?: string }

/** A step that is new, gone or different. A removed one comes with the step as it was and where it goes. */
export interface StepDiff {
  id: string;
  mark: Mark;
  was?: Was;
  item?: Item;
  parent?: string;
  list?: string;
  after?: string;
}

export interface StoryDiff {
  id: string;
  title: string;
  mark: Mark;
  steps: StepDiff[];
  /** a removed story as it was */
  story?: Story;
  /** participants a changed story had and has no more, which its removed steps still reach */
  gone?: Participant[];
  /** the story starts in a function renamed since: the name it had */
  renamed?: string;
  /** the story starts in a function moved to another file since: the file it was in */
  moved?: string;
}

/** A file the stories were built from that differs now (core/src/changes.rs). */
export interface Change { file: string; how: "added" | "changed" | "removed" }

/** A language the project uses, and whether its server can run. */
export interface Need {
  kind: string;
  state: "ready" | "download" | "unavailable";
  /** where the server is, what would be downloaded, or why it cannot run */
  detail: string;
  label: string;
  /** for a download, what it brings; two languages may need the same part (Node.js) */
  parts: Part[];
}

/** Something a download brings (core/src/lib.rs). */
export interface Part {
  label: string;
  /** exactly what is downloaded */
  detail: string;
  /** why it is needed, when that is not plain from the label */
  why?: string | null;
  /** where it is downloaded from */
  from: string;
  /** megabytes, roughly: downloaded, and kept on disk */
  downloadMb: number;
  diskMb: number;
}

export type Progress =
  | { step: "loaded"; name: string; files: number }
  | { step: "adapters"; names: string[] }
  | { step: "server-starting"; kind: string }
  | { step: "server-waiting"; kind: string; seconds: number; doing: string }
  | { step: "server-ready"; kind: string; name: string; readiness: string }
  /** a server kept running from an earlier read is used again; `changed` source files were sent to it first */
  | { step: "server-kept"; kind: string; name: string; changed: number }
  | { step: "server-failed"; kind: string; why: string }
  | { step: "downloading"; kind: string; label: string; done: number; total: number }
  | { step: "building" };

/** A language server kept running for the window's project (core/src/keep.rs). */
export interface Held {
  kind: string;
  name: string;
  /** memory it and the processes it started hold; null where it cannot be measured */
  bytes: number | null;
  /** when it started, in seconds since 1970 */
  since: number;
  reads: number;
}

/** What the settings show about this window's language servers. */
export interface ServerSettings {
  keep: boolean;
  /** the project they run for */
  project: string | null;
  running: Held[];
  /** why they were last closed, and when (seconds since 1970) */
  closed: [string, number] | null;
}

export interface Recent { path: string; name: string; stories: number; adapters: string[] }

export interface AppState {
  recent: Recent[];
  showWelcome: boolean;
  /** download the language servers a project needs without asking */
  autoDownload: boolean;
  /** keep a project's language servers running between reads */
  keepServers: boolean;
  /** the steps in plain English, where there is a key for a model */
  english: boolean;
  /** a folder given on the command line, opened instead of the welcome window */
  open?: string | null;
}

export const walk = (items: Item[], f: (i: Item) => void): void => {
  for (const it of items) {
    f(it);
    if (it.t === "frame") {
      walk(it.items, f);
      walk(it.else || [], f);
    }
    if (it.t === "call" && it.body) walk(it.body, f);
  }
};

export const effectsOf = (s: Story): Set<string> => {
  const e = new Set<string>();
  walk(s.items, (i) => {
    if (i.t === "call" && i.effect) e.add(i.effect);
  });
  return e;
};

/** What a call touches ("reads items", as the core writes it), by verb, in the order first touched. */
export const touchGroups = (touches: string[]): [string, string[]][] => {
  const by = new Map<string, string[]>();
  for (const t of touches) {
    const [verb, ...name] = t.split(" ");
    const names = by.get(verb) ?? [];
    if (!names.includes(name.join(" "))) names.push(name.join(" "));
    by.set(verb, names);
  }
  return [...by];
};

/** Every step of a story, nested ones included (what the command line counts as items). */
export const stepCount = (s: Story): number => {
  let n = 0;
  walk(s.items, () => n++);
  return n;
};

// What a person would call the flow: the words on the control, else the handler's name in plain words, else the event.
// A function that starts a story is named the same way: the words written where it is registered, else its name.
const words = (name: string, lead = /^(handle|handler|on)$/i) =>
  name
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter((w, i) => !(i === 0 && lead.test(w)))
    .join(" ")
    .toLowerCase();
const cap = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);

export function flowName(st: Story): { main: string; sub: string } {
  const n = baseName(st), extra = apart.get(st);
  return extra ? { ...n, main: `${n.main} · ${extra}` } : n;
}

// Stories of one place that would read the same are told apart by what differs: the handler's name when the words on
// the control named them, else the line the control is written on.
const apart = new WeakMap<Story, string>();
export function nameStories(stories: Story[]): void {
  const same = new Map<string, Story[]>();
  for (const st of stories) {
    const k = `${st.trigger.kind}\n${st.trigger.route}\n${baseName(st).main}`;
    same.set(k, [...(same.get(k) ?? []), st]);
  }
  for (const list of same.values()) {
    if (list.length < 2) continue;
    const handler = (st: Story) => (st.trigger.text && st.trigger.handler ? words(st.trigger.handler) : "");
    const by = list.map(handler);
    list.forEach((st, i) => apart.set(st, by[i] && by.indexOf(by[i]) === by.lastIndexOf(by[i]) ? by[i] : `line ${st.trigger.ref.from}`));
  }
}

/** The story a step refers to, told there: a screen it opens, or one it hands work to. */
export const refStory = (it: CallItem): string | undefined => it.story ?? (it.opens ? "open:" + it.opens : undefined);

function baseName(st: Story): { main: string; sub: string } {
  const t = st.trigger;
  // what starts by itself is named by its function, and says when it runs
  if ((t.kind === "schedule" || t.kind === "lifecycle") && t.handler && words(t.handler)) return { main: cap(words(t.handler)), sub: t.when ?? "" };
  if (t.kind === "function" || t.kind === "test") {
    const lead = t.kind === "test" ? /^(test|handle|handler|on)$/i : undefined;
    if (t.text) return { main: `“${t.text}”`, sub: "" };
    if (t.handler && words(t.handler, lead)) return { main: cap(words(t.handler, lead)), sub: "" };
    return { main: st.title, sub: "" };
  }
  if (t.kind !== "user-action" || (!t.text && !t.handler && !t.event)) return { main: st.title, sub: "" };
  const main = t.text ? `“${t.text}”` : t.handler && words(t.handler) ? cap(words(t.handler)) : cap(t.event ? words(t.event) : "action");
  return { main, sub: [startWords(t), ...alsoWords(st)].join(" · ") };
}

/** How a user action starts, as a person says it: "pressed on SaveButton". */
const startWords = (t: Trigger): string => [t.event && words(t.event), t.control && `on ${t.control}`].filter(Boolean).join(" ");

/** The other ways a story starts, each as it differs from the first: its words ("or “Done”"), another control of the
 *  same kind ("or another Pill"), else how it starts ("or field submitted on TextField"). */
export const alsoWords = (st: Story): string[] =>
  (st.also ?? []).map((a) =>
    a.text ? `or “${a.text}”`
    : a.event === st.trigger.event && a.control === st.trigger.control ? `or another ${a.control ?? "one"}`
    : `or ${startWords(a)}`);
