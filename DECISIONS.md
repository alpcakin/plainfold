# plainfold: Product Decisions

What is decided, built, proposed and open, as it stands now. When a decision changes, rewrite it; the history is in git. A built decision is its rule plus **Built**; the details are in the code.

**Labels:** **Decided** (the user agreed) · **Built** · **Proposed** (suggested, not confirmed) · **Open**.

## 0. Status (Decided, 2026-10-30)

plainfold is no longer developed as a product. Where a story starts, what it includes and how each framework reads are judgment calls with no ground truth to measure against, so every framework and project brings new cases. The repository is published as a portfolio project with a browser demo (`demo/`). The rest of this file is the state when work stopped.

## 1. What plainfold is (Decided)

A separate, read-only desktop app that shows what a codebase does as **stories** drawn as sequence diagrams. A story is what happens when a screen opens, a button is pressed, a request arrives or a job runs. Beside them is a **Setup** section for how the app runs and ships.
- Observer, not workspace: people code elsewhere (editor, agent) and open plainfold to understand. It never writes code and hosts no agent or terminal.
- The code is the single source of truth; English only represents it. Every element leads down to the exact lines.
- It notices changes made elsewhere, by people or agents, and shows them.

## 2. Audience and primary use (Decided)

- Anyone who wants to see the whole of a system they build, with a little more weight on developers. First, developers who build with AI and read code only some of the time, and experienced developers who want what no other tool shows (what reaches what across boundaries, what changed in behavior). Second, people who build with AI and don't read code. Later, teams. Students and teachers may be a channel (§16).
- One view serves both: an overview you can follow without reading code, with the real code one step down. It has to hold up for an experienced developer.
- The primary use is **following in the code what a person meets in the app** (Decided, 2026-10-14): the app opens and a screen appears, a button is pressed and something happens, a screen opens and data is fetched and checked. plainfold tells that experience technically, in diagrams and English, down to the lines. It cannot draw every flow, so it shows what a person would notice.
- The second is **live continuity**: what changed as people and agents edit, in the same words ("the list screen has a new Delete button, which deletes the list"). Open: a separate app is easy to forget. One candidate is a nudge at the right moment ("Claude finished: 3 parts changed, one login check changed").

## 3. Main view: a story's sequence diagram (Decided)

- Organized by behavior, not files: entry points (screens, user actions, endpoints, jobs), then the flows they run, then data and outside services. Files stay as small labels on parts.
- **The story list is the places a person meets (Decided, 2026-10-14).** First what runs as the app starts and the layouts around its screens. Then the screens, in the order a person reaches them from the start (along what the start, the layouts, each screen and its controls open; screens nothing found opens come last and say so). A screen opens into what it does when it opens and the controls drawn on it, under the parts of the screen that write them (a part with no controls of its own adds no heading; one drawn on several screens says how many, and is listed under each). Each control's row says what a person would notice: what it changes in the data, the endpoints and sign-in it calls, the screen it goes to, else what it reads. One that only goes to another screen reads as a link. Endpoints most of what a person does calls (a session refreshed before each request) are said once above the screens. Then the server (endpoints by file, what runs by itself) and the entry points no adapter places. Controls in parts no screen is found to draw are listed apart, as a gap. A part no screen draws that a step of a place opens (a sheet, a dialog, a screen opened by its widget) is in that place. A single-page app reads the same way: its parts are the second level. Tests keep their own tab. **Built**, checked on an Expo app, a Next.js app and a Flutter app with a FastAPI backend.
- **A diagram's first level shows what a person would notice (Decided, 2026-10-14).** In a story that reaches past its own code, what only the code holds is left out until "every step" is on: what the screen holds on the way (`setLoading(true)`; what it holds as a way ends, such as the message before a return, stays), a helper of the project's own that works a value out (`normalize(email) → string`), and side work. A step that changed since the user last looked is always shown. In a story that reaches nothing outside, those steps are all it does, so they stay. **Built**, seen on two apps.
- It stays a diagram, not an outline, because a deterministic diagram can be compared and posted (§16).
- The details panel shows fixed slots for a step: what it sends, what it gets back, what it does if it fails.
- The reading rules are in `app/CLAUDE.md`. **Built:**
  - columns at two levels (places or functions), the same while reading and spaced for what is shown (they used to keep room for every call open and closed, which left wide empty gaps over closed calls; spacing for what is shown is the user's idea, to be tried)
  - stories that open closed
  - every call that changes column returns
  - side work as one quiet row (left out at the first level, below)
  - no text over another lifeline
  - only what the diagram cannot show written beside it
  - tests in their own tab ("reached by", never "tested")
- **The diagram is a canvas (Decided).** It sits in a fixed area over a faint grid and moves and zooms there, as a chart does: the wheel, two fingers or a drag move it, a pinch or ⌘ with the wheel zooms toward the pointer (wheel-to-zoom was tried first and moved too much). The column boxes stay at the top; lifelines are solid, so they are not lost in the grid. The zoom a story opens at depends on its size: whole when that keeps it at 85% or more, else as wide as the area from the top. Opening a call slides the rows below rather than jumping. What the diagram cannot show sits in a pane of fixed height under it, a tab each, resized by dragging. **Built.**
- **Nesting is told by the numbers (Decided).** Opening many calls inside each other stacked a tint per level, and an indent in the gutter was lost once the diagram moved across. Nothing is painted behind an opened call and nothing is indented. Each row's number is its place in the story with every call open, like line numbers in an editor that folds: a closed call is followed by a jump (`2`, then `10`) that opening it fills, and no number changes (the user's idea; it replaced `2.1.2`, whose long numbers made a wide strip). The numbers stay at the left edge when the diagram is moved across, on one narrow strip. **Built.**
- **The open button (Proposed).** The small mark at the end of a call's words did not say clearly that it opens, or that it is open. It is now a small button: soft `▸` while closed, filled `▾` when open. **Built**, waiting for the user's word.
- **What uses what.** An index of every function, package, service, endpoint and table, with the stories that reach it, how, and at which step and line. It is made from one reading, counts what runs even where the diagram folds it, and is a lower bound ("at least"). A function's id is its file plus the names it is written inside (`file#Class.method`). An unnamed one is known by the call it is passed to. Ids are never parsed. **Built** in the core and the view: Go to, "N stories reach it", and the Data tab. A click in the diagram opens what it is on and does not narrow the list. Its own interface: Open.
- Not chosen for now (may return as secondary views): a whole-app board, a column trail, a story text view, a feature list.

## 4. Stories (Decided)

A story is one trigger and everything after it.
- Triggers: a screen opening, a user action, a request from outside, a schedule, a data change, a queue message, the app's lifecycle. **Built:** screen opening, user action, every endpoint (named by its method and full path, with the dependencies its framework runs first), a schedule and the app's lifecycle.
- What a framework or a library starts by itself is a story of its own, named by its function and by when it runs, in the adapter's words around the code's values ("every settings.every minutes", "every sun at 20:00", "when the app starts"). It is listed under the file that hands it over: the app's start first in its place, schedules before the endpoints. It is shown whatever it reaches, like an endpoint. Where the code hands the function over, that step is a reference to its story. **Built:** APScheduler jobs, FastAPI's `lifespan` and startup and shutdown handlers, the function that calls Flutter's `runApp`, a Next.js `layout` ("around every page under /docs"), `generateMetadata` and `middleware` (with its `config.matcher`), an Expo Router `_layout`.
- An endpoint is a story of its own whether the project calls it or not, listed under its file. What it answers is always a row, whatever the framework, even when it is its only step. A story that calls it shows it in place too, since the caller waits for its answer: a call, not a reference. Its row and its page say how many other stories reach it (at least); one nothing reaches says a request starts it. Any story's page says how many other stories reach where it starts (a screen another story opens, too). **Built.**
- A user action belongs to the place its control is written in (Decided). A button or a form in a component is a story there, told once however many screens draw the component; the list shows it under each screen that draws it (what draws what is in the index, as `draws`). Every component of the project counts, drawn by a screen or not (a layout, a project with no router adapter). Its place is where the interface runs: the browser, or a device when an adapter says so. Events the platform fires by itself (`onLayout`, `onLoad`) are not user actions, and neither is the start or end of a press on a control that handles the whole press. An `on…` prop given to a component of the project is a user action only where a control in it can set it off (directly, through a function of its own or through another component); one only called from code, such as at the end of an animation, is not, and props spread onto an element are not followed and count. **Built** for React, checked on a Next.js app and an Expo app; the press phases seen on one project only.
- The story list has a heading for each place a story starts in (device, browser, server), as its adapter gives it, device first like the columns. The other entry points have a heading of their own, since the code does not show where they run. **Built.**
- Where a function no adapter places runs (Decided): where the code that hands it over runs. A function value does not leave its program, so a function passed to a call or stored inside another function runs where the stories told so far reach that function, when they reach it in one place only. A job a server's start hands to a scheduler runs on the server. **Built**, checked on two FastAPI projects.
- Where nothing shows the place, the diagram says so: the project's code is the place `Code` (Proposed; it used to be `Server` for every entry point without an adapter, which was a guess). **Built**, waiting for the user's word.
- Without an adapter, a story starts wherever the code shows that something outside starts a function. **Built.** That is:
  - a top-level function or method nothing calls;
  - one registered with an outside call whose result is unused;
  - top-level code that calls the project's own functions.

  Not entry points: methods reached through a base the project calls, constructors and other methods the language runs, getters, and code in a file a tool made, as its first comments say ("auto-generated", "do not edit"): what of it nothing calls is no story. An adapter that matches names stories better.

  Code nothing runs is no entry point either (Proposed; in an app, every unused helper and every unused part of a UI kit was a story, a hundred in one project). That is:
  - a function declared with a name of its own that nothing names (no use, no import), in a module the project imports for something else. A module nothing imports may be run by a tool or a command, so its functions stay; so do methods, which are reached through their object, and tests;
  - a function stored under a key of an object where the project calls that key on a value it cannot follow (`model.load()`, typed by an interface): it runs there;
  - what a framework runs only where the code names it, when nothing names it (a React component).

  **Built**, checked on every fixture and on two apps of the user's; waiting for the user's word.
- Tests (found by each runner's naming rules) have their own tab and are always shown. A pytest file's fixtures, helper classes and `conftest.py` are no tests: pytest collects only `test*` functions (and the `test*` methods of `Test*` classes or classes with a base), so the others are no stories. A function another framework registers by a decorator (a route of an app written for a test) still is one. **Built**, checked on three Python projects. The bodies given to the runner (`describe`, `it`) are walked as frames; the runner's own calls (`expect`) are not drawn. **Built.**
- Calls that can run other code. **Built:**
  - A project function handed to an outside call whose signature takes a function is a frame, "when X runs it".
  - A call to a method that the project's subclasses override, or that classes implementing an interface write (a TypeScript field given a function counts as a method), shows each version as an alternative. Versions written in tests count only for calls written in tests.
  - A call on the class itself reaches only that class's member.
  - A function taken from a value a framework shares is followed to where it is given (React's `useContext(X)` and `use(X)`, to `<X.Provider value>`; what `useMemo`'s function returns). So is one a function of the project returns (a factory), one at a place in a list it returns (`const [on, setOn] = useToggle()`, Python's `a, b = f()`), down to a state setter, and one taken from a module loaded while the code runs (`await import("./store")`). The function a hook hands back is followed too: `[state, run] = useActionState(f, init)` (also `useFormState`, `useReducer`) runs `f`; a choice between functions there (`ok ? signIn : signUp`) is one story each. A function made by a call of the project's own (`save = guard(check, f)`) is known by the name it is stored under, and what the call was given stands for the made function's free names: `f(x)` inside what `guard` returns runs the `f` this call gave. Checked on three apps (an Expo app and two Next.js apps); Python's lists on a small example only.
  - A function that runs in one place whoever calls it is a call across to that place, and a crossing by itself: a Next.js server action (a function of a file that starts with `"use server"`, or one that starts with it; one a call made is where its name is stored) called from the browser. **Built**, checked on one app (a Next.js app with form actions).
  - A function taken from what the project's code returns or provides that is not found there is a gap ("not resolved"), never a silent plain statement. A function only the run decides (one the caller gives, one kept in a field or a list) is not a gap: it is said nowhere yet.
- A package call written inside the arguments of another package call, with nothing waiting for it, builds a value for that call (`op.create_table(t, sa.Column(…))`, `where(and(eq(a, b)))`): it is part of that call's row, not an arrow of its own. A call into the project's own folder that resolves to a file with no code (a `.pyi` stub, whose functions are made while the program runs) is a gap, never silent. A parameter stands for what the call being walked gave it (`fetchJson("/items")`), so an address inside a wrapper is known. **Built**, checked on five projects.
- A file's top-level code is a story only where it does something: a statement that is not a declaration (a call on its own, a condition, a loop, an awaited value), or a call whose result nobody uses that leaves its caller's lifeline. Code that only makes values (`const schema = make(…)`, a route registered by a one-call `createRoute(…)`) is set-up, and an `export default X` is no use that runs `X`. **Built**, checked on four projects.
- Only meaningful stories: an action is a story only if it crosses a boundary (network, database, outside service, installed package). A call into an installed package is an arrow to that package. A package that is not installed is followed by its imports: a name imported from it, and a value one of its functions makes in the same file or a value of a type imported from it (`(db: Client)`, or a type alias of one), when a method is called on it (`client.send()`, not `result.items.map()`), count as that package, shown by name only. The language's own library and platform are not drawn, and neither are packages an adapter handles. **Built.** Open: whether an entry with logic but no boundary is a story.
- Shared parts are told once, folded; the panel says how many other stories reach them (at least). **Built**; their own page is not built. A shared function stays in place in each story (its rules belong to every path through it).
- Another story is a reference, not a copy (Decided). A step that reaches the start of another story (a screen rendered or opened whose opening is a story of its own) is a row on its lifeline that goes to that story, told there once. What that story reaches counts for this one too, in the index. A screen opened by its address (a router's page) is a story whatever its opening reaches, so every step that opens it is a reference; a place known only by what it draws (a Flutter widget) is one where it crosses a boundary. **Built** for Flutter widgets, React screens and navigation.
  - A step that hands work to another story to run later (a job given to a scheduler) is a reference too. What that story reaches does not count for this one: it does not run here. **Built.**
  - A reference names the story it goes to after its `↗` (Proposed: three rows reading `add_job(…) ↗` could not be told apart). **Built**, waiting for the user's word.
- Branches stay inside the story as folded sections. **Built.**
- Names come from the code. A story is named by the words on the control (including a string constant, a choice between words, each of them: `Manage / Upgrade`, an attribute the component writes as text, or an accessibility label; a form by its control of type submit, else its first button), else the handler's name in plain words, else the event. Stories of one place that would read the same are told apart by the handler's name, else the line. **Built**, checked on two projects with user actions (an Expo app and a Next.js app).
- One story for one piece of work (Decided). Triggers on one screen whose steps are the same after the trigger (a button and the keyboard's submit that run one function) are one story that says each way it starts. It is named by the one with words on its control, else the first written. **Built**; seen on one project only (a Flutter app), unverified elsewhere.
- A step is something that runs. Declarations inside a body are not steps, and plain statements in a row are one step. **Built.**
- What the code works out (Decided). A sequence diagram shows who calls whom, not how a value is worked out, so:
  - A calculation (data in, data out: a total, a settlement, a sort) is one step in its story: the call, what it is given, what it returns. Its inside is not drawn as arrows.
  - Opening it shows its code, and the tests that reach it with what they give it and what they expect back, as written in the tests. Not built.
  - A short summary written by a model may come last, marked as one (§7). Not built.
  - What a function changes of what reaches it from outside is shown: a value it is given, or its own object (`self.items`, `this.last`, a Dart field written without `this`). A statement that does it is a step of its own; a call says what it changes, as the call writes it (`· changes cart.items`), from what it calls too. Changing means writing into it, or calling the language's own library on it for its effect; a call into the project or a package is an arrow of its own. A function that changes something is never dropped as side work. **Built.**
  - Shared state at the top of a file (a cache, a client made once) is changed by one story and read by others: later, as a thing in the Data tab with the stories that write and read it, like a table.
  - Not chosen: a flowchart of a function. It does not read easier than the code.
- Rules (Decided). A `throw`, or a `return` before the end of its function (code after it would run otherwise, or a loop stops), is a rule. Where every branch returns, none is early; a `return` followed only by code that throws is the way through, and the throw is the rule. It matters most when code changes.
  - Found from structure in every language; what a `catch` handles (by the types it names) ends nothing. Never folded into a plain step, wherever the function is reached from. **Built** (Python, TypeScript, Dart).
  - A throw in a `catch` that throws what it caught again (`rethrow`, a bare `raise`, `throw e`) is no rule of its own: it passes on how the `try` ended, named by what the catch handles. **Built.**
  - The answer to a request comes back to the sender as a value: what ends an endpoint ends no function of the sender. **Built.**
  - A call that can end early is never dropped as side work. Closed, it names how else it can end ("· or 403, Denied"), including what the functions it calls throw. Opened, each rule is a row where it is written, its condition in front; after branches that end its function, what follows has the else of each as its condition (`else (a), else (b)`). A `return` that is the way through only because a throw follows keeps its branch, so the throw's condition can name it. A `catch` (and a promise's `.catch`) is a frame of its own, named by what it handles. **Built.**
  - Under the diagram, "Where it can stop": one row per way the story ends (`401`, `Denied`), a line per condition as written with its place, the same condition in more places once. A throw counts wherever it is. An early return counts only where it ends the work of a place (the story's own function, or one another place calls, such as an endpoint); other early returns are rows inside their call. The story's own function is a screen's, or the handler of a user action, where an inline handler that only calls a function of its screen (`(_) => submit()`) counts as that function. **Built.**
  - A loop's `break` or `continue` is a row of its own where it is written, its condition in front (`[not found] continue`): that round of the loop ends there (Proposed; before, it hid in a plain step, so the reader could not tell a retry from a stop). A loop that only skips rounds shows nothing. **Built** (Python, TypeScript, Dart), waiting for the user's word.
  - A framework adapter may name what is thrown by the answer it becomes (FastAPI's `HTTPException` by its status), and calls that end the work by throwing inside the framework (Next.js `notFound()` is `404`, `redirect(url)` is `redirect url`). **Built**; the Next.js calls are seen on one project.
  - A rule added, removed or changed is a structural change signal (§6). Not built.
- List order: the screens in the order a person reaches them (§3, **Built**). Not built: within a screen, recently changed first, then risky controls (writes, payments, deletes, sign-in).

## 5. Setup (Decided, not built)

A Setup section beside the stories, read exactly from config files; the LLM only writes the sentences. Mockup: `prototype/sequence.html`.
- **Runs on:** Dockerfile, compose, vercel.json, fly.toml and similar files.
- **Ships with:** CI workflows, as a row of steps.
- **Settings:** environment values and secrets, values hidden: where each is defined, what uses it, whether the browser can see it.
- **Built with:** libraries and the project's commands, from the manifests. **Built** for packages that are not installed: which manifest lists each one, where it was looked for, and the command the project's own tools would install it with, where the files show the tool (a lockfile, the tool that made the environment, Flutter in `pubspec.yaml`), with a button to copy it. plainfold never runs it (§9): installing writes into the project and runs the packages' own code.
- **Developer tools:** kept out of the way.

Changes to these files (deploy rules, new secrets, values reaching the browser) are among the strongest change signals.

## 6. Changes (Decided)

`code → analyzers → model → views, and changes (the old model compared with the new one)`
- Noticing is separate from reading again. A file has changed when its content hash differs. The app says so at once ("3 files changed · Reload") but reads again only when asked, because a save may be half-written. Automatic triggers may come later: a quiet period with code that parses, a commit, an agent's "done". **Built** (noticing, Reload).
- Changes are what differs between two full models, never guessed from the changed files. The same code read twice gives the same model. Reading only the stories a change can affect may come later, once it matches a full read on two unrelated projects. **Built.**
- A step keeps its identity through its path of signatures from the top of its story (its kind, its ends, its code without the values), so edits elsewhere do not rename it. A renamed or moved function is matched by its code without its own names, one-to-one only. Its calls become changed steps and its story says "renamed from …". **Built.**
- Compared like `git diff`: the longest common sequence of signatures, where a removed and an added step of the same kind that share names count as one edited step. **Built.** Shown like it too. **Built:**
  - `+` for added, `−` for removed (kept in place, dashed), `~` for changed, with the old row above;
  - calls that hold a change start open;
  - a "Since you last looked" list, and one OK clears every mark;
  - the comparison is with the model at the last OK, kept while the app stays on the project.
- Structural signals are exact and get the strongest highlighting: a new database write, a removed login check, a secret exposed to the browser, a deploy that now runs for every branch. Not built.

## 7. Accuracy (Decided)

Visuals and accuracy are the top priorities, and the code is the source of truth.
- Structure is deterministic: analyzers and adapters decide parts, links, order, branches and line ranges. The LLM never decides structure, links or names. **Built.**
- Every line of a walked function belongs to a step; lines that don't are reported as coverage gaps. Inferred links are dashed. **Built.** The panel marks a step's English with `≈` (what it means is on hover and in "How to read this"); the diagram does not mark it yet.
- English text. **Built** (first version, `core/src/english/`):
  - One short text per step, asked one function at a time with that function's code, through an OpenAI-style endpoint (gpt-4o-mini by default, with the user's key from the keychain or `OPENAI_API_KEY`).
  - Names and values are copied from the code and marked. A text that fails a check against the code is not shown, and the step keeps its code text.
  - Answers are cached by the hash of what was sent. One toggle turns English off.

  Proposed: the text never hides control flow; a second checking pass, ideally by another model family; dropping language syntax so languages read alike; English written only for what people open and for changed parts.

## 8. When the English is wrong (Proposed)

Model text must be noticeable, contained and correctable.
- **Contained:** only code is read (§9); facts are inserted from the code; each job is small. If a text fails, the code's own name is shown (**Built**). The same code always gets the same text (**Built**, through the cache).
- **Noticeable:** every text is one click from its code; simplified text is marked `≈`.
- **Correctable**, without writing code:
  - "This is wrong" rewrites the text with a stronger model and stricter checks, optionally with a hint from the user.
  - A user's own correction is pinned to the code's hash, flagged when that code changes, and shared with the team.
- **Measured:** a test set of real projects with human-checked text decides the recommended models.

## 9. Security (Decided)

plainfold is never trusted more than the code it describes. Facts derived from code outrank words.
- **Only code counts.** Comments, docstrings, READMEs, commit messages and PR text never decide or check anything and are never sent to an LLM (**Built** for the walk and the English). Names are labels shown in code font, not facts. An agent's notes (§14) are shown as the agent's claim.
- **Against misleading text:**
  - The LLM refers to symbols and cannot add names or facts.
  - Names that read like instructions are masked before the LLM sees them (**Built**) and flagged to the user.
  - Text that contradicts the code's effects is hidden and the contradiction shown. This applies to names too ("named read-only, but deletes").
  - The LLM has no tools and no network, and each request is isolated. Its output is plain text only.
  - Hidden or direction-changing Unicode and look-alike letters are shown escaped, with a warning.
  - Anything sent to an agent is structured, with generated text marked as such.
- **Protecting plainfold:**
  1. The AI proxy holds our key, accepts only structured requests, builds the prompt itself, limits output, and requires sign-in and quotas.
  2. Project code never runs on our servers; a web demo would analyze in isolated containers, in restricted mode.
  3. On the user's machine, an untrusted repository opens in restricted mode: analyzers that can run project code (Gradle, MSBuild, Rust build scripts, TypeScript plugins, Dart analyzer plugins) start only once the user trusts it. A server inside the project folder is never run. Runtime observation starts only when the user asks.

## 10. Languages and frameworks (Decided)

The core knows no specific language or framework; everything specific is an adapter. **Built.**
- A language is a tree-sitter table plus a language server, and the story logic is written once. **Built:** TypeScript/JavaScript (typescript-language-server), Python (pyright), Dart (the Dart analysis server).
- Readiness has no time limit. **Built:**
  - A server is used only once its progress reports have ended and its repeated answers have stopped changing.
  - While waiting, the app shows what the server says it is doing. The wait ends when the server is ready, when it stops, or when the user stops waiting.
  - A server that stops is noticed at once, with the reason; if that happens during a read, the language counts as missing.
- No server means no stories in that language, and the reason is shown, never a tree-sitter guess. Missing dependencies get a clear warning: in the Setup tab, and in the stories that call into them, at the first call. **Built.**
- Servers are found or downloaded per project and never bundled. **Built** (downloads for TypeScript and Python; Dart from PATH):
  - The order is a development folder, then PATH, then plainfold's store.
  - A download asks first, unless the user chose otherwise, and is pinned by version and hash. Versions no longer pinned are removed.
  - Heavy runtimes are the user's own (JDK, .NET, Go, the Dart SDK). A server that comes with its runtime, like Dart's, is found and never downloaded.
  - Node.js is the exception: the user's own when it is new enough, else downloaded from nodejs.org at the current long-term support release, pinned per system.
- Not built: a stub (`.d.ts`, `.pyi`) and its source count as the same target; a second resolver (Jedi next to pyright) whose answers show as inferred links.

| Language | Server | Precision |
|---|---|---|
| TypeScript / JavaScript | typescript-language-server | High (built) |
| TypeScript on Deno (Supabase edge functions) | Deno language server | High |
| Python | pyright, Jedi as second resolver | Medium, better with type hints (pyright built) |
| Dart (Flutter) | Dart analysis server | High (built) |
| C# | Roslyn | Very high |
| Java · Go · Rust | Eclipse JDT LS · gopls · rust-analyzer | High |
| C / C++ | clangd | Medium; needs the build settings |

- **Framework adapters** are small and isolated, and say what they handle. They turn conventions into entry points, links and meaning. Each reads the code of its languages, and a package is matched only in them: one name in two languages is two packages. **Built:**
  - React (a component with user actions is a place of its own; a hook runs quietly on the lifeline of the component that uses it, a function it makes runs where it is called; a shared value from `useContext` is followed to its provider; a component wrapped in `forwardRef` or `memo` is the function it wraps), React Native, Next.js (app router; layouts, `generateMetadata`, `generateStaticParams` and `middleware` (`proxy` from Next.js 16, also one a wrapper is given) are what it runs by itself; a layout that only draws is no story; server actions), Expo Router (a screen's component written in its file, exported from another module or handed to a package's wrapper (`export default Sentry.wrap(Screen)`); `router.push(url)` is a reference to the screen that answers it, `(group)` folders in the URL or not; a `_layout` runs around its screens, and each tab of a tab bar it draws (`<Tabs.Screen name>`) is a reference to the screen it opens)
  - Flutter (every widget is a place: its opening, its user actions, what it renders; a builder handed to Flutter is drawing, walked quietly) and Flutter state (Riverpod, provider; resetting a provider, `ref.invalidate(p)`, is a state step)
  - FastAPI (routes with their prefixes, `Depends` walked first, `TestClient` requests linked to their endpoints, `HTTPException` named by its status, `lifespan` and startup and shutdown handlers)
  - APScheduler (jobs by `add_job`, `add_schedule` and `@scheduled_job`, with their triggers)
  - Supabase (the JavaScript client, also in edge functions whose `jsr:` import is not followed, and supabase-py; Storage as a participant of its own, a bucket or table named by a constant too), web `fetch` (an address written there or held in a constant), Dart's `http`
  - react-hook-form (a form's `handleSubmit(f)` runs `f`), TanStack Query (`mutate(x)` runs the mutation's `mutationFn`, then what runs when it succeeds, fails or ends, each a frame of what runs later; a query's `queryFn` runs later, when its data is first needed)
  - test runners: pytest, Jest, Vitest, Mocha, Dart's `test`
  - SWR (`useSWR(key, fetcher)`: the fetcher runs later, given the key, so a request to an address built from it links to its endpoint), class-name helpers (`clsx`, `tailwind-merge`, `classnames`, `class-variance-authority`: not drawn, since they only choose class names)
  - Riverpod: reading a provider (`ref.watch`, `ref.read`, `ref.listen`) runs its code the first time in a story, shown as a frame on the provider's own lifeline; the provider is no entry point of its own

  Later: routing (Django, ASP.NET, go_router), .NET services, Flutter Bloc, XAML bindings, Celery.
- **Evidence:**
  - On project code, the TypeScript server reached the same function as the compiler for every call. pyright and Jedi agreed on every own-code target in two Python projects (`spike/universal/RESULTS.md`).
  - The core reproduces the TypeScript prototype's steps on two unrelated projects.
  - The Dart server resolved all but 2 call sites in two Flutter apps.
- **Inferred links** (Decided; only the dashed line is built):
  - Before calling a link inferred, read what decides it: config, `.env` files, framework rules, dependency-injection registrations.
  - Line styles: solid is proven, solid with a person mark is confirmed by a user, dashed is inferred. An LLM may remove wrong candidates but can never make a link solid.
  - The user can confirm a dashed arrow, pick its target from ranked candidates, or hide it. Corrections are pinned to both ends, flagged when either changes, and shared. One correction can fix a whole pattern.
  - "Fix with Claude" asks the agent to make the link exact in code.
  - Runtime observation is a later option that only the user starts.

## 11. Stack and architecture (Decided)

The deciding factor is the end user's experience. plainfold is written with Claude Code.
- A Rust core handles watching, tree-sitter, the servers, the model, changes, LLM calls, and keys in the OS keychain. It runs all day, so it must be light and hard to crash. A Tauri shell has a TypeScript web UI: a small install and flexible diagrams. Not Electron (heavy), not a native Rust UI (young toolkits). **Built:** `core/`, and `app/` on macOS.
- Analyzers run as separate processes, on demand. A window keeps its project's servers between reads. **Built:**
  - Before a read, they are told what changed and asked until their answers settle. One whose project settings changed is replaced.
  - They stop with the window, with another project, after 20 minutes away, on low memory (macOS), or with the app. Settings show what each holds and can turn this off.
  - A read with kept servers equals one with new servers.
- Several languages in one project: each sub-project is found by its manifest and everything merges into one model. Links across languages come from boundaries:
  - HTTP route, by URL and method. **Built:** `fetch` to a Next.js route, a test client to a FastAPI route, Dart's `http` to a FastAPI route (inferred when the base address is known only at runtime). In JavaScript, a call named by an HTTP method whose URL a route of the project answers (`api.get("/items")`, a generated client's `client.delete({ url })`) is a request to it whatever library sends it, inferred, since only its name says it sends one; seen on one project (a generated client).
  - gRPC by `.proto`, a shared table by its name, a queue by its topic, FFI mostly inferred.
- Proposed:
  - Local-first: the code stays on the machine. The hosted AI passes small pieces through our proxy, never stored. With the user's key or a local model, code never reaches us.
  - The deterministic layer and the LLM layer are separate modules.
  - The stored model is versioned and can be committed, so a team pays to describe its code once.
- Platform: desktop only for now. macOS first, then Windows (its title bar is written but has never been built or run); Linux is untried. A web view may come later, for sharing and pull requests. A public demo of the view on saved models is **Built** (`demo/`, on GitHub Pages).
- Look and feel: plain and native, like Zed and Ghostty. The system font for the interface, monospace for code, each platform's own window chrome, light and dark from the system, one accent.

## 12. v1 scope

- Next.js + Supabase (Decided): what most AI-built apps use.
- Supabase: client calls are **Built** (tables, database functions, auth, edge functions by name). To do: edge function bodies (Deno server); tables from migrations; Row Level Security in plain English, with unprotected tables flagged; triggers and scheduled jobs from SQL; secrets read with `Deno.env.get`.
- Applications only (Decided): a library's or framework's own source, with its public API as entry points, is not read for now.
- Claude Code integration first (Proposed). After v1, .NET, mainly for business customers (Proposed). Python and Flutter are built.

## 13. AI provider (Proposed)

- Provider-agnostic. The default is a cheap hosted model we pay for, through our proxy, so nobody needs a key. Candidates: Gemini 2.5 Flash-Lite for labels and Flash for change summaries, under $0.50 a month for a heavy user.
- Options: the user's own key (one-click OpenRouter sign-in), or a local model (Ollama, Apple's on-device model).
- Our accuracy tests choose the recommended models; others are allowed with an "untested" warning. A consumer AI subscription is never used inside the app. Without any model, the views still work with the code's names.
- Built for now: the user's own key for an OpenAI-style endpoint (§7).

## 14. MCP (Proposed)

MCP is not the translation engine: an agent describing its own code is no independent check, and edits made outside the agent never arrive. It is for:
1. "Why" notes: the agent's reason for a change, shown next to plainfold's own reading of what changed.
2. A map for agents: the model as an MCP server.

Measure first whether it helps (`TODO.md`).

## 15. Business model (Open)

Product first, prices later. Proposed shape: understanding is free; continuity, assurance, scale and teams are paid.
- **Never paywalled:** reading and understanding, any language, the accuracy features.
- **Free:** hosted English for one project with a daily limit; unlimited English with your own key or a local model; today's changes; exports.
- **Individual** ($2.50–4 a month; annual billing matters at this price): unlimited projects, a better model for change summaries, full history, risk alerts, agent notes.
- **Team** (about $19 per seat): shared model and corrections, PR summaries with a diagram difference, web sharing, SSO.
- **Students:** the individual plan, free.

## 16. Ideas for later (Open)

- **Schools:** diagrams that always match students' code, exported to standard formats; teachers understand a repository in minutes. Mainly a distribution channel.
- **Plan versus built:** an agent sends its plan as a diagram over MCP; after coding, plainfold compares the two ("6 arrows planned, 5 built, one unplanned call to Stripe"). plainfold never becomes a diagram editor.
- **Diagrams in pull requests:** the stories a PR changes, with additions and removals marked, from the same model and `diff`.

## 17. Open questions

- Should steps stay close to the code or read more like a summary?
- Can the English be localized?
- How do changes reach the user: per save, one summary per finished agent task, notifications?
- Team pricing: per seat or per team? Which models make the recommended list?
