# CLAUDE.md

plainfold is a read-only desktop app that shows what a codebase does as stories (sequence diagrams) that lead down to the real lines. It reads code statically: it never runs the project's code, never writes to a project, and hosts no agent or terminal.

## Where things are

- `DECISIONS.md`: product decisions and their status. Read the section you touch before changing behavior.
- `TODO.md`: what is left, the next steps under **Next**. "What's next?" is answered from there.
- `core/`: the Rust core, the real code (story extraction, language servers, model, changes). Has its own `CLAUDE.md`.
- `app/`: the Tauri desktop app, shell (`src-tauri/`) and view (`src/`). Has its own `CLAUDE.md`.
- `spike/`: throwaway; it only supplies reference models, server executables and fixtures. Has its own `CLAUDE.md`.
- `samples/bookshelf`: the small sample project. `prototype/sequence.html`: the mockup of the Setup section (§5).
- `out/` (git-ignored): extracted models. The app's browser preview serves only files under the repository, so models go here.
- `README.md`: the public face of the repository. `demo/`: the browser demo, published to GitHub Pages by `.github/workflows/pages.yml` (the view built as static files, `demo/index.html`, the models in `demo/models`, `docs/screenshots`). The view reads those models as they are: after a change to the model's format, extract them again from the fixtures' commits in `core/scripts/fixtures.json`.

## The one rule: no overfitting

We want a general formula that works on every project, not a diagram that looks right for the project in front of us.

- Fixtures (`samples/`, `spike/universal/fixtures/`, any project the tool is tried on) only expose problems. They are never the thing being fixed.
- Never put a fixture's file name, folder, function, table, route, label text, library name or framework quirk into the extractor, the view or a heuristic. No `if name == …`, no special path, no constant tuned until one picture looks right.
- State the problem as a property of code or of the diagram ("a call that opens in place reads like the next sibling") and fix that property. Ask: is this still right in a project with a different language style, size, framework and naming?
- A fix counts only when checked on at least two unrelated projects (different domain and structure), or when it is a pure rendering rule that reads no project content. With one project, say so and mark it unverified.
- If the only way to make it look right is a special case, don't. Leave a visible gap ("not resolved yet"). An honest gap beats a hidden guess.
- Framework and vendor knowledge belongs in a small adapter that says what it handles. The core and the view stay framework-agnostic and still give a sensible diagram when no adapter matches.
- Examples in comments, docs, tests and commit messages use neutral names (`save`, `items`), never a fixture's identifiers.

## What a diagram claims (core and view keep this true)

- Order is source order, not runtime order: numbers mean "written in this order". What runs later (effects, callbacks, handlers) is its own labeled frame, never implied by position.
- A step is something that runs. A function or class written inside a body is declared, not run, so it is no step; storing one elsewhere (`handlers[key] = fn`) is.
- What sits under an opened arrow comes from the callee's definition; the arrow and its code block come from the call site. The panel says which is which.
- Code shown is the file as it was when the model was extracted.
- Every label is code text, comes from the language server, or is marked as written by a model and checked against the code. Nothing the code does not show is presented as fact.

## Documents stay current

- After a substantial change, and before saying the work is done, update what it affects:
  - this file and the nested `CLAUDE.md` files: only rules and commands that change how work is done here;
  - `DECISIONS.md`: mark what is now built (shorten it to the rule plus **Built**), rewrite decisions that changed;
  - `TODO.md`: delete what is finished, add what is left or was found on the way;
  - the `README` / `RESULTS` of every folder touched.
- Documents describe the current state. Rewrite or delete what is no longer true; history is in git.
- A decision is **Decided** only when the user explicitly agreed. Otherwise it is **Proposed** or **Open**.
- Nothing goes into a `CLAUDE.md` or a skill unless it changes what Claude does. What the code already says (module headers, command usage, file layout) stays in the code.
- Everything in the repository is in English (the user chats in Turkish). Docs, comments and commit messages use plain words in short sentences, like the existing ones.

## Working here

- Untracked work has no undo: ask the user to commit before large edits.
- Do not commit or push unless asked; when asked, use the `commit` skill.
- The repository is public: whatever is committed is published.
- Some fixtures are private projects. Never put their code or identifiers in commits, docs, artifacts, demo models or anything published.
- Verifying: a core change with `node core/scripts/check.mjs` (see `core/CLAUDE.md`); a view change in four looks with the `look` skill. Type-checking is not verification.
- The shell is zsh: `set -- $var` does not split on spaces; pass arguments explicitly (or `${=var}`).
