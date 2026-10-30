# plainfold desktop app

The Tauri 2 shell around the Rust core (`../core`), with the story view as its interface. How it behaves is decided in `../DECISIONS.md`.

## Layout

- `src-tauri/src/main.rs`: the shell. It reads a project with the core on a background thread (`open_project`, with `progress` events), keeps the window's language servers between reads, watches the project for changed files (`changes` events), downloads servers into the app data folder (`servers/`) after asking, and opens files in Zed, Cursor or VS Code (the first one installed). Recent projects and settings live in `state.json` in the app data folder (`~/Library/Application Support/dev.plainfold.desktop` on macOS). An app started from the Finder copies PATH from the login shell, so servers and Node are found.
- `src/`: the view.
  - `main.ts`: the windows (welcome, reading, story view), shortcuts and the browser preview's parameters
  - `model.ts`: the model's types and how stories are named
  - `places.ts`: the story list as the places a person meets: screens in the order they are reached, their controls
  - `diagram.ts`: folding, layout and the SVG of one story
  - `canvas.ts`: the diagram's viewport: zoom, moving, the default view, rows that slide
  - `changes.ts`: a story with what changed in it, ready for the diagram
  - `inspector.ts`, `code.ts`: the details panel and its code blocks
  - `english.ts`: the steps in plain English, looked up per step
  - `palette.ts`: Go to (⌘K)
  - `backend.ts`: calls into the shell, or stand-ins when the page runs in a plain browser
  - `styles.css`: colors for light and dark, platform differences (`html.mac`, `html.windows`)

## Run

```
npm install
npm run tauri dev                        # the app, welcome window first
npm run tauri dev -- -- -- <folder>      # opens a project folder directly
npm run check                            # type-checks the view
npm run tauri build -- --bundles app     # src-tauri/target/release/bundle/macos/plainfold.app
```

`PLAINFOLD_SERVERS` and `PLAINFOLD_TSSERVER` point at other language servers (`../core/CLAUDE.md`); `PLAINFOLD_KEEP_IDLE_SECS=<n>` shortens the 20 minutes after which an unused window's servers stop.

## Browser preview

`npm run dev`, then `http://localhost:1420/?preview=/@fs/<absolute path to a model.json>`. Vite serves only files under the repository, so put models in `out/`. The public demo is this preview built as static files (`../demo/`), where `preview` is a URL relative to the page. Parameters:

- `&platform=mac|windows`
- `&story=<story id>` (URL-encoded), `&level=parts|functions`, `&open=all` (every call opened), `&select=<step id>`
- `&screen=welcome|opening`, `&settings=1|closed|off`, `&ask=1|node` (the question before a download)
- `&palette=<query>`, `&focus=<index id>` (the list narrowed to what reaches it), `&tab=data|tests|setup`
- `&changed=<file>,<file>` (the toolbar's "files changed · Reload"), `&diff=/@fs/<absolute path>` (the output of `plainfold-core diff <old> <new> --json`)
- `&english=/@fs/<absolute path>` (the output of `plainfold-core english`), `&key=none`

## Platforms

- **macOS:** built and run. Overlay title bar, traffic lights inside the sidebar.
- **Windows:** written, never built or run. `tauri.windows.conf.json` turns off the system title bar; the view draws its own, with caption buttons, Segoe UI and the Windows accent.
- **Linux:** the system title bar; not tried.

## Shortcuts

⌘K go to · ⌘O open folder · ⌘F filter stories · ⌘[ / ⌘] back and forward · ⌘R read the project again · ⌘+ / ⌘− zoom, ⌘0 the default view · arrow keys move the diagram (Shift: further) · ⌘, settings. On Windows and Linux, Ctrl instead of ⌘. In the diagram, the wheel, two fingers or a drag move it; a pinch or ⌘ with the wheel zooms.
