// Small helpers shared by the view's parts: escaping, text measurement, icons.

export const esc = (s: unknown): string =>
  String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

export const $ = (id: string): HTMLElement => document.getElementById(id)!;

// Text width in the diagram's monospace font, measured once per string and size.
let ctx: CanvasRenderingContext2D | null = null;
const widths = new Map<string, number>();
export function tw(s: string, size: number): number {
  const key = size + "|" + s;
  const hit = widths.get(key);
  if (hit !== undefined) return hit;
  if (!ctx) ctx = document.createElement("canvas").getContext("2d");
  let w = s.length * size * 0.6;
  if (ctx) {
    ctx.font = `${size}px ${getComputedStyle(document.documentElement).getPropertyValue("--mono")}`;
    w = ctx.measureText(s).width;
  }
  widths.set(key, w);
  return w;
}

/** `s` cut in the middle with "…" until it is at most `max` wide in the diagram's font: the start and the end of a
 *  name are what tell it apart (`PATCH /items/{…}/done`). */
export function fitMiddle(s: string, size: number, max: number): string {
  if (tw(s, size) <= max) return s;
  for (let keep = s.length - 1; keep > 2; keep--) {
    const head = Math.ceil(keep * 0.55);
    const t = s.slice(0, head) + "…" + s.slice(s.length - (keep - head));
    if (tw(t, size) <= max) return t;
  }
  return s.slice(0, 1) + "…";
}

const svg = (body: string, size = 16, extra = "") =>
  `<svg width="${size}" height="${size}" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"${extra}>${body}</svg>`;

export const ICON = {
  search: (s = 16) => svg('<circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5L14 14"/>', s),
  back: () => svg('<path d="M10 3.5L5.5 8L10 12.5"/>'),
  forward: () => svg('<path d="M6 3.5L10.5 8L6 12.5"/>'),
  sidebar: () => svg('<rect x="1.5" y="2.5" width="13" height="11" rx="2.5"/><path d="M6 2.5v11"/>'),
  inspector: () => svg('<rect x="1.5" y="2.5" width="13" height="11" rx="2.5"/><path d="M10 2.5v11"/>'),
  minus: () => svg('<path d="M3.5 8h9"/>', 12),
  plus: () => svg('<path d="M3.5 8h9M8 3.5v9"/>', 12),
  chevDown: () => svg('<path d="M4.5 6.5L8 10l3.5-3.5"/>', 14),
  chevUp: () => svg('<path d="M4.5 9.5L8 6l3.5 3.5"/>', 14),
  folder: (s = 14) => svg('<path d="M1.75 4.25a1 1 0 0 1 1-1h3.1l1.4 1.5h5.9a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1z"/>', s),
  updown: () => svg('<path d="M5.5 6L8 3.5L10.5 6M5.5 10L8 12.5L10.5 10"/>', 14),
  screen: () => svg('<rect x="2" y="3" width="12" height="10" rx="2"/><path d="M2 6h12"/>', 14),
  action: () => svg('<path d="M3.5 2.5L12.5 7L8.7 8.2L7 12z"/>', 14),
  warn: (s = 14) => svg('<path d="M8 2.2L14.5 13.5h-13z"/><path d="M8 6.5v3.2M8 11.6v.1"/>', s),
  external: () => svg('<path d="M9 2.5h4.5V7M13.5 2.5L7.5 8.5M12 9.5v3a1 1 0 0 1-1 1H3.5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h3"/>', 13),
  copy: () => svg('<rect x="5" y="5" width="8.5" height="8.5" rx="1.5"/><path d="M11 5V3.5a1 1 0 0 0-1-1H3.5a1 1 0 0 0-1 1V10a1 1 0 0 0 1 1H5"/>', 13),
  done: () => svg('<circle cx="8" cy="8" r="6.5"/><path d="M5.2 8.2l1.9 1.9 3.8-4"/>'),
  busy: () => svg('<circle cx="8" cy="8" r="6.5" opacity="0.25"/><path d="M8 1.5a6.5 6.5 0 0 1 6.5 6.5"/>', 16, ' class="spin"'),
  todo: () => svg('<circle cx="8" cy="8" r="6.5" stroke-dasharray="2 2.2"/>'),
  database: () => svg('<ellipse cx="8" cy="4" rx="5" ry="2"/><path d="M3 4v8c0 1.1 2.2 2 5 2s5-.9 5-2V4M3 8c0 1.1 2.2 2 5 2s5-.9 5-2"/>', 14),
  arrow: () => svg('<path d="M2.5 8h10M9 4.5L12.5 8L9 11.5"/>', 14),
  file: () => svg('<path d="M4 1.75h5l3 3v9.5H4z"/><path d="M9 1.75v3h3"/>', 14),
  box: () => svg('<rect x="2.5" y="4" width="11" height="8" rx="2"/>', 14),
  check: () => svg('<path d="M3 8.5l3.2 3.2L13 4.8"/>', 14),
  clock: () => svg('<circle cx="8" cy="8" r="5.75"/><path d="M8 4.75V8l2.25 1.5"/>', 14),
  power: () => svg('<path d="M8 2.25v5"/><path d="M4.9 4.4a5 5 0 1 0 6.2 0"/>', 14),
  mark: () =>
    `<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><rect x="0.5" y="0.5" width="15" height="15" rx="4" fill="currentColor"/><path d="M4.5 4v8M8 4v8M11.5 4v8" stroke="var(--side)" stroke-width="1" stroke-dasharray="1.2 1.6"/><path d="M4.5 7.2h6" stroke="var(--side)" stroke-width="1.4"/></svg>`,
  winMin: () => `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M0 5h10"/></svg>`,
  winMax: () => `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><rect x="0.5" y="0.5" width="9" height="9" rx="1.5"/></svg>`,
  winClose: () => `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M0.5 0.5l9 9M9.5 0.5l-9 9"/></svg>`,
};

/** The app icon, drawn small (same drawing as src-tauri/icons/icon.svg). */
export const APP_ICON = (size: number) =>
  `<svg class="app-icon" width="${size}" height="${size}" viewBox="90 90 844 844" aria-hidden="true">
  <path class="icon-body" d="M285 100H700L924 324V739A185 185 0 0 1 739 924H285A185 185 0 0 1 100 739V285A185 185 0 0 1 285 100Z" fill="#1C1C1F"/>
  <path d="M700 100L924 324H748A48 48 0 0 1 700 276Z" fill="#4A4A52"/>
  <rect x="225" y="370" width="150" height="64" rx="18" fill="#E8E8EC"/><rect x="437" y="370" width="150" height="64" rx="18" fill="#E8E8EC"/><rect x="649" y="370" width="150" height="64" rx="18" fill="#E8E8EC"/>
  <path d="M300 468V836M512 468V836M724 468V836" stroke="#6B6B74" stroke-width="12" stroke-dasharray="22 26"/>
  <path d="M300 556H684" stroke="#FFFFFF" stroke-width="26"/><path d="M668 512L736 556L668 600Z" fill="#FFFFFF"/>
  <path d="M724 712H316" stroke="#8DB2F0" stroke-width="22"/><path d="M362 672L306 712L362 752" fill="none" stroke="#8DB2F0" stroke-width="22" stroke-linejoin="round" stroke-linecap="round"/>
</svg>`;
