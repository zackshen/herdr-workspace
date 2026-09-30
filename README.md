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
- [fzf](https://github.com/junegunn/fzf) for inline fuzzy directory suggestions.
- [fd](https://github.com/sharkdp/fd) for unvisited directories under
  `~/projects` and `~/devdash`; [zoxide](https://github.com/ajeetdsouza/zoxide)
  adds visited-directory history. Either source may be absent.
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

1. **Directory** — starts empty. Type a path fragment to filter zoxide's
   visited directories, nearby children, and unvisited directories under
   `~/projects` and `~/devdash` (up to four directory levels deep). The fd
   scan runs once when the form opens, not across all of `$HOME`. fzf ranks
   matches inline; four rows are visible at a time, with `Ctrl-J`/`Ctrl-K` or
   Up/Down scrolling through further results. Type more of a name (for example,
   `web_server`) to narrow crowded results. `Tab` accepts a suggestion into
   the field; `Enter` commits the highlighted directory and moves to Name.
   Manual paths still work when fd or zoxide is unavailable.
2. **Name** — workspace label. Defaults to the last path component. `Enter`
   shows the profile cards.
3. **Profiles** — one card per `[[profiles]]` in config. Arrow keys or click
   to select.
4. **Cancel** closes the popup. **Save** creates the workspace and applies
   the selected profile.

`Esc` is Cancel.

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
