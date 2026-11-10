# herdr-routines

A [Herdr](https://herdr.dev) plugin that runs coding agents and shell
commands on a schedule. A routine is a prompt, an agent (Claude, Codex,
Gemini, OpenCode or any other agent Herdr knows), a folder and a time. When
the time comes, the plugin opens a tab in the background, starts the agent
there and hands it the prompt. You can watch any run live, answer when the
agent asks something and keep talking to it afterwards. A routine can also
be a plain shell command or script, without any AI, and such a command can
also be a classic cron job that your system's cron runs.

Routines run as long as the Herdr server runs, also when no Herdr window is
open. Herdr can also start in the background at login (the list asks once,
and the "autostart" chip at the top or `A` switches it), so routines also
run after a restart without opening Herdr. Classic cron jobs run whenever
the computer is on. The "routines" chip at the top (or `S`) switches all of
them on or off at once.

```
 ◷ Scheduled routines    Routines (3)   Cron jobs (2)   Runs (41)     ● autostart on   ● routines on

    Routine                                     Schedule                  Last run
 ●  Inbox clean up                              Weekdays, hourly at :15   ✓ Finished  12 runs ›
    claude · Sonnet · Auto · ~/mail             next in 12 min            48 min ago · took 3 min
 ●  Nightly backup                              Daily at 02:00            ✓ Finished  30 runs ›
    shell · ./backup.sh --all · ~/code/app      next 02:00                10 h ago · took 4 min
 ●  Release notes                               Monthly on the 1st        –
    claude · Opus · Plan only · ~/code/app      off                       no run yet

 Inbox clean up ──────────────────────────────────────────────────────────────────────
 Schedule       Weekdays, hourly at :15 · next 10:15
 Agent          claude · Auto · Sonnet
 Folder         ~/mail
 Time limit     30 minutes, then stops · retries once after 5 min
 Last run       ✓ Finished · 48 min ago · 3 newsletters archived, 2 drafts written
 Prompt         Sort the new mail in the inbox: archive newsletters,
                draft replies to questions …

 ⏎ edit  n new  r run now  space on/off  ? all keys  q close
```

The dot in front of a routine is green when it is on and red when it is
off; a yellow warning sign marks a routine with a problem. In a tall enough
window, the selected routine's details show under the list.

## What it does

- **List of routines** with their schedule in words, the next time and the
  last run. Switch a routine on or off with one key, run it now, edit it,
  write its prompt, copy it (the copy starts switched off), or create and
  delete routines. The edit form is grouped into parts: the routine, its
  schedule, what happens when it runs long or fails, the agent extras and
  the prompt, of which it shows the first lines. Works with the keyboard
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
- **Classic cron jobs** have their own tab. `n`, then "Classic cron job",
  creates one: a shell command that the system's cron runs from your
  crontab, also when Herdr is closed. It is edited, switched and copied like
  a routine, and its runs (output, result, failures, retries, alerts) show
  in Runs like any other. The plugin keeps these jobs in a marked block of
  your crontab and rewrites only that block (after a backup in the state
  folder). On macOS, a cron job that works in Documents, Desktop or
  Downloads needs Full Disk Access for `/usr/sbin/cron` (System Settings,
  Privacy & Security, Full Disk Access).
- **Your other cron jobs** show in the Cron jobs tab too (◇). `⏎` on one (or
  `C` for all) offers: keep it as a cron job but manage it here, move it to
  Herdr (it runs in a tab instead), or copy it to Herdr switched off. Moving
  comments the old crontab line out after a backup, so nothing runs twice.
  Variables set in the crontab (such as `PATH`) come along.
- **Create with Claude:** `n`, then "Describe it to Claude", opens a tab
  where Claude asks what the routine should do, writes it and checks that
  it loads. You then approve it in the list with `a`.
- **Your approval:** a routine runs only as you saved it in the list or
  approved it there. When anything else creates or changes its files (an
  agent, an editor outside the list), it waits, marked "Changed outside the
  list", until you press `a`, read what it would run (every setting, every
  option and the whole prompt or command) to the end and approve it with
  `y`. When a waiting routine's time comes, Herdr tells you once. So an
  agent that writes routine files the normal way cannot make the
  scheduler run something you have not seen, such as a stronger
  permission mode or a shell command. It does not cover the agent's own
  settings in the routine's folder (such as `.claude/settings.json`),
  which the agent reads at start like in any other session.
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
- **Notification or popup** per routine ("Tell me by"): Herdr's usual
  notification, a popup in Herdr that stays in front until you press OK
  when a run fails, runs too long or needs you, or such a popup also when
  a run finishes. The popup can take you to the run's tab.
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

Needs Herdr 0.9.0 or newer and Rust 1.89 or newer (`cargo`) to build.

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
| `n` | New: a Herdr routine, a classic cron job, or describe it to Claude |
| `p` | Write the prompt |
| `r` | Run now (asks first) |
| `a` | Approve a routine changed outside the list (shows it first) |
| `space` | Switch on or off |
| `c` | Copy the routine; the copy starts switched off |
| `o` | Go to the tab of the last run |
| `l` | Result and output of the last run |
| `h` | Only this routine's runs (`Esc` shows all again) |
| `d` | Delete; asks whether its runs stay in Runs or go too (in Runs: delete the run) |
| `←` `→` `1` `2` `3` | Switch between routines, cron jobs and runs |
| `S` | All routines and cron jobs on or off (asks first) |
| `A` | Start Herdr at login, or not any more (asks first) |
| `C` | Your crontab's other jobs: keep them as cron jobs, move or copy them |
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

**Start at login** (`A` in the list, or a click on the "autostart" chip;
the list asks once on its first start) writes a launch agent
(`~/Library/LaunchAgents/herdr-routines.herdr-server.plist`) on macOS or a
systemd user service on Linux that runs `herdr server` at login. It takes
effect at the next login; turning it off (asks first) removes the entry
and leaves a running server alone.

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
