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
| `openvsx` | `on`: an Open VSX of its own for the run (`qa/fake/openvsx.py`, three made-up extensions with icons, READMEs, changelogs and manifests, one of each kind: things that do not work in Code Canvas, all that works, code only for the browser); the app is pointed at it with `CODE_CANVAS_OPEN_VSX`. What it was asked is in `openvsx.log` of the run. |
| `defaults` | `offered`: the first start asks about the default extensions, as an unscripted one does (`CODE_CANVAS_OFFER_DEFAULTS`). |
| `codex` | A fixture that is Codex's folder (`CODEX_HOME`) for the run, such as `codex-home`; without it, an empty folder of the run. |
| `registry` | `on`: package registries of its own for the run (`qa/fake/registry.py`, `CODE_CANVAS_REGISTRY` points the app at them) and `QA_NPM_TOKEN` set for the fixture's `.npmrc`; `no-token`: the same without the token. Each request is written to `registry.log` of the run; a private package asked where it must not be writes `leak.txt` there. |
| `extensions` | Folders of `qa/fake` put in the run's extensions folder before it starts: `relations-extension` plays a language server, `views-extension` gives code lenses and tree views, `grammar-extension` brings only grammars (made-up languages, one in a property list, an injection, and one for Rust); `trust-extension` plays ESLint (cannot run in a folder not trusted) and `limited-extension` plays TypeScript (runs, without `qa.limited.path` from the folder): each writes a line to a file of the first folder when it starts; `write-extension` asks the app (command `qa.write.outside`) to change a file of the folder next to the first; `child-extension` leaves a detached process writing to `qa-alive.txt`. |
| `relay` | `on`: a relay of its own for the run (`canvas-relay.exe` next to the program, listening on `127.0.0.1` at a random port, stopped at the end); `{RELAY}` in the steps is its address, so `share relay {RELAY}` before `share start` shares through it. What it printed is in `relay.log` of the run. |
| `relay-expect-log` | As `expect-log`, for what the relay printed. May repeat. |

A line `=== again` starts a part that is run after the app has ended: the app
is started again with no folders on the data of the first run (what it
remembered is what is being tested). Its pictures are `again-<name>.png`, and
`again-expect-log` / `again-forbid-log` check that second log.

In steps and in `expect-file`, `{FIX}` is the folder of the fixtures, `{WORK}`
the folder of the run, `{REPO}` this repository, `{RELAY}` the address of the
run's relay (with `relay: on`). In an `agent` step they are
written with forward slashes, because what follows is JSON (in a `touch` step
too: it can append a line of JSON to a file, such as a session of Codex).

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
| `deps-folders` | 960 TypeScript files in eight layers of four folders, five thousand imports, two import cycles (`core0` and `pages0` import each other; `util1` → `util2` → `util3` → `util1`). |
| `big-folder` | 30,720 TypeScript files in 6 × 8 × 8 folders of 80, each importing two of its folder, and a `package.json`; no git. Takes about 20 s to make and 30 s for the app to read the first time. |
| `empty` | An empty folder. |
| `versions-manifests` | One manifest of each kind whose versions the app shows (`package.json` with a `.npmrc` that sends `@latin-securities` to a private registry, `Cargo.toml`, `requirements.txt`, `go.mod`, a `.csproj`), with dependencies the fake registries know (`fixtures_versions.py`). |
| `trust-repo` | A folder whose `.vscode/settings.json` points an extension at a program of its own (`qa.limited.path`), and a TypeScript file (`fixtures_trust.py`). |
| `codex-home` | Codex's folder with one session at work a few minutes ago on `rust-small` (`fixtures_agents.py`), for `codex: codex-home`. |

More fixtures go in a `fixtures_<area>.py` beside it with its own `FIXTURES`
dictionary; they are picked up by name like the others.

## Things to know when writing one

- A script that ends with something typed and not saved meets the app's
  question: answer it (`answer Close without saving`) or save first (`key save`).
- Put a `wait` after a `mark`: the picture is taken a moment later.
- The runner sets `CODE_CANVAS_SHARE_ON=127.0.0.1` (no firewall prompt), and
  `CLAUDE_CONFIG_DIR` and `CODEX_HOME` to folders of the run (the real settings
  and sessions of Claude Code and Codex are never touched or read), and
  `NPM_CONFIG_USERCONFIG` to a file of the run (your own `.npmrc` and its tokens
  are never read).
- Do not install or open the Claude Code extension in a scenario: it would run
  with the real account.
