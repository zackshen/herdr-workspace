# herdr-workspace

A [Herdr](https://herdr.dev) plugin that creates a workspace **or Git worktree**
and applies a **layout profile** from a centered popup.

Bind an action, fill in the form, pick a profile card, Save. Herdr gets a new
workspace whose tabs, pane splits, pane names, and startup commands come from
the plugin config.

```
┌─ New workspace ──────────────────────────────────────────────────────┐
│ Directory                                                            │
│ ~/Documents/Repos/herdr-workspace/                                   │
│   ~/Documents/Repos/herdr-workspace/                                 │
│                                                                      │
│ Name                                                                 │
│ herdr-workspace                                                      │
│                                                                      │
│ ┌ Solo ──────────┐ ┌ Pair ──────────────┐ ┌ Full ─────────────────┐  │
│ │ One tab, one   │ │ Shell on the left, │ │ Code tab (editor,     │  │
│ │ shell.         │ │ agent on the right.│ │ agent, logs) + ops.   │  │
│ └────────────────┘ └────────────────────┘ └───────────────────────┘  │
│                                                                      │
│                                          [ Cancel ]       [ Save ]   │
└──────────────────────────────────────────────────────────────────────┘
```

## Requirements

- [Herdr](https://herdr.dev) ≥ 0.8.0
- [fzf](https://github.com/junegunn/fzf) provides inline fuzzy directory completion.
- [fd](https://github.com/sharkdp/fd) indexes `~/projects` and `~/devdash` to a depth of four.
- [zoxide](https://github.com/ajeetdsouza/zoxide) supplies optional directory history.
- Rust toolchain only if you build from source. `herdr plugin install`
  downloads a prebuilt binary.

## Install

Prebuilt binaries, no Rust toolchain needed:

```bash
herdr plugin install zackshen/herdr-workspace
```

`[[build]]` downloads `herdr-workspace` for this platform from the matching
[GitHub Release](https://github.com/zackshen/herdr-workspace/releases) into `bin/`.

To update, reinstall:

```bash
herdr plugin uninstall herdr-workspace && herdr plugin install zackshen/herdr-workspace
```

Or link a local checkout (`plugin link` does **not** run `[[build]]`):

```bash
cargo build --release
mkdir -p bin && cp target/release/herdr-workspace bin/
herdr plugin link /path/to/herdr-workspace
```

On macOS, replace the binary atomically after a rebuild:

```bash
install -S -m 755 target/release/herdr-workspace bin/herdr-workspace
```

Herdr does **not** bind keys declared in a plugin manifest. Add a binding to
`~/.config/herdr/config.toml` and reload:

```toml
[[keys.command]]
key = "prefix+shift+n"
type = "plugin_action"
command = "herdr-workspace.open"
description = "New workspace from layout profile"

[[keys.command]]
key = "prefix+shift+w"
type = "plugin_action"
command = "herdr-workspace.worktree"
description = "New worktree from layout profile"
```

```bash
herdr server reload-config
```

## Popup flow

1. **Directory** starts empty.
   Completion uses directory history, immediate child directories, and the original bounded project index.
   The index covers `~/projects` and `~/devdash` to a depth of four.
   The index excludes `.git`, `node_modules`, `target`, and `.venv`.
   The form does not scan the complete home tree.
   Matching uses directory names first, then complete history paths.
   Four rows remain visible, with scrolling through additional matches for a nonempty query.
   Manual paths and the directory picker work without search dependencies.

   1. Type a path or a fuzzy directory name.
   2. Use Up/Down or `Ctrl-J`/`Ctrl-K` to select a result.
   3. Press `Tab` to copy the selected path into Directory.
   4. Press `Enter` to commit the selected directory or a valid manual path.
   5. Click **Browse (Ctrl-O)** to open the directory picker.

2. **Name** — workspace label. Defaults to the last path component. `Enter`
   shows the profile cards.
3. **Profiles** — one card per `[[profiles]]` in config. Arrow keys or click
   to select.
4. **Cancel** closes the popup. **Save** creates the workspace and applies
   the selected profile.

`Esc` is Cancel.

### Directory picker

The picker reads one directory at a time.
It includes hidden directories, ignored directories, and directory symlinks.
It does not require `fd`, `fzf`, or `zoxide`.
The **Browse (Ctrl-O)** button sits beside Directory.
`Ctrl-O` opens the same picker when Directory has focus.
The `./` entry represents the current directory.
Cancel keeps the existing Directory input, Name, and profile.

1. Click **Browse (Ctrl-O)** to open the picker.
2. Use Up/Down to highlight a directory.
3. Press Right to open the highlighted directory.
4. Press Left or Backspace to open the parent directory.
5. Press Enter to choose the highlighted directory.
6. Press Escape to return without a selection.


## Worktree popup

`herdr-workspace.worktree` runs in the **already open** git workspace. It
creates a linked worktree instead of a brand-new directory:

1. **Branch** — new branch name (`feature/foo`).
2. **Base ref** — type to fuzzy-search local branches, remotes, and tags.
   `Tab` accepts a suggestion. Defaults to `origin/main` when that exists.
3. **Profiles** — same cards as New workspace.
4. **Save** calls `herdr worktree create`, applies the profile, then fetches
   and fast-forwards the new checkout onto the latest base.

The new workspace label is the branch name.

## Layout profiles

How to write profiles, the YAML structure, split rules, and named agents:
**[docs/profiles.md](docs/profiles.md)**.

Config lives in the plugin config directory (not the plugin checkout):

```bash
herdr plugin config-dir herdr-workspace
```

On first run, missing `config.yaml` is seeded from
[`config.example.yaml`](config.example.yaml). Edit that file and reopen the
popup — no rebuild.

```yaml
profiles:
  - id: pair
    name: Pair
    description: |
      Shell on the left, coding agent on the right.
    tabs:
      - name: main
        panes:
          - id: shell
            name: shell
          - id: agent
            name: agent
            split_from: shell
            direction: right
            ratio: 0.5
            agent: pair
            kind: claude
```

## How it works

Herdr actions run on the server with **no TTY**, so the form cannot run in
the action itself:

1. `herdr-workspace.open` opens the centered `form` popup (`placement = "popup"`).
   `herdr-workspace.worktree` opens the `worktree` popup instead.
2. Inside the popup, the binary draws a ratatui form. Save for a workspace
   calls `herdr workspace create`; Save for a worktree calls
   `herdr worktree create --workspace <current> --branch … --base …`. Then
   `tab create` / `tab rename`, `pane split`, `pane rename`, `pane run` or
   `agent start`. A worktree Save also `git fetch`es and fast-forwards onto
   the chosen base.
3. The popup closes on Cancel, Esc, or after a successful Save.

## Changelog

See [CHANGELOG.md](CHANGELOG.md). Regenerated from conventional commits:

```bash
brew install git-cliff   # or: cargo install git-cliff
scripts/changelog.sh                 # all tagged releases
scripts/changelog.sh --unreleased    # since the latest tag
scripts/changelog.sh --tag v0.1.1    # as if that tag already exists
```

Pushing a `v*` tag also writes those notes onto the GitHub Release.

## License

MIT
