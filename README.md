# herdr-routines

A [Herdr](https://herdr.dev) plugin that runs coding agents on a schedule.
A routine is a prompt, an agent (Claude, Codex, Gemini, OpenCode or any other
agent Herdr knows), a folder and a time. When the time comes, the plugin
opens a tab in the background, starts the agent there and hands it the
prompt. You can watch any run live, answer when the agent asks something and
keep talking to it afterwards.

Routines run as long as the Herdr server runs, also when no Herdr window is
open.

```
 Routines (3) │ Runs (41)                                      ● Scheduler running
──────────────────────────────────────────────────────────────────────────────────
    Name            Schedule                                     Agent   Next          Last run
 ●  Inbox clean up  Weekdays, hourly at :15 from 08:15 to 18:15  claude  in 12 min     ✓ Finished · 48 min ago
 ●  Dependency scan Mondays at 07:00                             codex   Mon 07:00     ✓ Finished · 6 d ago
 ○  Release notes   Monthly on the 1st at 09:00                  claude  off           never
```

## What it does

- **List of routines** with their schedule in words, the next time and the
  last run. Switch a routine on or off with one key, run it now, edit it,
  write its prompt in your editor, or create and delete routines.
- **Runs** of all routines, newest first: status, how it was started (on
  time, catch-up or by hand), how long it took. `⏎` jumps to the run's tab,
  `l` shows its saved output once the tab is closed.
- **Needs you:** when an agent asks a question or waits for an approval, the
  run is marked and Herdr shows a notification. Answer it in the tab; the
  run carries on. This also covers questions before the agent is ready, such
  as Claude asking whether to trust a new folder: the prompt goes in once
  you answered.
- **Missed times:** when Herdr or the computer was off or asleep at the
  time, each routine either runs once afterwards or skips to its next time,
  as you set it per routine.
- **No overlap:** a routine that is still running is not started again; the
  skipped time shows in the runs.
- **Time limit** per routine: a run still going after it is marked as timed
  out and you get a notification. Its tab stays open.
- **Tidy:** the tab of a finished run closes after an hour (configurable),
  unless you picked up the conversation in it.

## Install

Needs Herdr 0.9.0 or newer and Rust (`cargo`) to build.

```sh
herdr plugin install nolvane/herdr-routines --yes
```

Herdr builds the plugin and starts the scheduler. Open the list with the
plugin action "herdr-routines: open or close", or bind a key in
`~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+u"
type = "plugin_action"
command = "herdr-routines.open"
description = "herdr-routines"

[[keys.command]]
key = "prefix+shift+u"
type = "plugin_action"
command = "herdr-routines.open-split"
description = "herdr-routines (split)"
```

Update with `herdr plugin update`.

## Keys

In the list of routines:

| Key | Does |
| --- | --- |
| `↑` `↓` | Move |
| `⏎` | Jump to the last run's tab, or show its output when the tab is closed |
| `r` | Run now (asks first) |
| `space` | Switch on or off |
| `e` | Edit |
| `p` | Open the prompt in your editor (`$VISUAL`, `$EDITOR`, else `nano`) |
| `n` | New routine |
| `d` | Delete with its run history (asks first) |
| `l` | Output of the last run |
| `←` `→` | Switch between routines and runs |
| `S` | Start the scheduler when it is off |
| `q` | Close the list (asks first); routines keep running |

In the form, `↑` `↓` move between fields, `←` `→` pick a choice, typing
edits a text field, `Ctrl+U` clears it, `⏎` on "Prompt" opens your editor and
`Ctrl+S` or "Save" saves. `Esc` cancels.

## Routines are files

Each routine is a folder in the plugin's config directory
(`herdr plugin config-dir herdr-routines`), under `routines/<id>/`:

`routine.toml`

```toml
name = "Dependency scan"
enabled = true
# minute hour day month weekday, local time
schedule = "0 7 * * 1"
# Any kind `herdr agent start --kind` accepts
agent = "codex"
# Passed to the agent (optional)
args = []
folder = "~/code/app"
# "run-once" or "skip"
missed = "run-once"
time_limit_minutes = 120
```

`prompt.md` holds the prompt, as you would type it to the agent.

You, the list or another agent can create and change these files; the
scheduler reads them every few seconds. A routine with a mistake shows up in
the list with what is wrong.

**Schedules** are cron expressions with five fields in local time: `*`,
values, ranges `8-18`, steps `*/15`, lists `1,15`, and month and day names
(`jan`, `mon`). Day of week 0 and 7 are Sunday. The form offers every hour,
every day, weekdays and once a week, and any cron expression under "Custom".

**Unattended runs:** the agent runs with the settings and permissions it has
in that folder. If a routine should work without stopping for approvals,
give the agent the permission mode it needs through its own settings or
`args`, as you would when starting it yourself.

Runs are kept in the plugin's state directory
(`~/.local/state/herdr/plugins/herdr-routines/runs/`), 50 per routine.

## Settings

Optional, in `config.toml` in the plugin's config directory. See
[config.toml.example](config.toml.example): the workspace name, when
finished tabs close, notifications on finish, history length, icons and
where the list opens.

## Command line

```sh
herdr-routines routines   # all routines and their last run as JSON
herdr-routines fire <id>  # run a routine now
herdr-routines status     # whether the scheduler runs
```

The plugin actions "herdr-routines: restart the scheduler", "stop" and "status" do
the same from Herdr. The scheduler log is `routines.log` in the state
directory.

## Licence

MIT, see [LICENSE](LICENSE).
