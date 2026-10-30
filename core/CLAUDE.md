# core/

The Rust library (`src/lib.rs`) that reads a project into the model JSON the view reads (`src/model.rs`), with a command line for development. The commands and their arguments are in the header of `src/main.rs`; each module's header says what it is for.

## Rules

- The walker, the resolver and the story builder know no language and no framework. Syntax lives only in `src/lang/<language>.rs`; a framework or vendor only in `src/adapters/<name>.rs`, which says at its top what it handles and is active only when the project imports a package it handles.
- Structure (steps, links, order, names) comes only from code text and the language server. The English layer (`src/english/`) adds text that passes `check.rs` and changes nothing else; without a key, nothing there runs.
- A language server's answers are used only once it is ready: never add a time limit that falls back to early answers. A language without its server shows no stories and says why; never fall back to a tree-sitter guess (DECISIONS §10).
- Never run the project's code: no server or binary from inside the project folder, no project scripts, no build steps.
- The same code always gives the same model (only `stats` holds times): no map iteration order or machine-specific paths in it. `diff`, the renames and `check.mjs` depend on it.
- What the core cannot follow stays visible as unresolved; it is never guessed. The index (`story/reach.rs`) is a lower bound and is described as one.

## Checking a change

1. `cargo build` and `cargo test` (tests sit in the modules); add no new clippy warnings.
2. Before committing, `node scripts/check.mjs` (about two minutes the first time, under one after). It reads every fixture in `scripts/fixtures.json` with the last commit's core and with yours and prints the stories that differ. Read every difference. One you did not mean, in a project you were not working on, is the thing to look for. Tell the user what changed in each project.
3. A fixture that is missing is printed with where it comes from. A new fixture goes into `scripts/fixtures.json` with its repository and commit, cloned shallow into `spike/universal/fixtures/`.
4. Changes to kept language servers: `again` reads, runs an edit and reads again with the servers kept; compare the second model with a fresh `extract`.

## Running it

- From the repository root, after `cargo build` here: `core/target/debug/plainfold-core extract <project> out/<name>.json`, then open the model in the app preview (`look` skill).
- Language servers for development: `PLAINFOLD_SERVERS=<repo>/spike/universal/node_modules/.bin` (absolute) and `PLAINFOLD_TSSERVER=<repo>/spike/skeleton/node_modules/typescript/lib` for a project with no TypeScript of its own. Python needs `--python <interpreter>` when its environment is outside the project (the fixtures' environments are in `scripts/fixtures.json`). Dart needs `dart` on PATH and `flutter pub get` run in the project.
- `node scripts/story.mjs <model.json> <part of a story id>` prints stories as indented text: the way to read what a change did without the app.
- `PLAINFOLD_LSP_DEBUG=1` prints the server traffic.
- `english` sends code to a paid API with the user's key: ask before running it on more than one small project. `--show <n>` prints a request without sending it.
- `node scripts/compare.mjs <prototype model> <core model>` compares with the TypeScript prototype's models (`spike/skeleton`).
