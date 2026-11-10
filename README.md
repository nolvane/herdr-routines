# herdr-routines

A [Herdr](https://herdr.dev) plugin that runs coding agents and shell
commands on a schedule. A routine is a prompt, an agent (Claude, Codex,
Gemini, OpenCode or any other agent Herdr knows), a folder and a time. When
the time comes, the plugin opens a tab in the background, starts the agent
there and hands it the prompt. You can watch any run live, answer when the
agent asks something and keep talking to it afterwards. A routine can also
be a plain shell command or script, like a cron job, without any AI.

Routines run as long as the Herdr server runs, also when no Herdr window is
open. With `A` in the list, Herdr starts in the background at login, so
routines also run after a restart without opening Herdr.

```
 ◷ Scheduled routines    Routines (3)   Runs (41)                      ● scheduler on

    Routine                                     Schedule                  Last run
 ●  Inbox clean up                              Weekdays, hourly at :15   ✓ Finished
    claude · Sonnet · Auto · ~/mail             next in 12 min            48 min ago · took 3 min

 ●  Nightly backup                              Daily at 02:00            ✓ Finished
    shell · ./backup.sh --all · ~/code/app      next 02:00                10 h ago · took 4 min

 ●  Release notes                               Monthly on the 1st        –
    claude · Opus · Plan only · ~/code/app      off                       no run yet

 ⏎ edit  n new  r run now  space on/off  ? all keys  q close
```

The dot in front of a routine is green when it is on and red when it is
off; a yellow warning sign marks a routine with a problem.

## What it does

- **List of routines** with their schedule in words, the next time and the
  last run. Switch a routine on or off with one key, run it now, edit it,
  write its prompt, or create and delete routines. Works with the keyboard
  and the mouse, like the other Herdr plugins of this author; `?` shows all
  keys.
- **Permission mode, model and effort** per routine, picked from a list:
  for Claude Code the permission modes (Auto, Accept edits, Plan only and
  more), the models (Fable, Opus, Sonnet, Haiku or any typed name) and the
  effort; for Codex and Gemini their modes. Every agent that takes a model
  on its command line gets a model choice: the agent's own list where it
  can print one (Codex, Cursor, OpenCode, Kilo, Kiro, Devin, Pi, Grok,
  Antigravity; refreshed daily in the background), its documented aliases,
  models other routines use, or any typed name. New routines start in
  Auto, since nobody watches a scheduled run.
- **Shell commands:** choose "Shell command" as the agent and write the
  command or script instead of a prompt. It runs in its own tab in the
  folder, with your shell and its environment. Exit code 0 counts as
  finished; any other code marks the run as failed and Herdr shows a
  notification.
- **Create with Claude:** `n`, then "Describe it to Claude", opens a tab
  where Claude asks what the routine should do, writes it and checks that
  it loads. You then approve it in the list with `a`.
- **Your approval:** a routine runs only as you saved it in the list or
  approved it there. When anything else creates or changes its files (an
  agent, an editor outside the list), it waits, marked "Changed outside the
  list", until you press `a`, see what it would run (schedule, folder,
  agent, permission mode, every option and the prompt or command) and
  approve it with `y`. So an agent cannot quietly give a routine more
  rights or turn it into a shell command that runs unattended.
- **Prompt editor** in the list: type, paste, move with the arrows or the
  mouse, mark text with Shift and the arrows or by dragging, copy, cut and
  paste with `Ctrl+C`, `Ctrl+X`, `Ctrl+V`, undo with `Ctrl+Z`. `Esc`
  closes it and keeps what you wrote. `Ctrl+O` opens the prompt in your own
  editor instead.
- **Runs** of all routines, newest first: status, how it was started (on
  time, catch-up, retry or by hand), how long it took and what it came to.
  "n runs" next to a routine (or `h`) shows only its runs. `⏎` jumps to the
  run's tab, `l` shows its result and output, also after the tab is
  closed, and `f` opens the output file.
- **Output kept:** a shell command's whole output goes into a file; an
  agent's tab is saved every minute while it is open, so closing the tab by
  hand loses nothing.
- **Results:** each agent run is asked to end with a short summary, which
  the list shows next to the run. With "Earlier results" on, the next run
  starts with the summaries of the last ten runs (kept in one small file per
  routine), so a daily routine knows what it did before.
- **Check first:** an optional shell command that runs before the agent,
  such as "are there new files?". When it ends without an error the agent
  starts and gets its output; otherwise the run is noted as "Nothing to do"
  and costs nothing.
- **Retries:** a failed run (a command ending with an error, an agent that
  does not start) can be tried again up to three times a few minutes later.
- **Once, or when Herdr starts:** besides repeating schedules, a routine can
  run once on a date (then it switches itself off) or each time the Herdr
  server starts.
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
  out and you get a notification. Its tab stays open, or, if you choose
  "Stop the run", it is closed, which ends the agent or command.
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

`?` in the list shows every key. The main ones:

| Key | Does |
| --- | --- |
| `↑` `↓` | Move |
| `⏎` | Edit the routine (in Runs: go to the run's tab) |
| `n` | New routine: fill in the form, or describe it to Claude |
| `p` | Write the prompt |
| `r` | Run now (asks first) |
| `a` | Approve a routine changed outside the list (shows it first) |
| `space` | Switch on or off |
| `o` | Go to the tab of the last run |
| `l` | Result and output of the last run |
| `h` | Only this routine's runs (`Esc` shows all again) |
| `d` | Delete with its run history (asks first) |
| `←` `→` | Switch between routines and runs |
| `S` | Start the scheduler when it is off |
| `A` | Start Herdr at login, or not any more |
| `q` | Close the list (asks first); routines keep running |

The mouse works too: click a tab or a row, double click to open, scroll with
the wheel, click the keys in the footer.

In the form, `↑` `↓` move between fields, `⏎` opens the list of a choice
(or the prompt editor), `←` `→` step through a choice, typing edits a text
field, `Ctrl+U` clears it and `Ctrl+S` or "Save" saves. `Esc` goes back and
asks first when something changed.

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
# Optional, for claude, codex and gemini; left out means the agent's default
permission_mode = "auto"
# model = "<a model name the agent knows>"
# Optional, for claude: low, medium, high, xhigh or max
# effort = "high"
# More arguments for the agent's command line (optional)
args = []
folder = "~/code/app"
# "run-once" or "skip"
missed = "run-once"
time_limit_minutes = 120
# Optional: close the run's tab at the time limit (default: keep it)
# stop_at_limit = true
# Optional: try a failed run again, this many times, minutes apart
# retries = 2
# retry_delay_minutes = 10
# Optional, agents only: a shell command run first; the agent starts only
# when it ends without an error, and gets its output
# check_command = "git fetch -q && git log --oneline HEAD..@{u}"
# Optional, agents only: start each run with the last ten results
# remember = true
```

`prompt.md` holds the prompt, as you would type it to the agent. A shell
routine has `agent = "shell"` and its command in `command.sh` instead; it
runs in your `$SHELL`, line by line like a script.

You, the list or another agent can create and change these files; the
scheduler reads them every few seconds. A routine with a mistake shows up in
the list with what is wrong. A change made outside the list takes effect
once you approve it with `a` in the list (see "Your approval" above). The
folder name is the routine's id: lower case letters, digits and dashes.

**Schedules** are cron expressions with five fields in local time: `*`,
values, ranges `8-18`, steps `*/15`, lists `1,15`, and month and day names
(`jan`, `mon`). Day of week 0 and 7 are Sunday. Two more forms:
`@once 2026-12-24 08:15` runs once (the routine then switches itself off)
and `@start` runs each time the Herdr server starts. The form offers every
hour, every day, weekdays, once a week, once on a date, when Herdr starts,
and any cron expression under "Custom".

**Permission modes** become the agent's own flags:

| Agent | `permission_mode` | Flag |
| --- | --- | --- |
| claude | `auto`, `acceptEdits`, `plan`, `dontAsk`, `bypassPermissions`, `manual` | `--permission-mode <mode>` |
| codex | `auto`, `read-only`, `full-access` | `--sandbox workspace-write --ask-for-approval never`, `--sandbox read-only`, `--dangerously-bypass-approvals-and-sandbox` |
| gemini | `auto_edit`, `yolo` | `--approval-mode <mode>` |

Left out, the agent runs with the settings and permissions it has in that
folder. With a mode that asks before changes, a run waits in its tab until
you answer. Other agents take their flags through `args`. For Claude Code,
Codex and Gemini, `args` may not set permissions (such as
`--dangerously-skip-permissions` or `--sandbox`): that is what
`permission_mode` is for, so the list always shows the real mode. The list
shows `args` under the routine's name.

Runs are kept in the plugin's state directory
(`~/.local/state/herdr/plugins/herdr-routines/`), 50 per routine: under
`runs/<routine id>/`, each run has a record `<run id>.json` (status,
times, result) and next to it its output (`.log` from the tab, `.out` from
a shell command), the prompt it got (`.prompt`) and, for shell commands,
the script. The run id is its start time, such as `20261110-193702-582`.
The routine id is the folder name, made from the first name and never
changed, so renaming a routine keeps its runs. `memory/<routine id>.md`
holds the earlier results of a routine that remembers. The plugin's
folders and files are readable only by you, since prompts, commands and
saved output can hold secrets.

**Start at login** (`A` in the list) writes a launch agent
(`~/Library/LaunchAgents/herdr-routines.herdr-server.plist`) on macOS or a
systemd user service on Linux that runs `herdr server` at login. It takes
effect at the next login; turning it off removes the entry and leaves a
running server alone.

## Settings

Optional, in `config.toml` in the plugin's config directory. See
[config.toml.example](config.toml.example): the workspace name, when
finished tabs close, notifications on finish, history length, icons and
where the list opens.

## Command line

```sh
herdr-routines routines   # all routines and their last run as JSON
herdr-routines fire <id>  # run an approved routine now
herdr-routines status     # whether the scheduler runs
```

The plugin actions "herdr-routines: restart the scheduler", "stop" and "status" do
the same from Herdr. The scheduler log is `routines.log` in the state
directory.

## Licence

MIT, see [LICENSE](LICENSE).
