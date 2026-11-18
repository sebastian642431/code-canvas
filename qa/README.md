# Scenarios

Tests that run the real program. Each one starts Code Canvas on a folder made
for it, plays a script inside the app (no mouse or keyboard of yours is used),
takes pictures of the window where the script says so, and then checks how it
ended: the exit code, the log, files on disk.

```
cargo build --release
powershell -File qa\run-all.ps1               every scenario
powershell -File qa\run-all.ps1 -Only git     the ones with "git" in their path
powershell -File qa\run.ps1 -Scenario qa\scenarios\git\stage-one-change.scn
```

`PASS name` or `FAIL name` and why. Pictures, the data folder and the log of a
run stay in `%TEMP%\code-canvas-qa\<name>` (or `-Work <folder>`): look at the
pictures, a scenario that passes can still draw something wrong.

Unit tests are apart: `cargo test --all`.

## A scenario

A `.scn` file under `scenarios/<area>/`: a header of `key: value` lines, an empty
line, then one step per line.

```
title: One change of a file is staged and the rest stays unstaged
folders: git-changes
expect-log: script: Part

wait 3000
left changes
part stage src/steps.rs 39
wait 1500
mark staged
quit
```

| Key | Meaning |
|---|---|
| `title` | What it shows, in one line. |
| `folders` | Fixtures the app opens, in order: `git-changes, rust-small`. `name/sub` opens a folder inside one. |
| `fixtures` | Fixtures made but not opened (to open later with `workspace put {FIX}\name`). |
| `limit` | Seconds before the run is stopped (90). |
| `expect-log` | A regular expression the log must match somewhere. May repeat. |
| `forbid-log` | One it must not. `panicked`, `panic:` and `] ERROR ` are always forbidden. |
| `allow-log` | Lines matching it are not checked against the forbidden ones. |
| `expect-file` | `path :: regex`: a file (relative to the fixtures) whose text must match. |
| `absent-file` | A file that must not exist when it ends. |
| `guest-expect-log` | As `expect-log`, for the second instance. |
| `prefs` | The preferences the app starts with, as JSON (also the guest's): `{"autosave": false}` for a scenario about what is typed and not saved. |
| `github` | `empty`, or a file of `qa/fake` with the files of a gist: the run gets a GitHub of its own on this computer (`qa/fake/github.py`) to sign in to. |
| `extensions` | Folders of `qa/fake` put in the run's extensions folder before it starts: `relations-extension` plays a language server. |

A line `=== again` starts a part that is run after the app has ended: the app
is started again with no folders on the data of the first run (what it
remembered is what is being tested). Its pictures are `again-<name>.png`, and
`again-expect-log` / `again-forbid-log` check that second log.

In steps and in `expect-file`, `{FIX}` is the folder of the fixtures, `{WORK}`
the folder of the run, `{REPO}` this repository. In an `agent` step they are
written with forward slashes, because what follows is JSON.

The steps are the ones of `CODE_CANVAS_SCRIPT`: see the table in
[docs/NOTES.md](../docs/NOTES.md), `script.rs`. `mark <name>` takes a picture
called `<name>.png`.

**Two instances.** After a line `=== guest` come the steps of a second
instance, started once the first has shared its session; `{INVITE}` in them is
the invitation. Its pictures are `guest-<name>.png`.

## Fixtures

`fixtures.py` makes them anew for every run, so a scenario can change them:

| Name | What it is |
|---|---|
| `rust-small` | A tiny Rust project, committed, nothing changed. |
| `plain` | The same with no git. |
| `git-changes` | With three changes not staged in `src/steps.rs` and a new file. |
| `git-history` | Several commits, a merge, a file deleted along the way. |
| `git-subfolder` | A repository whose `app/` folder is the project. |
| `odd-files` | CRLF, tabs, no final newline, empty, not UTF-8, binary, a 5000 column line, 12000 lines, names with spaces and accents. |
| `typescript` | A tiny TypeScript project with a `package.json`. |
| `empty` | An empty folder. |

More fixtures go in a `fixtures_<area>.py` beside it with its own `FIXTURES`
dictionary; they are picked up by name like the others.

## Things to know when writing one

- A script that ends with something typed and not saved meets the app's
  question: answer it (`answer Close without saving`) or save first (`key save`).
- Put a `wait` after a `mark`: the picture is taken a moment later.
- The runner sets `CODE_CANVAS_SHARE_ON=127.0.0.1` (no firewall prompt) and
  `CLAUDE_CONFIG_DIR` to a folder of the run (Claude Code's real settings are
  never touched).
- Do not install or open the Claude Code extension in a scenario: it would run
  with the real account.
