// The diagram's viewport: a fixed area over a faint grid, in which the diagram moves and zooms. A wheel, two fingers
// or a drag from anywhere move it; a pinch or ⌘/Ctrl with the wheel zooms toward the pointer. The column boxes stay
// at the top while the rows move under them, and the step numbers at the left edge, each beside its row. It keeps its element between stories, so where it is looking survives a
// redraw; only a new story or "default view" sets it again.
import { ICON } from "./ui";

/** A story opens whole when that keeps it at least this large; otherwise it fits the width, not below this. */
const MIN_FIT = 0.85;
const Z_MIN = 0.3, Z_MAX = 2;
/** Room kept between the diagram and the viewport's edges, on screen. */
const M = 24;
/** The grid's spacing at 100%; zoomed out, it doubles rather than gets dense. */
const GRID = 24;
/** How long a move or a zoom the user did not drag takes, and how long rows take to slide when a call opens. */
const EASE_MS = 180, SLIDE_MS = 160;

/** One drawn story: the SVG, the column boxes on their own for the top, the step numbers on their own for the left,
 *  their sizes, and where each row starts. */
export interface Sheet {
  svg: string;
  head: string;
  gut: string;
  gutW: number;
  /** where the first row starts: the strip behind the numbers runs from there to the end */
  gutTop: number;
  W: number;
  H: number;
  headH: number;
  /** the top of each element that keeps its key between drawings (`data-r`), for the slide when a call opens */
  ys: Map<string, number>;
}

const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

export class Canvas {
  readonly el: HTMLElement;
  z = 1;
  x = 0;
  y = 0;
  /** the user moved or zoomed since the default view */
  moved = false;
  private sheet: HTMLElement;
  private head: HTMLElement;
  private gut: HTMLElement;
  private strip: HTMLElement;
  private gutW = 0;
  private gutTop = 0;
  private pct: HTMLElement;
  private W = 1;
  private H = 1;
  private headH = 0;
  private anim = 0;
  /** the default view waits until the viewport has a size */
  private homeLater = false;

  constructor() {
    this.el = document.createElement("div");
    this.el.className = "dia";
    this.el.innerHTML =
      `<div class="sheet"></div><div class="gstrip"></div><div class="dgut"></div><div class="dhead" hidden></div>` +
      `<div class="dzoom"><button type="button" data-act="zout" aria-label="Zoom out">${ICON.minus()}</button>` +
      `<button type="button" class="zp" data-act="zfit" title="Default view"></button>` +
      `<button type="button" data-act="zin" aria-label="Zoom in">${ICON.plus()}</button></div>`;
    [this.sheet, this.strip, this.gut, this.head, this.pct] = [this.el.querySelector(".sheet")!, this.el.querySelector(".gstrip")!, this.el.querySelector(".dgut")!, this.el.querySelector(".dhead")!, this.el.querySelector(".zp")!];
    this.wire();
    new ResizeObserver(() => {
      if (!this.el.clientWidth) return;
      if (this.homeLater || !this.moved) this.home();
      else this.place();
    }).observe(this.el);
  }

  /** Puts a drawing in. Where the view looks stays; `prev` (the row tops of the drawing before) makes rows that moved
   *  slide from where they were and new ones fade in, as when a call opens. */
  set(s: Sheet, prev?: Map<string, number>): void {
    [this.W, this.H, this.headH, this.gutW, this.gutTop] = [s.W, s.H, s.headH, s.gutW, s.gutTop];
    this.sheet.innerHTML = s.svg;
    this.head.innerHTML = `<svg class="seq" width="${s.W}" height="${s.headH}" viewBox="0 0 ${s.W} ${s.headH}" aria-hidden="true">${s.head.replace(/tabindex="0"/g, 'tabindex="-1"')}</svg>`;
    this.gut.innerHTML = `<svg class="seq" width="${s.gutW}" height="${s.H}" viewBox="0 0 ${s.gutW} ${s.H}" aria-hidden="true">${s.gut}</svg>`;
    this.place();
    if (prev && !still()) this.slide(prev, s.ys);
  }

  /** The view a story opens at: whole when it fits at a readable size, else as wide as the viewport (not below
   *  MIN_FIT) from the top; centred across when narrower. */
  home(ease = false): void {
    const vw = this.el.clientWidth, vh = this.el.clientHeight;
    if (!vw || !vh) {
      this.homeLater = true;
      return;
    }
    this.homeLater = false;
    const all = Math.min((vw - 2 * M) / this.W, (vh - 2 * M) / this.H);
    const z = all >= MIN_FIT ? Math.min(1, all) : Math.max(MIN_FIT, Math.min(1, (vw - 2 * M) / this.W));
    const x = this.W * z <= vw - 2 * M ? (vw - this.W * z) / 2 : M;
    this.moved = false;
    this.go(z, x, M, ease);
  }

  /** Zooms by `f` around the middle of the viewport. */
  zoomBy(f: number): void {
    this.moved = true;
    const z = clamp(this.z * f, Z_MIN, Z_MAX), k = z / this.z, cx = this.el.clientWidth / 2, cy = this.el.clientHeight / 2;
    this.go(z, cx - (cx - this.x) * k, cy - (cy - this.y) * k, true);
  }

  /** Moves the diagram across by `dx` of its own units at once, as when the columns moved under what was clicked:
   *  that stays where it is on screen. Not a move of the user's. */
  hold(dx: number): void {
    if (!dx) return;
    this.x += dx * this.z;
    this.place();
  }

  panBy(dx: number, dy: number): void {
    this.moved = true;
    this.go(this.z, this.x + dx, this.y + dy, true);
  }

  /** Brings a box of the diagram (its own units) into view, if any of it is outside; across only when it must. */
  reveal(b: { x: number; y: number; width: number; height: number }, ease = true): void {
    const vw = this.el.clientWidth, vh = this.el.clientHeight, z = this.z, top = this.headShown(this.y) ? this.headH * z : 0;
    let { x, y } = this;
    if (b.y * z + y < top + M || (b.y + b.height) * z + y > vh - M) y = top + (vh - top) / 3 - b.y * z;
    if ((b.x + b.width) * z + x < M || b.x * z + x > vw - M) x = 2 * M - b.x * z;
    if (x !== this.x || y !== this.y) this.go(z, x, y, ease);
  }

  // ---------- where it looks ----------
  private go(z: number, x: number, y: number, ease: boolean): void {
    cancelAnimationFrame(this.anim);
    [z, x, y] = this.bound(z, x, y);
    if (!ease || still()) {
      [this.z, this.x, this.y] = [z, x, y];
      return this.place();
    }
    const from = [this.z, this.x, this.y], t0 = performance.now();
    const step = (t: number) => {
      const p = Math.min(1, (t - t0) / EASE_MS), e = 1 - (1 - p) ** 3;
      [this.z, this.x, this.y] = [from[0] + (z - from[0]) * e, from[1] + (x - from[1]) * e, from[2] + (y - from[2]) * e];
      this.place(false);
      if (p < 1) this.anim = requestAnimationFrame(step);
    };
    this.anim = requestAnimationFrame(step);
  }

  /** Keeps some of the diagram in view: a side larger than the viewport can move until its edge leaves M of room; a
   *  smaller one moves inside the viewport. */
  private bound(z: number, x: number, y: number): [number, number, number] {
    const lim = (p: number, c: number, v: number) => (c + 2 * M <= v ? clamp(p, M, v - c - M) : clamp(p, v - c - M, M));
    return [z, lim(x, this.W * z, this.el.clientWidth), lim(y, this.H * z, this.el.clientHeight)];
  }

  private headShown = (y: number) => y < 0;

  private place(bound = true): void {
    if (bound) [this.z, this.x, this.y] = this.bound(this.z, this.x, this.y);
    const { z, x, y } = this;
    this.sheet.style.transform = `translate(${x}px, ${y}px) scale(${z})`;
    this.head.hidden = !this.headShown(y);
    this.head.style.height = `${this.headH * z}px`;
    (this.head.firstElementChild as HTMLElement | null)?.style.setProperty("transform", `translate(${x}px, 0) scale(${z})`);
    // the numbers move with the rows, and across only until they reach the left edge; there, one strip from the first
    // row to the last is behind them
    (this.gut.firstElementChild as HTMLElement | null)?.style.setProperty("transform", `translate(${Math.max(0, x)}px, ${y}px) scale(${z})`);
    this.el.classList.toggle("across", x < 0);
    if (x < 0) {
      this.strip.style.transform = `translate(0, ${y + this.gutTop * z}px)`;
      this.strip.style.width = `${this.gutW * z}px`;
      this.strip.style.height = `${(this.H - this.gutTop) * z}px`;
    }
    let g = GRID * z;
    while (g < 14) g *= 2;
    this.el.style.backgroundSize = `${g}px ${g}px`;
    this.el.style.backgroundPosition = `${x}px ${y}px`;
    this.pct.textContent = `${Math.round(z * 100)}%`;
  }

  // ---------- rows that slide ----------
  private slide(prev: Map<string, number>, ys: Map<string, number>): void {
    for (const e of this.el.querySelectorAll<SVGElement>(".sheet [data-r], .dgut [data-r]")) {
      const k = e.dataset.r!, a = prev.get(k), b = ys.get(k);
      if (a === undefined) e.animate([{ opacity: 0 }, { opacity: 1 }], { duration: SLIDE_MS, easing: "ease-out" });
      else if (b !== undefined && a !== b) e.animate([{ transform: `translateY(${a - b}px)` }, { transform: "none" }], { duration: SLIDE_MS, easing: "ease-out" });
    }
  }

  // ---------- input ----------
  private wire(): void {
    const el = this.el;
    // a wheel or two fingers move the diagram, along a side only when it does not fit on it; a pinch (a wheel with
    // Ctrl in Chromium) or ⌘/Ctrl with the wheel zooms toward the pointer, a mouse notch by a bounded step
    el.addEventListener("wheel", (e) => {
      e.preventDefault();
      const r = el.getBoundingClientRect(), unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? r.height : 1;
      const dx = e.deltaX * unit, dy = e.deltaY * unit;
      cancelAnimationFrame(this.anim);
      this.moved = true;
      if (e.ctrlKey || e.metaKey) return this.zoomAt(Math.exp(-clamp(dy, -15, 15) * 0.01), e.clientX - r.left, e.clientY - r.top);
      if (this.W * this.z + 2 * M > r.width) this.x -= dx;
      if (this.H * this.z + 2 * M > r.height) this.y -= dy;
      this.place();
    }, { passive: false });
    // a pinch in WebKit comes as a gesture, not as a wheel
    let g0 = 1;
    type Gesture = Event & { scale: number; clientX: number; clientY: number };
    el.addEventListener("gesturestart", (e) => { e.preventDefault(); g0 = this.z; });
    el.addEventListener("gesturechange", (e) => {
      e.preventDefault();
      const g = e as Gesture, r = el.getBoundingClientRect();
      this.moved = true;
      this.zoomAt((g0 * g.scale) / this.z, g.clientX - r.left, g.clientY - r.top);
    });
    // a drag from anywhere moves the diagram; a press that does not move is a click
    let start: { x: number; y: number; px: number; py: number; id: number } | null = null, dragged = false;
    el.addEventListener("pointerdown", (e) => {
      dragged = false;
      if (e.button !== 0 || (e.target as Element).closest(".dzoom")) return;
      start = { x: e.clientX, y: e.clientY, px: this.x, py: this.y, id: e.pointerId };
    });
    el.addEventListener("pointermove", (e) => {
      if (!start) return;
      const dx = e.clientX - start.x, dy = e.clientY - start.y;
      if (!dragged && Math.hypot(dx, dy) < 4) return;
      if (!dragged) {
        dragged = true;
        el.setPointerCapture(start.id);
        el.classList.add("grabbing");
        cancelAnimationFrame(this.anim);
      }
      this.moved = true;
      [this.x, this.y] = [start.px + dx, start.py + dy];
      this.place();
    });
    const end = () => {
      start = null;
      el.classList.remove("grabbing");
    };
    el.addEventListener("pointerup", end);
    el.addEventListener("pointercancel", end);
    el.addEventListener("click", (e) => {
      if (!dragged) return;
      dragged = false;
      e.stopPropagation();
    }, true);
    // a step reached with Tab is brought into view (a click focuses what is already in view)
    el.addEventListener("focusin", (e) => {
      const t = e.target as Element;
      if (t instanceof SVGGraphicsElement && this.sheet.contains(t) && t.matches(":focus-visible")) this.reveal(t.getBBox(), false);
    });
    // focusing a step must not scroll the viewport itself: it only moves by its transform
    el.addEventListener("scroll", () => el.scrollTo(0, 0));
  }

  private zoomAt(f: number, cx: number, cy: number): void {
    const z = clamp(this.z * f, Z_MIN, Z_MAX), k = z / this.z;
    [this.z, this.x, this.y] = [z, cx - (cx - this.x) * k, cy - (cy - this.y) * k];
    this.place();
  }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
