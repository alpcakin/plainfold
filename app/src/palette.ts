// "Go to": jump to a story, a participant (the first story it is in) or a file. Everything listed comes from the model.
import type { ProjectModel } from "./model";
import { effectsOf, flowName } from "./model";
import { ICON, esc } from "./ui";

export interface Entry {
  kind: "story" | "participant" | "file";
  main: string;
  sub: string;
  right: string;
  warm: boolean;
  mono: boolean;
  story?: number;
  participant?: string;
  file?: string;
  search: string;
}

export function entries(m: ProjectModel): Entry[] {
  const out: Entry[] = [];
  m.stories.forEach((st, i) => {
    const n = flowName(st), fx = [...effectsOf(st)];
    out.push({ kind: "story", main: n.main, sub: [n.sub, st.trigger.route].filter(Boolean).join(" · "), right: fx.join(", "), warm: fx.some((f) => f !== "reads"), mono: false, story: i, search: `${n.main} ${n.sub} ${st.title} ${st.trigger.route}`.toLowerCase() });
  });
  const seen = new Map<string, Entry & { count: number }>();
  m.stories.forEach((st, i) => {
    for (const p of st.participants) {
      const e = seen.get(p.id);
      if (e) e.count++;
      else seen.set(p.id, { kind: "participant", main: p.name, sub: p.sub ?? p.kind, right: "", warm: false, mono: true, story: i, participant: p.id, search: `${p.name} ${p.sub ?? ""} ${p.kind}`.toLowerCase(), count: 1 });
    }
  });
  for (const e of seen.values()) out.push({ ...e, right: e.count === 1 ? "in 1 story" : `in ${e.count} stories` });
  for (const f of Object.keys(m.files).sort()) out.push({ kind: "file", main: f, sub: "", right: "", warm: false, mono: true, file: f, search: f.toLowerCase() });
  return out;
}

export function filter(all: Entry[], q: string): Entry[] {
  const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hit = all.filter((e) => terms.every((t) => e.search.includes(t)));
  const per = terms.length ? 8 : 5;
  return (["story", "participant", "file"] as const).flatMap((k) => hit.filter((e) => e.kind === k).slice(0, per));
}

const mark = (text: string, q: string) => {
  const t = q.trim().toLowerCase();
  const i = t ? text.toLowerCase().indexOf(t) : -1;
  if (i < 0) return esc(text);
  return esc(text.slice(0, i)) + `<mark>${esc(text.slice(i, i + t.length))}</mark>` + esc(text.slice(i + t.length));
};

const GROUP = { story: "Stories", participant: "Participants", file: "Files" };
const icon = (e: Entry) => (e.kind === "story" ? ICON.action() : e.kind === "file" ? ICON.file() : ICON.box());

export function list(shown: Entry[], q: string, at: number): string {
  if (!shown.length) return `<div class="pal-none">Nothing matches “${esc(q)}”.</div>`;
  let h = "", last = "";
  shown.forEach((e, i) => {
    if (e.kind !== last) {
      h += `<div class="pal-g">${GROUP[e.kind]}</div>`;
      last = e.kind;
    }
    h += `<button type="button" class="pal-i${i === at ? " on" : ""}" data-act="pick" data-i="${i}">` +
      `<span class="pal-ic">${icon(e)}</span><span class="pal-m${e.mono ? " mono" : ""}">${mark(e.main, q)}</span>` +
      (e.sub ? `<span class="pal-s">${mark(e.sub, q)}</span>` : "") +
      `<span class="grow"></span>` +
      (e.right ? `<span class="pal-r${e.warm ? " warm" : ""}">${esc(e.right)}</span>` : "") +
      (i === at ? `<span class="pal-k">↵</span>` : "") +
      `</button>`;
  });
  return h;
}
