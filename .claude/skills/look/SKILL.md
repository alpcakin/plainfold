---
name: look
description: See the plainfold app as the user would. Takes screenshots of the browser preview in the four looks (light and dark, macOS and Windows) and of the real app window. Use after any change to app/src or app/src-tauri, and when the user asks to run, open, show or screenshot the app or to open a project in it.
---

# Look at the app

Type-checking is not verification. After a view change, look at the part you changed in all four looks, then at what surrounds it. Say what you saw, including what looks wrong.

## The four looks (browser preview)

1. Get a model under `out/`: one already there, or `core/target/debug/plainfold-core extract <project> out/<name>.json` (settings in `core/CLAUDE.md`).
2. Start the preview in the background and leave it running: `npm run dev` in `app/` (port 1420).
3. Run `.claude/skills/look/shots.sh out/<name>.json <scratchpad dir> [key=value …]`. It writes `mac-light.png`, `mac-dark.png`, `windows-light.png` and `windows-dark.png` in about five seconds.
   - Pass values as written; the script URL-encodes them: `story='action:/login#submit' level=parts`.
   - The parameters are listed in `app/README.md` under "Browser preview". Story ids are in the model's `stories[].id`.
   - `LOOKS="mac-dark windows-light"` takes only those looks. `W=… H=…` sets the window size (default 1440×900).
4. Read every image.

Headless Chrome cannot click. Put the state you need in the URL. If no parameter reaches that state, add one to the start section at the end of `app/src/main.ts`, next to the others. To show the user an interaction, use Claude in Chrome on `http://localhost:1420/?preview=…` if they want it.

## The real window (macOS only; Windows has never been built)

1. Start the app in the background from `app/`: `npm run tauri dev -- -- -- <project folder>`. The first build takes minutes. Set the server variables from `core/CLAUDE.md` when the project needs development servers.
2. Run `.claude/skills/look/window.sh <out.png>`. It waits for the window, then captures that window only. Capturing the whole screen is not allowed.
3. If you opened the app for your own check, stop it afterwards; its language servers hold about 1 GB each. If the user wants to try it, leave it running and tell them what to look at.
