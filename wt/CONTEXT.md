# `wt` — Design Spec

A Rust CLI for creating and managing git worktrees as siblings under a parent directory.

## Layout

```
PARENT/
├── .bare/                       # bare git repo (core.bare = true)
│   └── wt-config.toml           # share-list, post_create, optional remote override
├── .git                         # file: "gitdir: ./.bare"
├── .env                         # shared files live here; symlinked into worktrees
├── main/                        # worktree for default branch
├── feature/
│   └── auth/                    # worktree for feature/auth (nested, mirrors git refs)
└── fix/
    └── login-bug/
```

The bare repo holds all git state. Every branch — including the default — is a worktree under `PARENT/`. Branch names with `/` map to nested directories (mirrors `.git/refs/heads/` storage). This is isomorphic to git's own ref layout, so collisions that git forbids cannot arise in the filesystem either.

## Commands

| Command | Behavior |
|---|---|
| `wt init <url> [parent]` | Clones `<url>` bare into `parent/.bare`, creates a worktree for the default branch at `parent/<default-branch>/`. |
| `wt adopt [path]` | Transforms an existing regular clone in place. Refuses if dirty. Interactive checklist for shared files. Atomic. |
| `wt new <branch> [base]` | Creates/attaches a worktree for `<branch>`. Auto-fetches. Handles new branches, remote-only branches, and existing local branches uniformly. `[base]` defaults to `origin/HEAD`; `.` = HEAD of the current worktree. Errors only if `<branch>` already has a worktree. |
| `wt rm <branch>` | Removes worktree + branch + empty parent dirs + prunes. Refuses on dirty / unpushed / CWD-inside without `--force`. |

No `wt ls`, no `wt checkout`, no shell wrapper. Use `find`, `cd`, and `git` for the rest. Each worktree is a normal git working tree — `git` commands work as usual from inside any of them.

## `wt new` semantics

Given `wt new <branch> [base]`:

| Situation | Behavior |
|---|---|
| Branch doesn't exist anywhere | Create branch from `[base]` (default `origin/HEAD`), create worktree. |
| Branch exists only on `origin` | Create local branch tracking `origin/<branch>`, create worktree. |
| Branch exists locally, no worktree | Create worktree on the existing branch. `[base]` ignored. |
| Branch already has a worktree | Error, print the existing path. |

`[base]` accepts:
- omitted → `origin/<default-branch>` (after fetch)
- `.` → `HEAD` of the worktree the command was run from (errors if not inside a worktree)
- any other ref → passed through to git

`--no-fetch` skips the auto-fetch.

## `wt adopt` flow

1. Refuse if `git status --porcelain` is non-empty (any tracked modification or untracked-unignored file).
2. Refuse if `.git` and `PARENT` cross filesystems (the move of `node_modules` etc. would become a copy).
3. Scan working tree for ignored files via `git check-ignore`. Present in `dialoguer::MultiSelect`; pre-check common candidates (`.env*`).
4. Do all work in `PARENT/.wt-adopt-tmp/`, then a single `rename(2)` at the end so a crash leaves either the old layout or the new — never a half-state.
5. Move tracked files into `PARENT/<current-branch>/`.
6. Move *unshared* ignored files into `PARENT/<current-branch>/` (so `node_modules` etc. aren't orphaned).
7. Move *shared* ignored files into `PARENT/` and symlink them from `PARENT/<current-branch>/`.
8. Convert `.git` to `PARENT/.bare/` (set `core.bare = true`, register the worktree).
9. Write `PARENT/.bare/wt-config.toml` with the share-list.

## `wt rm` defaults

1. Refuse if worktree is dirty or branch has unpushed commits, unless `--force`.
2. Refuse if CWD is inside the worktree.
3. `git worktree remove PARENT/<branch>`.
4. Delete the local branch. `--keep-branch` to preserve it.
5. Walk up removing empty parent dirs until hitting `PARENT` or a non-empty dir.
6. `git worktree prune`.

## `wt-config.toml`

Lives at `PARENT/.bare/wt-config.toml`. Not tracked in git (share-list is per-developer; making it branch-aware is a rabbit hole).

```toml
shared = [".env", ".env.local"]   # files in PARENT/, symlinked into every worktree
post_create = "pnpm install"      # optional; runs in new worktree's cwd
remote = "upstream"               # optional; overrides auto-detected branch.<default>.remote
```

- **`shared`**: applied automatically on every `wt new` (after the worktree is added, before `post_create`). Symlinks, not hardlinks or copies — edits flow both ways, single source of truth.
- **`post_create`**: optional command, run in the new worktree's cwd with stdout/stderr streamed. Failure leaves the worktree in place — user can fix and re-run setup, or `wt rm && wt new`.
- **`remote`**: optional. If unset, the tool reads `git config --get branch.<default-branch>.remote` and falls back to `origin`.

## Stack

- **Rust** for the strict typing on failure-prone paths (`wt adopt` atomicity), static-binary distribution, and the `clap` derive ergonomics.
- **Shell out to `git`** via `std::process::Command` (libgit2's worktree support is the weakest part of the C library, and this tool is entirely about worktrees). Parse `--porcelain` outputs where needed; let exit codes propagate.

Crates:

- `clap` (derive) — CLI parsing, `--help`, completions.
- `serde` + `toml` — config parsing.
- `anyhow` — error propagation in `main`.
- `dialoguer` — interactive share-list checklist in `wt adopt`.
- `walkdir` — recursive directory operations.
- `std::fs` + `std::process` — everything else.

## Implementation notes

1. **Default-branch detection**: `git symbolic-ref refs/remotes/origin/HEAD` → `refs/remotes/origin/main`. Cache during `wt init`/`wt adopt`.
2. **`wt rm` of nested branch** (e.g. `feature/auth`): after removing `PARENT/feature/auth/`, walk up removing empty dirs until `PARENT` or a non-empty dir.
3. **Symlink targets**: relative, not absolute. `PARENT/feature/auth/.env → ../../.env`, not `/Users/.../PARENT/.env`. Keeps the parent dir portable.
4. **Shell completions**: `clap_complete` generates bash/zsh/fish at build time. Ship them in the release tarball.
5. **Distribution**: GitHub releases with prebuilt binaries for darwin-arm64, darwin-x64, linux-x64; `cargo install wt` as fallback.

## Out of scope for v1

- `wt ls` — `find PARENT -type d -not -path '*/.bare/*'` is fine.
- `wt checkout` — folded into `wt new`.
- Shell wrapper / auto-`cd` — `cd PARENT/<branch>` is fine.
- Detached-HEAD worktrees (commit/tag checkouts) — break the "dir name = branch name" model.
- Bulk sweep of merged branches — manual `wt rm` per branch.
- Multiple remotes beyond the auto-detected default.
- Tracked-in-repo project config (share-list is per-developer).
