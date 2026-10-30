// The details panel: what the selected step is, and the lines behind it. The arrow and its code block come from the
// call site; the definition is a different place, and the panel says which is which.
import { codeBlock } from "./code";
import type { StoryView } from "./diagram";
import { html as saidHtml } from "./english";
import type { Finding, Item, ProjectModel } from "./model";
import { alsoWords, flowName, refStory, touchGroups } from "./model";
import { ICON, esc } from "./ui";

export interface PanelState {
  /** code blocks opened past their first lines, by key */
  expanded: Set<string>;
}

const FRAME_SAYS: Record<string, string> = {
  alt: "Runs only when the condition holds.",
  catch: "Runs only when the code before it throws.",
  loop: "Repeats for each element.",
  effect: "Runs after the screen has appeared, not in the order written.",
};
const MARK_SAYS: Record<string, string> = { added: "Added", removed: "Removed", changed: "Changed" };
const SITE: Record<string, string> = { call: "Call site", return: "Returned at", self: "Written at", frame: "Condition at" };

const actions = (file: string, line: number) =>
  `<div class="acts"><button type="button" class="btn" data-act="edit" data-file="${esc(file)}" data-line="${line}">${ICON.external()}Open in editor</button>` +
  `<button type="button" class="btn" data-act="copy" data-text="${esc(file + ":" + line)}">${ICON.copy()}Copy location</button></div>`;

/** How many other stories (and, with `tests`, tests) reach a thing, as a way into them (the list narrows to them).
 *  Nothing when only this story does. */
function reachedBy(v: StoryView, model: ProjectModel, id: string, what: string, tests = true): string {
  const thing = model.index?.find((x) => x.id === id);
  if (!thing) return "";
  const here = v.S.trigger.kind !== "test" && thing.uses.some((u) => u.story === v.S.id);
  const n = v.reach.stories(id) - (here ? 1 : 0), t = tests ? v.reach.tests(id).length : 0;
  if (!n && !t) return "";
  const words = [n ? `${n} ${here ? "other " : ""}${n === 1 ? "story" : "stories"}` : "", t ? (t === 1 ? "1 test" : `${t} tests`) : ""].filter(Boolean).join(" and ");
  return `<button type="button" class="link toggle" data-act="focus" data-thing="${esc(id)}">› ${words} ${n + t === 1 ? "reaches" : "reach"} ${esc(what)} · list them</button>`;
}

/** The tests that reach the function a step calls, each a way into that test; nothing in a project without tests. */
function testsReaching(v: StoryView, model: ProjectModel, id: string): string {
  if (!model.stories.some((s) => s.trigger.kind === "test")) return "";
  const list = v.reach.tests(id)
    .map((t) => model.stories.findIndex((s) => s.id === t))
    .filter((i) => i >= 0)
    .map((i) => `<button type="button" class="exit" data-act="story" data-id="${i}"><span>${esc(flowName(model.stories[i]).main)}</span><span class="at">${esc(model.stories[i].trigger.route)}</span></button>`)
    .join("");
  if (!list) return `<p class="note">No test reaches it that plainfold can follow.</p>`;
  return `<div class="sec"><div class="blk-h"><span class="blk-l">Tests that reach it</span></div>${list}</div>`;
}

const head = (title: string) => `<div class="ititle">${esc(title)}</div>`;

export function panel(v: StoryView, model: ProjectModel, findings: (Finding & { kind: string })[], st: PanelState): string {
  const sel = v.sel;
  if (!sel) return `<p class="hint">Select a step to see the lines behind it.</p>`;
  const files = model.files;
  const block = (ref: Item["ref"], label: string, context: number, max: number, key: string) =>
    codeBlock(files, ref, { label, context, max, key, expanded: st.expanded.has(key) });

  if (sel.startsWith("p:")) {
    const p = v.PI[sel.slice(2)];
    if (!p) return "";
    let h = head(p.name) + reachedBy(v, model, p.id, p.name);
    h += p.ref
      ? `<div class="sec">${block(p.ref, "Definition", 0, 14, "p:" + p.id)}</div>` + actions(p.ref.file, p.ref.from)
      : `<p class="note">Not in this project's code. plainfold knows it only through the calls that reach it.</p>`;
    return h;
  }
  if (sel.startsWith("f:")) {
    const f = findings[+sel.slice(2)];
    if (!f) return "";
    let h = `<div class="kicker">${f.kind === "signal" ? "What the code shows" : "Not resolved yet"}</div>` + head(f.text);
    if (files[f.ref.file]) h += `<div class="sec">${block(f.ref, "Where", 3, 16, "f:" + sel)}</div>` + actions(f.ref.file, f.ref.from);
    return h;
  }

  const it = v.BY[sel];
  if (!it) return "";
  // the arrow already says who calls whom; the panel starts with the code text
  const title = it.t === "frame" ? `${it.kind} ${it.label}` : (it.t === "self" && it.kind === "render" ? "render " : "") + it.label;
  // in plain English, the code text right under it; the English is the model's words around names from the code
  const said = v.saidOf(it);
  // the ≈ in front says the words are a model's; what it means is on hover and in "How to read this"
  const by = `In plain English by ${v.englishBy}, checked against this code. Names and values are copied from it.`;
  let h = said ? `<div class="ititle en"><span class="approx" title="${esc(by)}">≈</span> ${saidHtml(said)}</div><div class="icode">${esc(title)}</div>` : head(title);
  if (it.t === "call" && it.body) {
    h += `<button type="button" class="link toggle" data-act="toggle" data-id="${esc(it.id)}">${v.open.has(it.id) ? "▾ Close in the diagram" : "▸ Open in the diagram"}</button>`;
  }
  // a frame the core explains itself (a callback a library runs, the versions a call can run) says so in its note
  if (it.t === "frame") h += `<div class="route">${esc(it.kind === "effect" && it.note ? `Runs ${it.note}, not in the order written.` : it.note ? `${it.note}.` : FRAME_SAYS[it.kind])}</div>`;
  if (it.t === "self" && it.kind === "changes") h += `<div class="route">Changes a value the function is given, or its own object</div>`;
  if (it.t === "self" && it.kind === "jump") h += `<div class="route">This round of the loop ends here: continue goes on to the next round, break leaves the loop</div>`;
  if (it.t === "return" && (it.throws || v.condOf.has(it.id))) h += `<div class="route">${it.throws ? "A rule: the work ends here with what is thrown" : "A rule: the work ends here early"}</div>`;
  if (said?.masked?.length) h += `<p class="note warm">${esc(said.masked.join(", "))} ${said.masked.length === 1 ? "reads" : "read"} like an instruction, so the model was not shown ${said.masked.length === 1 ? "it" : "them"}.</p>`;

  // what changed since the user last looked
  const m = v.mark.get(it.id);
  if (m) h += `<div class="pill d-${m}"><span class="dot"></span>${MARK_SAYS[m]} since you last looked</div>`;
  if (m === "removed") {
    // the lines it stood for are gone from the file, so only how it was written is left to show
    const was = (it.t !== "frame" && it.detail) || title;
    return h + `<div class="sec"><div class="blk-h"><span class="blk-l">Was</span><span class="blk-f">${esc(it.ref.file)}:${it.ref.from}</span></div><pre class="was-code">${esc(was)}</pre></div>` +
      `<p class="hint">Its code is no longer in the file, so only the way it was written is shown.</p>`;
  }
  const w = v.was.get(it.id);
  if (m === "changed" && w) h += `<div class="sec"><div class="blk-h"><span class="blk-l">Before</span></div><pre class="was-code">${esc(w.detail ?? w.label)}</pre></div>`;

  if (it.t === "call") {
    if (v.isSide(it)) {
      const n = v.reach.stories(it.to);
      h += `<p class="note">Side work: what it gives back is not used, it reaches no table or endpoint of the project, and ${n} of the ${v.reach.total} stories reach it. It is one quiet row and never a column; Every step shows it as a call.</p>`;
    }
    // an inferred link is not proven: that is said up front; an exact one is in "How this was found"
    if (it.link.kind === "inferred") h += `<div class="pill inferred"><span class="dot"></span>inferred · ${esc(it.link.how)}</div>`;
    // what it touches, a line per verb; what it changes in the warm color, as in the list
    for (const [verb, names] of touchGroups(it.touches ?? [])) {
      h += `<div class="route${verb === "reads" ? "" : " warm"}">${esc(verb[0].toUpperCase() + verb.slice(1))} ${esc(names.join(", "))}</div>`;
    }
    // what it reaches outside the project's code: its row names it only when it changes no data
    const to = it.body && it.from !== it.to ? v.sideTargets(it) : [];
    if (to.length) h += `<div class="route">Reaches ${esc(to.join(", "))}</div>`;
    if (it.ends?.length) h += `<div class="route">Can end with ${esc(it.ends.join(" · "))}, thrown inside and not caught</div>`;
    if (it.changes?.length) h += `<div class="route">Changes ${esc(it.changes.join(", "))}</div>`;
    if (it.note) h += `<p class="note">${esc(it.note)}</p>`;
    // the tests that reach a function are listed below, so here only the stories count
    if (it.from !== it.to) h += reachedBy(v, model, it.to, v.pname(it.to), false);
    if (it.table) h += reachedBy(v, model, `${it.to}/${it.table}`, it.table);
  }
  if (it.t === "return" && v.condOf.has(it.id)) h += `<div class="pill"><span class="dot"></span>only when ${esc(v.condOf.get(it.id)!)}</div>`;

  if (it.t === "call" && refStory(it)) {
    const i = model.stories.findIndex((s) => s.id === refStory(it));
    const says = it.opens ? "Starts the story" : "Runs later as the story";
    if (i >= 0) h += `<button type="button" class="link toggle" data-act="story" data-id="${i}">› ${says} “${esc(flowName(model.stories[i]).main)}”</button>`;
  }

  if (it.t === "call" && it.from !== it.to && ["function", "endpoint", "screen", "component"].includes(v.PI[it.to]?.kind ?? "")) h += testsReaching(v, model, it.to);
  // only a branch has a condition; a loop, a frame that runs later and the versions a call can run are just written there
  const site = it.t === "frame" && (it.kind !== "alt" || it.note) ? "Written at" : it.t === "return" && it.throws ? "Thrown at" : SITE[it.t];
  h += `<div class="sec">${block(it.ref, site, it.ref.from === it.ref.to ? 2 : 1, 14, "site:" + it.id)}</div>`;
  // the other ways the same story starts, each where it is written
  if (it.t === "self" && it.kind === "trigger") {
    const words = alsoWords(v.S);
    (v.S.also ?? []).forEach((t, i) => (h += `<div class="sec">${block(t.ref, words[i][0].toUpperCase() + words[i].slice(1), 1, 8, `also:${i}`)}</div>`));
  }
  if (it.t === "call" && it.target) h += `<div class="sec">${block(it.target, "Definition", 0, 12, "def:" + it.id)}</div>`;
  else if (it.t === "call" && !it.body) h += `<div class="sec"><div class="blk-h"><span class="blk-l">Definition</span></div><p class="note">Outside the project's code, so there is no body to show.</p></div>`;

  // a rethrow only passes on how what it calls ended: no rule of its own
  const exits = v.exitsOf.get(it.id)?.exits.filter((e) => !e.passes) ?? [];
  if (exits.length) {
    h += `<div class="sec"><div class="blk-h"><span class="blk-l">Rules that end it</span></div>` +
      exits
        .map((e) => `<button type="button" class="exit" data-act="reveal" data-id="${esc(e.id)}"><span>${v.condOf.has(e.id) ? `<span class="cond">[${esc(v.condOf.get(e.id)!)}]</span> ` : ""}<span class="${e.throws ? "throw" : ""}">${esc(e.label)}</span></span><span class="at">${esc(e.ref.file)}:${e.ref.from}</span></button>`)
        .join("") +
      `</div>`;
  }
  h += actions(it.ref.file, it.ref.from);
  if (it.t === "call") h += `<details class="how"><summary>How this was found</summary><p>${esc(it.link.kind)} · ${esc(it.link.how)}</p></details>`;
  return h + `<p class="hint">Code is the file as it was when the project was read.</p>`;
}
