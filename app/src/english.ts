// The steps in plain English (core/src/english): written by a model, checked against the code, and kept apart from
// the model. A step is found again from itself alone, the way the core keys it (`site_key`).
import type { Item, Said } from "./model";
import { esc } from "./ui";

/** The key the core gives a step's text: where it is written and what it is. */
export function siteKey(it: Item): string | null {
  const kind =
    it.t === "call" ? "call"
    : it.t === "self" && (it.kind === "state" || it.kind === "render" || it.kind === "reads") ? `self/${it.kind}`
    : it.t === "frame" && ((it.kind === "alt" && !it.note) || it.kind === "loop" || it.kind === "catch") ? `frame/${it.kind}`
    : null;
  return kind && `${it.ref.file}:${it.ref.from}-${it.ref.to} ${kind} ${it.label}`;
}

/** A text as segments: the model's words (""), and the names and values copied from the code ("nm"); cut past `max`
 *  characters, the panel shows it whole. */
export function segs(s: Said, max = Infinity): [string, string][] {
  const out: [string, string][] = [];
  let at = 0;
  for (const [a, b] of s.names) {
    if (a > at) out.push([s.text.slice(at, a), ""]);
    out.push([s.text.slice(a, b), "nm"]);
    at = b;
  }
  if (at < s.text.length) out.push([s.text.slice(at), ""]);
  if (s.text.length <= max) return out;
  const cut: [string, string][] = [];
  let left = max - 1;
  for (const [t, c] of out) {
    if (left <= 0) break;
    cut.push([t.slice(0, left), c]);
    left -= t.length;
  }
  cut.push(["…", ""]);
  return cut;
}

/** As HTML (`nm` spans), for the panel. */
export const html = (s: Said): string => segs(s).map(([t, c]) => (c ? `<span class="${c}">${esc(t)}</span>` : esc(t))).join("");
