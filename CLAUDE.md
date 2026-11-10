# herdr-routines

A [Herdr](https://herdr.dev) plugin that runs coding agents on a schedule,
each run in its own tab. What it does and how users install it:
[README.md](README.md). Why it is built this way:
[docs/decisions.md](docs/decisions.md).

## Stack

- Rust (edition 2021), one binary built by `scripts/build.sh` into the plugin
  root, where `herdr-plugin.toml` starts it.
- `ratatui` for the list, `chrono` for local time, `serde`, `serde_json` and
  `toml` for the files.
- Everything goes through the `herdr` command line (`HERDR_BIN_PATH`); the
  plugin never starts an agent itself and never reads a login or token.
- Needs Herdr 0.9.0 or newer (`min_herdr_version` in `herdr-plugin.toml`).

## Layout

- `scheduler.rs`: the background loop, when a routine is due, the pid file.
- `runner.rs`: starting a run and following it (states, notifications,
  closing finished tabs), the check command, results and memory.
- `herdr.rs`: the Herdr calls. `routine.rs`: routines and settings.
  `agent.rs`: permission modes, models and effort per agent and their
  flags. `models.rs`: model lists the agents print. `store.rs`: run
  records. `schedule.rs`: cron, `@once` and `@start`; `cron.rs`: cron
  itself. `shell.rs`: commands in the login shell with a time limit.
  `autostart.rs`: Herdr at login. `paths.rs`: where files go.
  `approval.rs`: the owner's approval of routines changed outside the
  list.
- `app.rs`: the list's state, keys and mouse. `ui.rs`: drawing, in the look
  of the Git Panel and Multiaccount plugins (keep them alike).
  `form.rs`: the edit form. `editor.rs`: the prompt editor. `pane.rs`: the
  `open` action.

## Conventions

- Code, comments and docs are in English. Texts in the list are English too.
- Notable technical decisions go into `docs/decisions.md` (decision, why,
  date).
- The repository is public: no personal data, no secrets, no private setup.
  Examples and tests use made-up routines and folders.
- Deleting a routine, running one by hand and closing the list ask for `y`
  first. Switching a routine on or off does not, since the same key undoes it.
- Files that other processes read are written through `paths::write_atomic`
  (owner-only, never through a link). New folders use
  `paths::create_private_dir`.
- A routine runs only when `Routine::can_run` holds: no problem and
  approved (`approval.rs`). Anything the list saves for the owner goes
  through `Routine::save_approved` or `approval::approve_file`; plain
  `Routine::save` leaves the routine waiting for approval.
- Herdr calls about a run's agent use `Run::agent_target`; calls about its
  pane check `own_pane` first (same tab and terminal).
- The scheduler and a `fire` process both write run records: a record in
  `Starting` belongs to the process that launches it; the scheduler only
  takes over records that are `Running` or `NeedsYou`, or `Starting` for
  longer than 10 minutes.
- Pane, tab and workspace ids only mean something on the Herdr server that
  made them. Every run records its server (`Run::server`); only that
  server's scheduler touches its tab.

## Checks

```sh
./scripts/check.sh            # default stage: merge
./scripts/check.sh --stage quick
```

Every stage runs the same steps: gitleaks secret scan, `cargo fmt --check`,
`cargo clippy` with warnings as errors, the unit tests and the release build.
There are no integration tests in the checks, since they need a running
Herdr server and an agent that is logged in.

To try a change by hand without touching your own Herdr session, use a
throwaway session and separate config and state folders:

```sh
herdr --session rttest server &          # headless test server
export HERDR_SOCKET_PATH=~/.config/herdr/sessions/rttest/herdr.sock
export HERDR_PLUGIN_CONFIG_DIR=/tmp/rt/config HERDR_PLUGIN_STATE_DIR=/tmp/rt/state
target/debug/herdr-routines start        # scheduler for the test server
```

Create a routine with a cheap prompt ("Reply with OK"), open the list in a
pane of the test session (`herdr pane run`, `herdr pane read`), and stop
everything afterwards with `herdr-routines stop`,
`herdr session stop rttest` and `herdr session delete rttest`.

## Non-goals

- No headless runs: every run is an interactive agent in a tab.
- No scheduler of its own outside Herdr: routines run while the Herdr
  server runs. The optional "start at login" entry (`autostart.rs`) starts
  `herdr server`, not the plugin.

## Releasing

**Version.** It lives in two places that must always match: `version` in
`herdr-plugin.toml` and `[package] version` in `Cargo.toml`. After changing
it, run `cargo build --release` once (without `--locked`) so `Cargo.lock`
picks up the new version, then `./scripts/check.sh --stage release`.

**What ships.** Users install from GitHub, and `herdr plugin update` fetches
the newest commit of the ref recorded at install and builds it. A merge to
`main` is therefore what reaches users at their next update. After an update,
the plugin action "herdr-routines: restart the scheduler" (or a Herdr restart)
starts the new scheduler; an open list keeps the old binary until it is
closed and opened again.

**Try a release before it reaches users.** Build a local checkout with
`sh scripts/build.sh` and `herdr plugin link "$PWD"`, or install a pushed
branch with `herdr plugin install nolvane/herdr-routines --ref <branch> --yes`.
Back to the normal install: `herdr plugin unlink herdr-routines`, then
`herdr plugin install nolvane/herdr-routines --yes`.

**Changelog and GitHub release.** `CHANGELOG.md` follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). A release ends with
an annotated tag `vX.Y.Z` on the released commit and a GitHub release whose
notes are the version's section of `CHANGELOG.md`.
