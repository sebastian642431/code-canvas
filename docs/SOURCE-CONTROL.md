# Source Control: towards a native Fork

Goal: the Source Control side of Code Canvas becomes a complete git client, native and written in Rust,
so nobody needs a second app. The model is [Fork](https://git-fork.com): its sidebar, its commit graph,
and its menus. Every git experiment runs on the free test repo
<https://github.com/sebastian642431/TEST> (cloned at `c:\rysted\TEST`).

The WORKSPACE list is shared by the Files tab and the Source Control tab on purpose: both need the
folders, their branch, and quick branch switching.

## What there is today

- Changes of the open folder: stage and unstage a file, all, or part of a file (hunks and single lines).
- Commit, amend, commit and push, commit and sync; discard a file or everything (after a question).
- Stash and pop; fetch, pull (fast-forward only), push, publish a new branch.
- The last 40 commits as a timeline, and the files of a commit.
- The branch chip of a workspace folder switches between local branches.

## Phases

### 1. Branches from the remotes (done)
The branch chip's popup lists local branches (in folders such as `feature/`, with ahead/behind) and
those of each remote (what is on GitHub). A remote branch is brought to local and switched to in one
click (`git switch -c <name> --track <remote>/<name>`). A refresh button fetches.

### 2. A Fork-style sidebar in Source Control (done, except what is noted)
Branches, Remotes, Tags and Stashes are under the changes, with their right-click menus; deleting,
dropping and checking out a tag ask first. Still to do: Submodules, Worktrees, a filter, the pin /
show-only / hide buttons (they act on the graph: phase 3), interactive rebase and pull requests.
- Sections: Local Changes, All Commits, Branches, Remotes, Tags, Stashes, Submodules, Worktrees; a filter on top.
- Branches in folders, current one in bold with a check, ahead/behind counts; per-row buttons:
  pin, show only this branch's commits, hide it from the graph.
- Right-click on a branch: Checkout, Pull into current, Merge into current, Rebase current onto it,
  Interactive rebase, New branch, New tag, Rename, Delete (local, and on the remote after a question),
  Push, Copy name, Create pull request.
- Tags: checkout, push, delete. Stashes: apply, pop, drop, see its changes; stash with a message.

### 3. The commit graph
- The whole history with lanes and colours, loaded as it scrolls, branch and tag labels on commits.
- Search by message, author, hash, and by changed file; filter to chosen branches.
- A commit's details: message and people, its changes with the diff viewer, the file tree at that commit.
- File history and blame from any file.

### 4. Rewriting history
Cherry-pick, revert, reset (soft, mixed, hard: the hard one asks), interactive rebase
(reorder, squash, fixup, reword, drop) by dragging rows, amend an older commit, reflog to undo.

### 5. Merges and conflicts
A banner while a merge, rebase or cherry-pick is in progress (continue, skip, abort), and a built-in
three-way conflict editor on the canvas: theirs, ours, result.

### 6. Remotes and GitHub
Pull with rebase or merge, push with tags, force with lease (asks), manage remotes, clone, open a
commit or branch on GitHub, create a pull request from a branch.

### 7. The rest of Fork
Git clean, patches (create, apply), bisect, Git LFS, git-flow, submodule update, worktrees, custom
commands, a command palette for every git action.

## How it is built

- Git commands run through the git CLI, as today (`git.rs`): it is what people already have, and
  it keeps credentials, hooks and config working. Fork does the same with its bundled git, and reads the
  repository itself to be fast.
- Reading a large history fast (phase 3) may need a native reader; `gix` (gitoxide) is the Rust option.
  A new dependency is a decision for the user, taken when phase 3 starts.
- Every action that loses work asks first, in the app's own modal (never an OS dialog).
- Each phase gets QA scenarios under `qa/scenarios/git/` with fixtures that use a local bare repo as
  the remote, so tests never need the network.
