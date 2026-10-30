# spike/

Throwaway. Nothing new is built here; new work goes into `core/` or `app/`. It stays only for what development still uses:

- `skeleton/`: the TypeScript prototype. It supplies reference models for `core/scripts/compare.mjs` (`node spike/skeleton/src/extract.ts <project> <model.json>`) and the TypeScript in `node_modules/typescript/lib` used as `PLAINFOLD_TSSERVER`. Its one-page view (`src/view.html`, `src/render.ts`) was replaced by `app/src`: do not develop it.
- `universal/`: the language server executables (`npm install --ignore-scripts`, then `node_modules/.bin`) and the fixtures in `fixtures/` (git-ignored; how `compass_app` was set up is in `RESULTS.md`). `RESULTS.md` holds the measurements behind DECISIONS §10.
