// What changed in a story since the user last marked the changes as seen, made ready for the diagram: removed steps
// are put back where they were, and every step gets its mark. Everything comes from the core's comparison
// (core/src/diff.rs); nothing here compares code.
import type { Item, Mark, Story, StoryDiff, Was } from "./model";
import { walk } from "./model";

export interface Marked {
  story: Story;
  /** step id → how it changed; what an added or removed step holds has the same mark */
  marks: Map<string, Mark>;
  /** a changed step as it was */
  was: Map<string, Was>;
  /** the steps the comparison names itself, not those inside an added or removed one: these show even where the
   *  diagram would hide them (a local step) */
  named: Set<string>;
}

const markAll = (items: Item[], m: Mark, marks: Map<string, Mark>) => walk(items, (i) => marks.set(i.id, m));

/** The list a removed step goes back into: the top of the story, or one of a step's lists ("" or "else"). */
function listOf(items: Item[], parent: string | undefined, name: string | undefined): Item[] | null {
  if (!parent) return items;
  let found: Item[] | null = null;
  walk(items, (i) => {
    if (found || i.id !== parent) return;
    if (i.t === "call") found = i.body ?? (i.body = []);
    else if (i.t === "frame") found = name === "else" ? (i.else ?? (i.else = [])) : i.items;
  });
  return found;
}

export function withChanges(story: Story, d: StoryDiff | undefined): Marked {
  const marks = new Map<string, Mark>(), was = new Map<string, Was>(), named = new Set<string>();
  if (!d) return { story, marks, was, named };
  if (d.mark !== "changed") {
    const s = d.mark === "removed" && d.story ? d.story : story;
    markAll(s.items, d.mark, marks);
    return { story: s, marks, was, named };
  }
  const s: Story = structuredClone(story);
  // a participant only removed steps reach goes back after the last one of its group, as the core orders them
  for (const p of d.gone ?? []) {
    const at = s.participants.map((q) => q.group).lastIndexOf(p.group);
    s.participants.splice(at >= 0 ? at + 1 : s.participants.length, 0, p);
  }
  // removed steps that share a place keep their order: each goes after the one put back before it
  const last = new Map<string, string>();
  for (const st of d.steps) {
    if (st.mark === "removed" && st.item) {
      const list = listOf(s.items, st.parent, st.list);
      if (!list) continue;
      const key = `${st.parent ?? ""}|${st.list ?? ""}|${st.after ?? ""}`;
      const anchor = last.get(key) ?? st.after;
      list.splice(anchor ? list.findIndex((i) => i.id === anchor) + 1 : 0, 0, st.item);
      last.set(key, st.item.id);
      markAll([st.item], "removed", marks);
      named.add(st.item.id);
    } else if (st.mark === "added") {
      walk(s.items, (i) => {
        if (i.id === st.id) markAll([i], "added", marks);
      });
      named.add(st.id);
    } else if (st.mark === "changed") {
      marks.set(st.id, "changed");
      if (st.was) was.set(st.id, st.was);
      named.add(st.id);
    }
  }
  return { story: s, marks, was, named };
}

export const SIGN: Record<Mark, string> = { added: "+", removed: "−", changed: "~" };
