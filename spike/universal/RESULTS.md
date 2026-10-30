# Universal walker spike: results (2026-10-02)

**Status.** Done and ported. The walker, the LSP client and the hop-following now live in the Rust core (`core/`), together with the story logic this spike did not test. This folder is kept for two things: the language server executables (`node_modules/.bin`) and the fixtures the core is checked against (`fixtures/`, listed in `core/scripts/fixtures.json`). The measurements below are the evidence behind `DECISIONS.md` §10.

**Question.** Can one language-neutral walker on tree-sitter, with calls resolved through each language's LSP server, be as accurate as a per-language analyzer?

**Answer.** Yes, for what was measured: TypeScript and Python, on four unrelated real projects. Resolving the project's own code loses nothing compared with the compiler. The remaining gaps are framework rules and projects whose dependencies are not installed, and the second architecture would have the same gaps.

## TypeScript: measured against the compiler

The oracle is the TypeScript compiler (`checker.getSymbolAtLocation` through imports, the same engine as the skeleton). The server is typescript-language-server pinned to the same TypeScript version, so only the mechanism differs.

| | bookshelf (sample, 8 files) | an Expo app (private, 194 files) |
|---|---|---|
| Call sites found: tree-sitter vs compiler | 50 / 50, same positions | 7,382 / 7,382, same positions |
| Same declaration as the compiler | 29 / 29 resolved | 5,326 identical |
| Different place | 0 | 606, **all library calls**: the same function reached as its `.d.ts` (compiler) or its source through declaration maps (server). **0 in project code** |
| Server resolves, compiler does not | 0 | 1,449, all `jest` globals in test files (the oracle's program did not load `@types/jest`) |
| Arrows the skeleton draws (M2) | 4 / 4 in one hop | 162: 114 in one hop, +39 by following returned values = **153 (94%)**. The 9 left are all `ref.current()` through React `useRef`, which the skeleton resolves with a React-specific rule |
| Time to resolve every call site | 0.3 s | 4.6 s for 9,340 sites |

## Python: two independent engines

Python has no compiler to use as an oracle. pyright (through LSP) and Jedi (run directly) answer the same question for every call site. Each project is measured twice: with its dependencies installed, and in an environment without them (a user who has not set the project up yet).

| App code (tests separate) | FastAPI template (typed, 40 files) | Flask tutorial (untyped, 9 files) |
|---|---|---|
| Call sites | 451 | 94 |
| pyright, dependencies installed | **451 / 451 resolved** (63 own, 360 library, 28 stdlib) | **90 / 94** (4 unresolved, all untyped parameters or untyped values) |
| Jedi, dependencies installed | 443 / 451 | 74 / 94 |
| pyright, no dependencies | 91 / 451 (every library call lost; own code unaffected) | 38 / 94 |
| Own-code answers: pyright = Jedi | 221 / 221 (app + tests), with and without dependencies | 27 / 27 |
| Own-code answer lands on the called name | 221 / 221 | 27 / 27 |

In test code, pyright cannot see through untyped pytest fixtures. One installed library (starlette) conditionally imports a package that is not installed, which hides `TestClient`'s methods from pyright but not from Jedi. Jedi infers untyped parameters from their callers, which pyright does not do.

The same walker, unchanged, produces the same shape for a Python endpoint as for a React handler: calls in order, branches and loops as frames, each call resolved (`DEMO=<function name>`).

## What the core took from it

| Finding | In `core/` |
|---|---|
| Early answers are silently weaker (the TypeScript server stops at the `import` before the project is loaded; pyright returns no library targets before applying the environment) | Built: the core waits for each server's progress reports, then checks that repeated answers have stopped changing. Answers before readiness are never exact |
| Without dependencies, 80% of the FastAPI app's calls lose their target, while own code still resolves | Built: a warning listing the packages that are not installed |
| A `.d.ts` / `.pyi` stub and the source of the same function are both correct | Decided, not built |
| Framework rules stay adapters (React `useRef`: the 9 arrows above) | Built: the React adapter; the core reproduces all 162 arrows |
| A second engine (Jedi) answers where pyright stops (untyped parameters) | Decided, not built |

Still not measured: Go, Java, C# and the other languages, and a human check of a sample of answers.

## Reproduce

```
npm install --ignore-scripts
node src/measure-ts.mjs ../../samples/bookshelf ../skeleton/out/bookshelf.model.json
DEMO=<function> node src/measure-ts.mjs <expo app> <its skeleton model.json>
DEMO=create_user node src/measure-py.mjs fixtures/full-stack-fastapi-template/backend fixtures/venv-fastapi/bin/python deps
```

The fixtures are shallow clones: full-stack-fastapi-template `cb740b6`, flask `d73fa1c`. The environments were built with `uv` (Python 3.14). Results are written to `out/`.

`fixtures/suno2openai` and `fixtures/podcast-ranking` are the projects the core's schedules are checked against, each with its own environment (`venv-suno`, `venv-podcast`; the packages are listed in `core/scripts/fixtures.json`).

`fixtures/compass_app` is a Flutter app the core's Dart support is checked against (not measured by this spike): flutter/samples' `compass_app`, copied out of that repository's pub workspace, with `resolution: workspace` and the repository's own lint package removed from its two `pubspec.yaml` files so that `flutter pub get` (in `app/`) and `dart pub get` (in `server/`) run on their own.
