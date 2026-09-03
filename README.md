# tsk

Plain-text tasks in your terminal, in the spirit of [PlainTasks](https://github.com/aziz/PlainTasks) and TaskPaper. Written in Rust. Runs **standalone in any terminal or as a [herdr](https://herdr.dev) plugin** — same binary, same files.

```
Inbox:  2
  ☐ Write the parser @today
  ☐ Decide on the config schema @next
  ✔ Pick a name @done(26-09-02 10:15)

Someday:  1
  ☐ Sync back to an external tracker
```

## Why

PlainTasks got the format right: your tasks are a text file you can read, diff, grep, and commit. No database, no sync service, no lock-in. `tsk` keeps that file format and gives it a terminal UI, so the same list works in your editor, in `git log`, and in a pane next to your shell.

## Install

**Standalone** (needs a Rust toolchain):

```
cargo install --git https://github.com/chrisg32/tsk
tsk ~/todo.todo
```

**As a herdr plugin:**

```
herdr plugin install chrisg32/tsk
```

The install step downloads a prebuilt binary for your platform from the [releases](https://github.com/chrisg32/tsk/releases) when one exists for that version, and otherwise builds from source with `cargo`. Bind a key to the `tsk.open` action (or `tsk.open-split` / `tsk.open-tab`) in your herdr config:

```toml
[[keys.command]]
key = "prefix+t"
type = "plugin_action"
command = "tsk.open"
description = "open tasks"
```

## Which file opens

`tsk FILE` opens that file (created on first save). With no argument, in order: `$TSK_FILE`, the `file` in your config, a `*.todo` / `*.taskpaper` / `*.tasks` file in the current directory, the last file you opened, then `~/todo.todo`. `tsk which` tells you what it would pick and why.

Under herdr the `open` actions pass the workspace's directory along, so a per-project `todo.todo` is found automatically.

## Keys

Press `?` inside `tsk` for this list.

| | |
|---|---|
| `j` `k` `↑` `↓` · `gg` `G` · `[` `]` | move · top/bottom · previous/next project |
| `Enter` `e` `a` · `i` | edit line, cursor at end · at start |
| `o` `O` · `m` · `p` | new task below/above · new note · new project |
| `Space` `x` · `c` · `s` · `t` | toggle done · cancelled · `@started` · `@today` |
| `1` `2` `3` · `@` | `@critical` / `@high` / `@low` · add or remove any tag |
| `Tab` `S-Tab` · `J` `K` | indent/outdent block · move block down/up |
| `d` `D` · `A` · `u` `U` | delete line/block · archive done tasks · undo/redo |
| `z` `Z` · `h` | fold project / fold all · hide or show done tasks |
| `/` `n` `N` · `L` | search, next, previous · open link on the line |
| `w` `R` · `q` `Q` | save / reload · quit / quit discarding changes |

Editing uses the usual line-editor keys: `←` `→` `Home` `End`, `Ctrl-a`/`Ctrl-e`, `Ctrl-w` and `Alt-←`/`Alt-→` for words, `Ctrl-k`/`Ctrl-u` to kill to the end/start. `Enter` commits, `Esc` cancels. An empty commit deletes the line.

Every change is saved immediately (turn `autosave` off to save with `w` instead). If another program changes the file, `tsk` reloads it; if you both have changes, it tells you and leaves yours alone until you press `R`.

## Format

The file format is PlainTasks/TaskPaper:

- A line ending in `:` is a **project**. Projects nest by indentation and may carry tags: `Work: @focus`.
- A line starting with a bullet is a **task**: `☐` open, `✔` done, `✘` cancelled. `-`, `+`, `[ ]`, `[x]` and the other PlainTasks bullets are read too; new tasks use the bullets from your config.
- `@tag` and `@tag(value)` anywhere in a task. `@today`, `@critical`, `@high`, `@low`, `@due(date)` (red when overdue), `@started`, `@done`, `@cancelled`, `@lasted` are highlighted; anything else is just a tag.
- Everything else is a **note** belonging to the task or project above it.

Marking a task done appends `@done(26-09-02 14:30)`; if it had `@started(...)`, `@lasted(2h05m)` is added too. `A` moves done and cancelled tasks (with their notes and subtasks) to the top of an `Archive:` project, each tagged `@project(Parent / Child)` so you can see where it came from.

Files are written exactly as they were read except for the lines you changed — indentation style (tabs or spaces), trailing newline, and CRLF are all preserved.

## Configuration

`tsk config path` prints the location (`~/.config/tsk/config.toml`, or the plugin config dir under herdr); `tsk config init` writes a commented example. Every key is optional:

```toml
file = "~/todo.todo"          # default file when none is given
open_bullet = "☐"
done_bullet = "✔"
cancelled_bullet = "✘"
date_format = "%y-%m-%d %H:%M" # inside @done(...), @started(...), ...
indent = "tab"                 # for files with no indentation yet: "tab" or a number
autosave = true
mouse = true                   # unset: on standalone, off under herdr
display_indent = 2             # screen columns per level
archive_project = "Archive"
show_done = true               # toggle at runtime with h
```

## How the herdr integration works

`tsk` is an ordinary TUI first. herdr is detected at runtime from the environment it injects (`HERDR_ENV=1`, `HERDR_BIN_PATH`, ...), never required at compile time. Under herdr:

- config and state move to `HERDR_PLUGIN_CONFIG_DIR` / `HERDR_PLUGIN_STATE_DIR`,
- mouse capture defaults off so herdr's own mouse handling keeps working,
- the `open` actions call back into herdr (`herdr plugin pane open`) to open the pane with the right working directory.

The [manifest](herdr-plugin.toml) declares one pane (`main`) and three actions. herdr discovers the plugin through the repository's `herdr-plugin` topic; the repository name itself doesn't matter.

For local development: `cargo build --release && mkdir -p bin && cp target/release/tsk bin/ && herdr plugin link "$PWD"`.

## Development

```
cargo test
cargo run -- examples/todo.todo
```

CI runs `cargo fmt --check`, `clippy -D warnings`, and the tests on Linux and macOS. Pushing a `v*` tag builds release binaries for macOS (arm64, x86_64) and Linux (x86_64, arm64) and attaches them to a GitHub release; the tag must match the version in both `Cargo.toml` and `herdr-plugin.toml`.

## Credit

The format and much of the interaction design are owed to [PlainTasks](https://github.com/aziz/PlainTasks) by Aziz Köksal and contributors.

## License

[MIT](LICENSE)
