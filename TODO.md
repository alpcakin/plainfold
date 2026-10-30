# TODO

What is left. Work has stopped (`DECISIONS.md` §0); this is what was open then. The reasons are in `DECISIONS.md` (section numbers in brackets). Add work as soon as it comes up. Delete an item when it is done, and mark its decision **Built**. Keep **Next** to a few items.

## Next

- Windows: build and run it.
- What the code works out [§4]: opening a calculation step shows its code and the tests that reach it, with what they give it and expect back.
- Rules [§4, §6]: a rule added, removed or changed is a structural change signal ("a removed login check"). Rules are checked on one project per language; look at them on a Next.js app with server actions.

## Story extraction (core)

- Requests through an HTTP client library [§10]: a call named by its method is linked, inferred, only when its own URL is a route's path. Not yet: a path relative to a base the client is made with (`axios.create({ baseURL: "/api" })`, then `api.get("/items")`), a request whose method is in its options (`axios({ method, url })`), and exact links for a library known to send requests. Seen on one project; a second with axios is needed.
- TanStack Query: a mutation made in another file (a project's own hook around `useMutation`) is not followed, and a query's `enabled` (it may never run) is not shown. A `handleSubmit` from a project's own hook around `useForm` is not followed either. What runs when a mutation ends, and what a query loads, are seen on one project.
- Shared state at the top of a file (`_cache[key] = v`, a client made once) is not shown as changed [§4]: a thing in the Data tab with the stories that write and read it.
- A method of the language's own library called on a value the language server knows no type of (an untyped Python parameter: `items.append(x)`) is not shown as a change: only what it can place is. Writing into it (`items[k] = v`) is.
- A package that throws on an error answer (axios on a 4xx) is not followed: the call that sends the request does not say it can end with it. A catch-all rethrow passes on only what is named in the `try`.
- Two stories of one place that read the same and share a handler, but whose steps differ, are told apart by their line. The condition that chooses between the controls would be a better name.
- A list taken apart by place is followed in TypeScript and Python, not in Dart's records.
- A function a caller gives as an argument (`useChat({ upload })`, then `upload()` inside) is not followed, except where a project function made the one that calls it (`save = guard(check, f)`). Follow it to what each caller passes, as alternatives (the Dart field item below is the same).
- A function only the run decides (one the caller gives, one kept in a field or a list, a loop's variable) is said nowhere. Say it once per story, not once per call: as a gap it was over a hundred lines in a library.
- One story for triggers with the same steps is seen on one Flutter app only. Check it on a React form that saves from a button and from Enter. Controls that hand the function other values (`select(null)` and `select(item)`) are one story too, since the steps do not show the value; the name then comes from one control's words ("All"), which tells only that one.
- A page's story does not say which layouts are drawn around it: a reference to each layout's story would.
- The places a person meets [§3]:
  - Screens nothing found opens come last (six in one Expo app): URLs only known at runtime, deep links, notifications. Follow a URL held in a variable to what it is given.
  - A tab of a tab bar is a reference from its layout, not a control named by its title (`options.title`). Routes of a tab folder that the layout does not list are not followed; `Drawer` is seen on no project.
  - A default export handed to the project's own wrapper (`export default withAuth(Screen)`) gives no Expo Router screen: what the wrapper does around it is not followed.
  - Flutter: a widget a screen opens (`Navigator.push`, a sheet's or a dialog's builder) is a screen only where its opening crosses a boundary; the list puts its controls under the screen that opens it. As `Route.screen` it would be a story of its own.
  - Controls not found on a screen in one app: a dialog a parent shows by its state, a toast. What draws them is not followed.
  - The message a person sends in a chat app goes to its endpoint through the AI SDK's `useChat` (`/api/chat` by default): not linked. An adapter for `ai` would.
- Entry points without an adapter [§4]:
  - A whole module nothing imports keeps its functions as entry points (it may be a tool's), though in an app it is most often unused code.
  - What the project keeps as data (a list of models, of providers) is in no story; the Setup section could list it.
  - Handed functions are shown only when the callee's signature takes a function. A call into an untyped library with no adapter, and unnamed functions written in the call (`setTimeout(() => …)`), are not.
  - A script that calls only libraries shows nothing.
  - Top-level code that does something (a call on its own) becomes a story of its file; one that only makes values does not. A route registered by a router library with no adapter (TanStack Router's `createFileRoute`) is no screen yet.
  - An override whose base method is never called is an entry of its own.
  - Subclass versions are not checked for classes outside the project. A call through an interface whose versions are objects (not classes) runs none of them: `model.load()` with each model written as `{ load: … }`.
- What starts by itself [§4]:
  - Schedules: only APScheduler. Others would be adapters of their own: node-cron, Celery beat, a cron in `vercel.json` (SQL schedules are under Supabase).
  - An APScheduler job named by a string (`"module:func"`) or written in the call (a `lambda`) is not followed: the call stays a plain arrow into the package.
  - `HTTPException` raised in code a schedule runs is named by its status (`500`), though no request gets that answer. Name it by its status only inside an endpoint's story.
  - The place of a function handed over inside another one is checked on two FastAPI projects. Check it on a browser or device project (a handler given to `addEventListener` inside a component).
- Packages that are not installed [§5]:
  - A package imported under another name than the one it is installed as (`import jose`, installed as `python-jose`) is found in no manifest. Its manifest name is known only once it is installed.
  - Not read: `Pipfile`, `setup.cfg`, a Dart pub workspace. No command for a `setup.py` (installing runs it), a `package.json` without a lockfile, or a Python environment outside the project.
  - A name the server cannot follow because its package is missing is not said in the story where it reads as plain code (see below).
- Calls into packages [§4]:
  - A method a package inherits from another names the defining package (`app.add_middleware` → starlette).
  - A name a project module takes from a package that is not installed is traced to it only within one file: a re-export, a value made in another module, or one read off such a value first (`result.items.map()`) read as plain code.
  - A chain that goes into a package and comes back to the project's own code (`useStore.getState().save()` with zustand) is one arrow into the package.
  - The language's own library is never drawn, so file and process calls (`open()`, `fs`, Dart's `HttpClient`) cross no boundary. Decide whether they should.
  - Side work is decided per project function, not per package method, so `Sentry.captureException` in every failure branch is still an arrow (`session.commit()` must stay one). Hooks from packages are arrows too.
- FastAPI [§10]:
  - `with` items are not walked, so a dependency that only opens and yields a resource shows nothing.
  - The dependencies given to a router or the app (`APIRouter(dependencies=…)`) are not walked.
  - `Depends(instance)` runs its `__call__`; this is not shown.
  - `background_tasks.add_task(f)` is missing; it should be a frame that runs later.
  - A route's `status_code` and `response_model` are not shown on its returns.
  - An async test client (`httpx.AsyncClient(transport=ASGITransport(app=app))`) is not linked to the endpoint.
- Adapters as data, over four general patterns, so a new framework is mostly a table: routing by file path (Next.js, Expo Router, SvelteKit, Nuxt, Remix), by decorator (FastAPI, Spring, NestJS, ASP.NET), by a registration call (Express, Flask, Go, Laravel), and interface events (`onClick`, Vue `@click`, Flutter `onPressed`) [§10].
- Download recipes for the next languages: gopls, rust-analyzer, Eclipse JDT LS, a C# server, Astro. Non-npm servers use the one-file-per-platform download Node.js uses (`servers::Program`) [§10].
- Downloaded Node.js [§10]:
  - `.tar.xz` and `.zip` would halve the download but need two more unpacking libraries.
  - Its pinned version is moved by hand (`servers::NODE`) for a security release.
  - On musl Linux (Alpine) it cannot start: check that the reason shown says so.
  - The Windows path (`node.exe`) has never run.
- A stub and its source count as the same target [§10].
- A second resolver (Jedi) for untyped Python, shown as inferred links [§10].
- Deno language server for Supabase edge function bodies [§10, §12]. Today tsserver reads them and does not follow `jsr:` or URL imports; a Supabase client made from one, or a value of its type, is still known.
- Supabase: tables from migrations, Row Level Security in plain English, triggers and scheduled jobs from SQL, `Deno.env.get` secrets [§12].
- Inferred links: read config, `.env` and dependency-injection registrations first [§10].
- A URL the project writes to one of its own routes (a link in an email, a form's `action`, a redirect) is not linked. It could be a reference to that endpoint's story [§10].
- Later adapters: Next.js `pages/`, React Router, TanStack Router, Django, ASP.NET, .NET services, SQLModel / SQLAlchemy, Flutter Bloc, XAML, Celery [§10].
- Story order: recently changed first, then risky ones [§4].
- aiosql: a query called as a method of an object `aiosql.from_path(...)` makes (`queries.get_user(conn, …)`) is a gap now; the SQL files name the table it reads or writes, so an adapter could draw it [§10].
- Riverpod: only a provider written with a function inside it (`FutureProvider((ref) { … })`) runs at its first read; `NotifierProvider(Todos.new)` (its `build`) and the code-generated `@riverpod` providers are not followed yet.
- SWR: `useSWRMutation`, `useSWRInfinite`, a fetcher set once for the app, a key that is a function. TanStack Router (`createFileRoute`) gives no screens: a story for each route file would need an adapter.
- Alembic: a migration's `upgrade` and `downgrade` are stories of their own; they could be told as migrations, named by their revision.
- A Next.js middleware written as a call of the project's own (not of a package) around a function is not found.

## Dart and Flutter [§10]

- A function kept in a field and called through it is not followed (`Command0(this._action)`, then `_action()`). This is why a command-object app's buttons stop at `execute`. Follow a field set from a constructor parameter to what each construction passes, as alternatives.
- go_router: link `go` / `push` to the widget each `GoRoute` builder makes. `Navigator.pushNamed` with a route table is not linked either.
- A shelf adapter would give a Dart server's routes. Its client (`HttpClient`) waits on the language-library decision above.
- Riverpod and provider are each checked on one project, and so is a builder walked quietly. `ref.listen(p, (prev, next) { … })` is not walked.
- Every widget is a place: check that a large app's list stays readable and that `IndexedStack` screens are not too deep.
- A `switch` expression's arms are walked in a row, not as alternatives. The value a `switch` statement tests is not walked.
- A text field's `InputDecoration(labelText: …)` is not used for its words.
- The Dart analysis server runs the analyzer plugins in `analysis_options.yaml`: part of restricted mode [§9].
- Test doubles outside `test/` count as versions for calls outside tests.

## What uses what [§3]

- Its own interface. Next on the same index: a stories × tables matrix, and links between stories through data ("A writes `t`, B reads it").
- A narrowed list does not survive a rename of what it is narrowed to (follow `renames.rs`, say "no longer found"). Back and forward do not remember it.
- A use inside a folded row opens the story at that row without marking where inside.
- The index is a lower bound. Missing from it:
  - unresolved calls (count them per name: "+3 unresolved calls with this name");
  - functions a package runs later (`app.listen(port, () => …)`);
  - package calls made while a screen draws;
  - entry points that became no story.
- Ids:
  - The same name twice in one place is told apart by order (`~2`), so adding one before it shifts the other.
  - An unnamed function's id changes with the words of its call.
  - Whatever keeps an id across readings must follow renames.
- Tables:
  - `table` holds a table or a database function, told apart only by the effect.
  - There is one database participant per project.
  - A Supabase query is drawn where it is built, not where it runs.
  - supabase-py needs the package installed.
- Shape:
  - The index is about a fifth of the model JSON.
  - A thing gets the kind of the first participant that draws it.
  - Two uses on one line of one step are one use.
  - "Also called from" (a static call graph) and the index (the stories) can disagree.

## Changes [§6]

- Show which stories a change probably touches, from their steps' files, next to "files changed" (only a hint).
- Changes to files only the language server reads (`package.json`, lock files, `tsconfig.json`, `pyproject.toml`) are not noticed and need the server restarted.
- Keep the model of the last OK on disk per project, so opening a project tells what changed while plainfold was closed.
- Name the exact signals in the change summary: a new write or delete, a new table, a sign-in call that is gone.
- A removed story's participants open their definition from the current files.
- Read again automatically: a quiet period with code that parses, a commit, an agent's "done".
- Say why a step changed when its own code did not, from the files its resolution went through.
- Read only the stories a change can affect, once that matches a full read on two unrelated projects.
- Renames not found (they stay removed and added): renamed and edited at once; a method calling itself through `self.`; two identical functions renamed together; functions no story reaches.
- Two sibling steps with the same signature are told apart by order, so adding one before the other renames the later one.

## Language servers: start-up and memory [§10, §11]

A loaded server holds 0.4 to 1.4 GB and uses no processor while it waits.
- Kept servers:
  - A second window needs per-window watches and "since you last looked" models. The same project in two windows would run two copies.
  - Low memory closes them on macOS only; memory is measured with `ps`, so Windows shows "not measured".
  - Package installs are noticed only through the files package managers write. An environment given with `--python` is not watched.
  - Files a server reads from disk by itself (`.d.ts`) are not sent when they change.
  - The project's files are still read in full on every read.
- While reading again, keep the old diagram on screen ("reading…") and swap in the new one with its changes.
- Keep a server's own cache on disk per project where it has one (Eclipse JDT LS).
- Measure Java (spring-petclinic with JDT LS, cold and cached) before relying on estimates.
- Memory pressure, to research: set tsserver's heap limit as a setting; check whether tsserver dying alone goes unnoticed (it is a child of typescript-language-server); one server at a time; leaner pyright settings; load only the sub-project a story lives in.

## Setup section [§5]

- Runs on, ships with, settings, built with, developer tools. The tab shows only how the project was read for now.

## English and the LLM layer [§7, §8, §13]

- A second checking pass, and an overview someone who does not read code can follow (the code one step down, in the panel).
- "This is wrong" and user corrections pinned to the code's hash.
- The hosted proxy and a local model (the user's own key is built).

## Desktop app

- Diagram constants to watch, each checked on two projects: side work is a function reached by at least 3 stories and a fifth of them (`SIDE_MIN`, `SIDE_SHARE`); a story opens at functions when that makes at most 6 columns (`MAX_COLS`); an endpoint at least half of the screens and controls call is said once above the screens (`places.ts`).
- The first level keeps what the screen holds only as a way ends (before a return, or last in its branch). Seen on two apps; check it on a form-heavy one.
- The story list: a long list of screens is not grouped (by folder or by tab); a control's row shows at most three things it does.
- Room kept for closed calls and one-row branches can look empty.
- The Tests tab builds a view of every story; not measured beyond 200 stories.
- A closed call shows its last return, so a function that returns inside a loop or `try` and `None` after reads "→ None · or None".
- Linux: try it.
- Settings: the downloaded servers, their size, remove, update.
- A native menu bar (File › Open Folder, Open Recent).
- Choose the editor "Open in editor" uses; today the order is fixed (Zed, Cursor, VS Code).
- Forget a recent project from the welcome window.
- A shared part's own page [§4].
- Signing, notarization and updates.

## Security [§9]

- Restricted mode for an untrusted repository. Until the user trusts it, use plainfold's TypeScript instead of the project's own.
- Show hidden or direction-changing Unicode and look-alike letters escaped, with a warning.

## Integrations

- MCP [§14]. Measure first. On a few tasks in unrelated fixtures (change a function's signature, change code that writes a table), run an agent once with search alone and once with `uses` / `diff` / `again` callable from the shell. Compare tool calls, tokens and misses, and build the server only if the difference is real. Answers must say what is unresolved, so "nothing reaches it" is not read as "safe to change".
- Claude Code integration [§12].

## Clean-up

- Fix the existing clippy warnings in `core/src/adapters` (unnecessary casts, collapsible `if`s).
- Remove `spike/skeleton`'s one-page view (`src/view.html`, `src/render.ts`).
