# tsk

A plain-text task TUI in the spirit of [PlainTasks](https://github.com/aziz/PlainTasks) and TaskPaper — written in Rust, and designed to run **either standalone in any terminal or as a [herdr](https://herdr.dev) plugin**.

> **Status: pre-release.** The repository is scaffolding only right now. Nothing described under *Usage* below is implemented yet; it documents the intended design. Watch the repo or check the issues for progress.

## Why

PlainTasks got the format right: your tasks are a plain text file you can read, diff, grep, and commit. No database, no sync service, no lock-in. `tsk` keeps that file format and gives it a terminal UI, so the same list works in your editor, in `git log`, and in a pane.

## Design

`tsk` is a normal TUI binary first. herdr is detected at runtime, never required at compile time:

- Outside herdr, it's an ordinary terminal application.
- Inside herdr, `HERDR_ENV=1` unlocks additive integration — command-palette actions, event hooks, link handling, pane control — and none of it changes the core behavior.

One binary, no feature flags, identical task files either way.

## Task format

The plain-text format follows the PlainTasks/TaskPaper conventions:

```
Inbox:
 ☐ Write the parser @today
 ☐ Decide on the config schema @next
 ✔ Pick a name @done(2026-09-02)

Someday:
 ☐ Sync back to an external tracker
```

Projects end in `:`, tasks are prefixed with a box, tags are `@word` and may take a `@tag(value)` argument. Everything is a file on disk — `tsk` never owns your data.

## Usage

### Standalone

```
cargo install --git https://github.com/chrisg32/tsk
tsk ~/todo.todo
```

Prebuilt binaries will be attached to GitHub Releases so a Rust toolchain isn't required.

### As a herdr plugin

```
herdr plugin install chrisg32/tsk
```

The repository carries the `herdr-plugin` topic and a `herdr-plugin.toml` at its root, which is all herdr
needs — the repository name itself is not significant. The plugin declares a pane running the same `tsk`
binary, plus actions invocable from herdr's command palette. Configuration lives in `HERDR_PLUGIN_CONFIG_DIR` under herdr and in `~/.config/tsk` standalone.

## Development

```
cargo build
cargo test
cargo run -- path/to/list.todo
```

## Credit

The format and much of the interaction design are owed to [PlainTasks](https://github.com/aziz/PlainTasks) by Aziz Köksal and contributors.

## License

Intended to be MIT; the `LICENSE` file lands with the initial scaffolding.
