// The story model: what the deterministic layer hands to the UI (and later to the LLM layer).
// Everything here comes from the code. Labels are code text, never English written by a model.

export type Ref = { file: string; from: number; to: number }; // 1-based, inclusive

export type Group = "device" | "browser" | "server" | "supabase" | "outside";

export type Participant = {
  id: string;
  name: string;
  kind: "screen" | "component" | "endpoint" | "function" | "database" | "auth" | "service";
  group: Group;
  sub?: string;
  ref?: Ref;
};

// How an arrow was resolved. "exact" = proven by the analyzer or a framework rule; "inferred" = a guess.
export type Link = { kind: "exact" | "inferred"; how: string };

export type Effect = "reads" | "writes" | "deletes";

export type Item =
  // body: what happens inside the callee, when it crosses a boundary itself (the UI folds it)
  | { t: "call"; id: string; from: string; to: string; label: string; detail?: string; link: Link; effect?: Effect; ref: Ref; target?: Ref; body?: Item[]; note?: string; opens?: string; table?: string; touches?: string[] }
  | { t: "return"; id: string; from: string; to: string; label: string; detail?: string; ref: Ref }
  | { t: "self"; id: string; at: string; kind: "local" | "state" | "render" | "reads" | "trigger"; label: string; detail?: string; ref: Ref }
  | { t: "frame"; id: string; kind: "alt" | "loop" | "effect"; label: string; note?: string; ref: Ref; items: Item[]; else?: Item[] };

export type Call = Extract<Item, { t: "call" }>;

export type Finding = { text: string; ref: Ref };

export type Coverage = { fn: string; file: string; total: number; covered: number; missing: number[] };

export type Story = {
  id: string; // "open:/", "action:/books/new#save"
  title: string;
  trigger: { kind: "screen-open" | "user-action"; route: string; ref: Ref; event?: string; control?: string; text?: string; handler?: string }; // user-action: what was used, the words on it, and the function it runs
  participants: Participant[];
  items: Item[];
  signals: Finding[]; // things the code shows for sure (no guessing), worth a look
  gaps: Finding[]; // what the extractor could not resolve yet
  coverage: Coverage[];
};

export type ProjectModel = {
  name: string;
  stories: Story[];
  files: Record<string, string>; // source of every file the stories point into
  stats: { ms: number; llmCalls: number };
};
