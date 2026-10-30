// Code blocks in the details panel: the file as it was when the model was extracted, with light highlighting.
// Each line's text is its own flex item (`.ct`): highlighted spans placed directly in a flex row lose the spaces
// between them.
import type { Ref } from "./model";
import { esc } from "./ui";

const KW = /^(?:const|let|var|return|if|else|await|async|function|export|default|import|from|type|new|null|true|false|for|of|while|throw|try|catch|finally|typeof|in|def|class|None|True|False|elif|with|as|raise|lambda|pass|yield)$/;

export function hl(line: string): string {
  const re = /(\/\/.*$|#.*$)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\b\d+(?:\.\d+)?\b)|([A-Za-z_$][\w$]*)(?=\s*\()|([A-Za-z_$][\w$]*)/g;
  let out = "", last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(line))) {
    out += esc(line.slice(last, m.index));
    const [t] = m;
    if (m[1]) out += `<span class="c-cm">${esc(t)}</span>`;
    else if (m[2]) out += `<span class="c-str">${esc(t)}</span>`;
    else if (m[3]) out += `<span class="c-num">${esc(t)}</span>`;
    else if (KW.test(t) && line[m.index - 1] !== ".") out += `<span class="c-kw">${t}</span>`;
    else out += m[4] ? `<span class="c-fn">${esc(t)}</span>` : esc(t);
    last = m.index + t.length;
  }
  return out + esc(line.slice(last));
}

export const lines = (r: Ref) => (r.from === r.to ? `line ${r.from}` : `lines ${r.from}–${r.to}`);

export interface BlockOpts {
  /** the heading over the block: what this code is ("Call site", "Definition") */
  label: string;
  /** lines shown around the range; the range itself is highlighted when context > 0 */
  context: number;
  /** at most this many lines unless `expanded` */
  max: number;
  expanded?: boolean;
  /** key for the "show the rest" button */
  key?: string;
}

/** A labeled code block for `ref`, or an empty string when the file is not in the model. */
export function codeBlock(files: Record<string, string>, ref: Ref, o: BlockOpts): string {
  const src = files[ref.file];
  if (src == null) return "";
  const all = src.split("\n");
  const a = Math.max(1, ref.from - o.context);
  const b = Math.min(all.length, ref.to + o.context);
  const end = o.expanded ? b : Math.min(b, a + o.max - 1);
  let h = "";
  for (let l = a; l <= end; l++) {
    const on = o.context > 0 && l >= ref.from && l <= ref.to;
    h += `<div class="cl${on ? " hl" : ""}"><span class="cn">${l}</span><span class="ct">${hl(all[l - 1])}</span></div>`;
  }
  const more = b - end;
  return (
    `<div class="blk-h"><span class="blk-l">${esc(o.label)}</span><button type="button" class="blk-f" data-act="edit" data-file="${esc(ref.file)}" data-line="${ref.from}" title="Open in editor">${esc(ref.file)} · ${lines(ref)}</button></div>` +
    `<div class="code"><div class="code-in">${h}</div></div>` +
    (more > 0 && o.key ? `<button type="button" class="link more" data-act="more" data-key="${esc(o.key)}">Show ${more} more line${more === 1 ? "" : "s"}</button>` : "")
  );
}
