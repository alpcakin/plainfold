---
name: commit
description: Finish a piece of plainfold work and commit it. Updates the documents, runs the checks the change needs, and writes the message in this repository's style. Use when the user asks to commit or push ("commit", "commitle", "pushla", "commitle pushla").
---

# Commit

Push only when the user asked for it ("pushla"). Never force-push, amend a pushed commit, or skip a check without saying so.

## 1. What changed

Run `git status` and `git diff --stat`. Commit only this work. Leave unrelated changes alone and mention them.

## 2. Checks, by what the change touches

Run each check only if it has not already passed in this conversation since the last edit to the files it covers. Edits to Markdown files cover no check. If you skip a check, name it and say when it passed ("check.mjs passed after the last core edit"). If you cannot tell, for example because the work came from another session, run it.

- `core/`: `cargo build` and `cargo test` in `core/`, then `node core/scripts/check.mjs`. Read its differences. A difference you did not intend, in a project you were not working on, stops the commit: tell the user before going on.
- `app/src/`: `npm run check` in `app/`, and the four looks (`look` skill).
- `app/src-tauri/`: `cargo build` in `app/src-tauri/`.
- Documents only: no checks.

Report failures with their output. Don't commit past one unless the user says so.

## 3. Documents

Follow "Documents stay current" in the root `CLAUDE.md`:
- `DECISIONS.md`: mark what is now built (the rule plus **Built**) and rewrite what changed.
- `TODO.md`: delete what is done and add what was found.
- The touched folder's `README`.
- A `CLAUDE.md` only if a rule or command changed.

Put these in the same commit as the code.

## 4. Message

Match `git log`:
- The subject says what plainfold now does, in plain words, in the imperative, with no prefix or scope: "Notice renamed and moved functions", "Show what uses what in the app". Keep it under about 72 characters.
- The body is short plain sentences wrapped at about 80 columns. Say what changed in behavior, what it was checked on (and what is unverified), and what it left unchanged elsewhere. Use a short list when there are several parts.
- Use neutral names in examples. Never name a private fixture or quote its code.
- End with the attribution lines the system gives for commits.

Pass the message with a heredoc so its lines stay intact.
