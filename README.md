# herdr-routines

A [Herdr](https://herdr.dev) plugin that runs coding agents and shell
commands on a schedule. A routine is a prompt, an agent (Claude, Codex,
Gemini, OpenCode or any other agent Herdr knows), a folder and a time. When
the time comes, the plugin opens a tab in the background, starts the agent
there and hands it the prompt. You can watch any run live, answer when the
agent asks something and keep talking to it afterwards. A routine can also
be a plain shell command or script, without any AI, and such a command can
also be a cron job that your system's cron runs.

Routines run as long as the Herdr server runs, also when no Herdr window is
open. Herdr can also start in the background at login (the list asks once,
and the "autostart" chip at the top or `A` switches it), so routines also
run after a restart without opening Herdr. Cron jobs run whenever
the computer is on. The "routines" chip at the top (or `S`) switches all of
them on or off at once.

```
 ◷ Scheduled routines    Routines (3)   Cron jobs (2)   Runs (41)   ● autostart on   ● routines on

      Routine                                 Schedule                  Last run
 ●    Inbox clean up                          Weekdays, hourly at :15   ✓ Finished  12 runs ›
      claude · Sonnet · Auto · ~/mail         next in 12 min            48 min ago · took 3 min
 ●    Nightly backup                          Daily at 02:00            ✗ Failed  30 runs ›
      shell command · ./backup.sh --all · …   next Thu 02:00            10 h ago · took 4 min
 ○    Release notes                           Monthly on the 1st        –
      claude · Opus · Plan only · ~/code/app  off                       no run yet

  Inbox clean up ────────────────────────────────────────────────────────────────────────────────
  Schedule        Weekdays, hourly at :15 · next in 12 min
  Last run        ✓ Finished · 48 min ago · 3 newsletters archived, 2 drafts written
  Runs with       claude · Sonnet · Auto
  Folder          ~/mail
  Time limit      30 minutes, then the run is stopped
  If it fails     tries once more after 5 minutes
  Run history     The last 30 runs, none older than 90 days
  Prompt          Sort the new mail in the inbox: archive newsletters,
                  draft replies to questions …

 ⏎ edit  n new  r run now  space on/off  l output  h its runs  p prompt  ? all keys  q close
```

The sign in front of a routine says how it stands, by shape as well as by
colour: `●` on, `○` off, `!` a problem, `◐` waits for your approval. The
selected routine's details show under the list whenever rows are free. In a
narrow pane the list keeps the last run: it moves into the routine's second
line, and the header drops its title before anything else ("routines off"
always stays).

## What it does

- **List of routines** with their schedule in words, the next time and the
  last run. What needs you comes first: a line at the top says how many
  runs need you, went wrong or are running and what runs next, and the
  list is sorted the same way (`s` sorts by name instead). Switch a
  routine on or off with one key, run it now, edit it,
  write its prompt, duplicate it (the copy starts switched off), or create
  and delete routines. The form starts with what a routine needs: its name,
  what it runs with, the prompt (its first lines show), when and where.
  Everything else (permission mode, model, time limit, retries, alerts, run
  history and more) is under "More settings", with values that suit most
  routines. Works with the keyboard and the mouse, like the other Herdr
  plugins of this author; `?` shows all keys.
- **Permission mode, model and effort** per routine, picked from a list:
  for Claude Code the permission modes (Auto, Accept edits, Plan only and
  more), the models (Fable, Opus, Sonnet, Haiku or any typed name) and the
  effort; for Codex and Gemini their modes. Every agent that takes a model
  on its command line gets a model choice: the agent's own list where it
  can print one (Codex, Cursor, OpenCode, Kilo, Kiro, Devin, Pi, Grok,
  Antigravity; refreshed daily in the background), its documented aliases,
  models other routines use, or any typed name. New routines start in
  Auto, since nobody watches a scheduled run.
- **Shell commands:** choose "Shell command" under "Runs with" and write the
  command or script instead of a prompt. It runs in its own tab in the
  folder, with your shell and its environment. Exit code 0 counts as
  finished; any other code marks the run as failed and Herdr shows a
  notification.
- **Cron jobs** have their own tab. `n`, then "Cron job",
  creates one: a one-line shell command that the system's cron runs from
  your crontab, also when Herdr is closed. It stays an ordinary line in
  your crontab: the list edits that line (schedule, command, on or off),
  and keeps what cron has no place for (its name, retries, alerts, its run
  history) in the plugin. Edits you make to the line by hand win
  and show in the list. Its runs (output, result, exit code, failures,
  retries, alerts) show in Runs like any other, and `r` runs it now, in
  the background, as cron would. Every change to the crontab keeps a
  backup of it in the state folder first (`crontab-backups/`: the crontab
  as it was before the plugin first changed it stays there for good, of
  the later ones the last five), changes only the job's own lines and
  leaves every other line as it is, byte for byte. On macOS, a cron job
  that works in Documents, Desktop or Downloads needs Full Disk Access for
  `/usr/sbin/cron` (System Settings, Privacy & Security, Full Disk Access).
  **Before you remove the plugin or its state folder, hand these jobs
  back to cron** (see [Uninstall](#uninstall)): cron starts them through
  that folder and cannot start them without it.
- **Your other cron jobs** show in the Cron jobs tab too (◇), marked "only
  in your crontab". `⏎` on one (or `C` for all) takes them over: keep it as
  a cron job (its line stays where it is, as you wrote it, and the list
  follows its runs), move it to Herdr (it runs in a tab instead), or copy
  it to Herdr switched off. Keeping and moving change your crontab, so they
  ask first. Moving comments the old crontab line out, so nothing runs
  twice. Variables set in the crontab (such as `PATH`) come along.
- **How cron jobs are followed:** the plugin puts a comment above a managed
  job's line and sets `SHELL` around it to a small wrapper (a link to the
  plugin in its state folder, named after the job). Cron starts the
  command through that wrapper, which records the run, runs the command
  with the shell your crontab had, hands its output on to cron and returns
  its exit code. While routines are switched off in the list, the wrapper
  skips the job's times. The name, time limit, retries and alerts it uses
  are the ones you approved; at the time limit it stops the command and
  everything the command started.
- **Run history:** each routine and cron job keeps its last 30 runs, none
  older than 90 days, by default. "Run history" in the form (under "More
  settings") changes that (such as 10 runs, 100 runs and a year, or all
  runs).
- **Create with Claude:** `n`, then "Describe it to Claude", opens a tab
  where Claude asks what the routine should do, writes it and checks that
  it loads. You then approve it in the list with `a`.
- **Chat about a run:** `t` on a run that ended (in Runs, on its page, or
  on a routine for its last run) opens a tab with Claude Code that reads
  the run's record and output and the routine's files, tells you what the
  run did and why it ended as it did, and answers your questions. When you
  ask for it, it changes the routine; such a change waits for your `a` in
  the list like any change from outside. Reading the run needs no answer
  from you; before it changes a file or runs a command it asks (Claude
  Code's `manual` mode; rules in your own Claude Code settings still
  apply). It uses the routine's account when it names one.
- **Your approval:** a routine runs only as you saved it in the list or
  approved it there. When anything else creates or changes its files (an
  agent, an editor outside the list), it waits, marked `◐` and "New, not
  approved yet" or "Changed outside the list", until you press `a`, read
  what it would run (every setting, every
  option and the whole prompt or command) to the end and approve it with
  `y`. For a routine you approved before, what changed since is marked:
  under the list, the changed settings are named and their rows are
  yellow with `▌` in front; in the view, new or changed lines have `▌`
  and the new words in yellow, the line as it was stands above in grey
  with `−` and what is gone crossed out, and `n` jumps to the next
  change. When a waiting routine's time comes, Herdr tells you once. So an
  agent that writes routine files the normal way cannot make the
  scheduler run something you have not seen, such as a stronger
  permission mode or a shell command. A cron job that waits keeps the
  crontab line you approved (`space` still switches that line on and off);
  the changed command reaches the crontab only when you approve it. For a
  cron job the view says that it is one: where its line goes, that cron
  starts it also when Herdr is closed, and that it runs in your home
  folder. Extra arguments get a warning line of their own. A routine too
  long to show in full (more than 65,535 rows) cannot be approved in the
  list; shorten it first. Saving in the form or the editor keeps a routine
  approved only when it was approved when you opened it and nothing else
  changed its files meanwhile. The approval does not cover the agent's
  own settings in the routine's folder (such as `.claude/settings.json`),
  which the agent reads at start like in any other session.
- **Prompt editor** in the list: type, paste, move with the arrows or the
  mouse, mark text with Shift and the arrows or by dragging, copy, cut and
  paste with `Ctrl+C`, `Ctrl+X`, `Ctrl+V`, undo with `Ctrl+Z`. `Esc`
  closes it and keeps what you wrote (a routine's prompt is saved at once;
  in the form it goes back to the form). `Ctrl+D` leaves without keeping
  what you wrote since it opened, after asking. `Ctrl+O` opens the prompt in
  your own editor instead.
- **Runs** of all routines, newest first: status, how it was started (on
  time, catch-up, retry or by hand), how long it took, what it came to and,
  when it failed, why. "n runs ›" next to a routine (or `h`) shows only its
  runs, and `Esc` comes back to the routine. `p` shows only the runs that
  went wrong or need you, and `/` searches name, reason and result. `⏎`
  opens a run's page: status and reason, when and how it was started, how
  long it took, its result and the end of its output, with the keys that
  apply (go to its tab, the whole output, stop, run again, open the file,
  delete). `l` shows the result and output directly, also after the tab is
  closed (of a very long output its end), with the reason of a failure kept
  above the output, and `f` opens the output file.
- **Output kept:** a shell command's whole output goes into a file; an
  agent's tab is saved every minute while it is open, so closing the tab by
  hand loses nothing.
- **Results:** each agent run is asked to end with a short summary, which
  the list shows next to the run. With "Earlier results" on, the next run
  starts with the summaries of the last ten runs (kept in one small file per
  routine), so a daily routine knows what it did before. A summary is kept
  up to 2,000 characters.
- **Check first:** an optional shell command that runs before the agent,
  such as "are there new files?". When it ends without an error the agent
  starts and gets its output (the first 8,000 characters); otherwise the
  run is noted as "Nothing to do" and costs nothing.
- **Text from runs is handed on as data:** a check command's output and
  earlier results can hold whatever a run read (a mail, a web page, a
  commit message). They go into the next prompt cut to length, without
  hidden characters, inside a marked block, with the note not to follow
  instructions in them. That lowers the risk that such text steers the
  agent; it cannot rule it out, so give a routine that reads untrusted
  text a permission mode that fits.
- **Retries:** a failed run (a command ending with an error, an agent that
  does not start) can be tried again up to three times a few minutes later.
- **Schedules without cron syntax:** every so many minutes or hours, daily,
  on chosen weekdays, weekly, every two weeks, monthly on a day, on the
  last day or the last working day of the month, on the first Monday (or
  any numbered weekday), once on a date (then the routine switches itself
  off), or each time the Herdr server starts. The form shows the schedule
  in words and its next three times. Five-field cron works too, and in the
  file also `L` and `LW` as day of the month, `mon#1` and `friL` as
  weekday, `@every 2 weeks from 2026-11-09 09:00` and cron's `@daily` and
  the like. A cron job takes only what the system's cron knows; one set
  for a single date runs on that date only (its crontab line has no year,
  so cron would start it again a year later, and the plugin's wrapper then
  skips it).
- **Needs you:** when an agent asks a question or waits for an approval, the
  run is marked and Herdr shows a notification. Answer it in the tab; the
  run carries on. This also covers questions before the agent is ready, such
  as Claude asking whether to trust a new folder: the prompt goes in once
  you answered.
- **Notification or popup** per routine ("Alerts"): Herdr's usual
  notification, a popup in Herdr that stays in front until you press OK
  when a run went wrong or needs you, or such a popup also when a run
  finishes. The popup can take you to the run's tab.
- **Every run that went wrong is told:** a command that ends with an error,
  a run past its time limit, an agent that could not be started (not
  signed in, a check command that broke), a tab closed before the run was
  done, and an agent that stopped without a result (see below). When Herdr
  is closed, as it can be for a cron job, the system's own notification is
  used instead.
- **No result:** every agent run is asked to end with a summary. A run
  whose agent stops without one (it reached a usage limit, or gave up)
  shows as "No result" instead of "Finished", and you are told. It is not
  tried again by itself, since the work may be done.
- **A row of failed runs is told once:** when a routine goes wrong again
  and again (an hourly one over night), you are told at the first run, at
  every tenth, and when it works again, not at each one. A routine can
  switch itself off after a number of runs in a row that went wrong.
- **After the run:** an optional command of your own that runs when a run
  went wrong (or after every run), such as a script that posts to your
  chat or phone. It gets `HERDR_ROUTINES_STATUS` (`finished`, `failed`,
  `timed-out`, `closed`, `no-result`), `_NAME`, `_ID`, `_RUN`, `_MESSAGE`,
  `_RESULT`, `_OUTPUT` (the output file) and `_STARTED`, runs in the
  routine's folder and may take five minutes. It is part of what you
  approve.
- **Missed times:** when Herdr or the computer was off or asleep at the
  time, each routine either runs once afterwards or skips to its next time,
  as you set it per routine. Running once can have a limit, such as "if
  less than 2 hours late", so a morning routine does not start in the
  evening. A skipped time shows in Runs with the reason, and one
  notification says what was caught up and what was skipped.
- **A few at a time:** Herdr starts at most three runs side by side
  (`max_parallel` in the settings); further ones wait for a free place and
  start when a run ends. A run that waits for your answer takes no place.
- **Awake while a run is going:** the computer does not go to sleep in the
  middle of a run (`caffeinate` on macOS, `systemd-inhibit` on Linux). The
  "awake" switch at the top chooses between during runs, always while
  Herdr runs (so no time is missed for sleep) and off. Only the computer
  is kept awake: the screen still goes dark as its settings say, and runs
  go on behind it. A closed laptop lid still sleeps; time slept does not
  count towards a run's time limit.
- **Accounts:** with the
  [Multiaccount](https://github.com/nolvane/herdr-multiaccount) plugin
  installed, a Claude Code routine can name the account it runs with;
  without a name Multiaccount decides as for any other session.
- **Stop a run** from the list: its tab is closed (what it wrote is kept),
  or a cron job's command is stopped.
- **No overlap:** a routine that is still running is not started again; the
  skipped time shows in the runs. Such notes ("Skipped", "Nothing to do")
  are kept besides the runs and never push a real run out of the history.
- **Time limit** per routine: a run still going after it is marked as timed
  out and you get a notification. Its tab stays open, or, if you choose
  "Stop the run", it is closed, which ends the agent or command. A cron
  job and a check command are stopped at their limit, together with
  everything they started.
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

## Uninstall

**First hand your managed cron jobs back to cron.** A cron job the plugin
manages is started by cron through a small wrapper in the plugin's state
folder (`SHELL=…/bin/cron/<id>` above its line in the crontab). Without
that folder cron can no longer start the job at all, and it does not tell
you. So before you remove the plugin or delete its state folder, run:

```sh
herdr-routines release-cron
```

It takes the plugin's marker and `SHELL` lines out of your crontab and
leaves each job's own line as an ordinary cron job, where it was (a job
that was switched off stays, commented out). The crontab as it was before
is in `crontab-backups/` in the state folder. `crontab -l` shows the
result. Routines that Herdr runs need nothing: they stop with the plugin.

If the plugin is already gone, the copy that the wrappers use may still be
there: `~/.local/state/herdr/plugins/herdr-routines/bin/herdr-routines
release-cron` does the same. Otherwise edit the crontab by hand
(`crontab -e`): delete the lines `# herdr-routines: …` and the `SHELL=`
line above and below each job.

Then remove the plugin with Herdr's plugin command, switch "start at
login" off before if you turned it on (`A` in the list), and delete the
config and state folders if you do not want to keep the routines and
their runs.

## Keys

`?` in the list shows every key. The main ones:

| Key | Does |
| --- | --- |
| `↑` `↓` | Move (`PgUp` `PgDn` a page, `g` `G` first and last) |
| `⏎` | Edit the routine (in Runs: open the run's page) |
| `n` | New: a Herdr routine, a cron job, or describe it to Claude |
| `p` | Write the prompt or command |
| `r` | Run now (asks first) |
| `a` | Approve a routine that is new or was changed outside the list (shows it first, with what changed marked; `n` there goes to the next change) |
| `space` | Switch on or off |
| `c` | Duplicate the routine; the copy starts switched off |
| `o` | Go to the tab of the last run (in Runs: of the run, or its output once the tab is closed) |
| `l` | Output of the last run |
| `x` | Stop the run that is going (asks first) |
| `t` | Chat about the run that ended (on a routine: its last run): a tab with Claude that has the run to look at |
| `h` | The routine's runs (`Esc` comes back to it); in Runs: only this routine's runs, again for all |
| `s` | Sort by what needs you first, or by name; the list remembers it |
| `p` `/` | In Runs: only runs that went wrong or need you; search the runs |
| `f` | In Runs: open the output file |
| `d` | Delete; asks whether its runs stay in Runs or go too (in Runs: delete the run) |
| `←` `→` `1` `2` `3` | Switch between routines, cron jobs and runs |
| `S` | All routines and cron jobs on or off (asks first) |
| `A` | Autostart: Herdr starts at login, or not any more (asks first) |
| `W` | Awake: keep the computer awake during runs, always, or not |
| `C` | Take over your crontab's other jobs: keep them as cron jobs, move or copy them |
| `i` | Nerd Font icons or plain symbols |
| `q` | Close the list (asks first); routines keep running |

The footer shows the keys that work where you are: the list's keys in the
list, a dialog's keys while a dialog is open. A key that does not apply to
the row says why (such as `h` on a routine without runs, or `r` on a job
that is only in your crontab).

The mouse follows one rule everywhere: a click marks a row or a field, and
a click on what is marked opens it. What does only one thing acts at once:
a tab, a chip at the top, a key in the footer, a button, `‹` and `›` of a
choice, an entry of a list to choose from, and "n runs ›" beside a routine.
The wheel scrolls the view without moving the mark, a click beside a dialog
closes it, and a right click on a routine, cron job or run opens a menu of
what can be done with it, each entry with its key.

A question is answered with `y`. `⏎` answers yes too where the same key
undoes it (switching all routines, autostart, taking over cron jobs);
deleting, discarding and running by hand need `y` itself. Any other key
means no.

In the form, `↑` `↓` move between fields, `⏎` opens the list of a choice,
the prompt editor or "More settings", and `←` `→` step through a choice.
In a text field the arrows, `Home` and `End` move the cursor, `Backspace`
and `Del` delete, and `Ctrl+U` clears the field. `Ctrl+S` saves from
anywhere, and the save button stays in sight. `Esc` goes back and asks
first when something changed.

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
# Optional: try a failed run again, this many times (at most 3), minutes
# apart (at least 1)
# retries = 2
# retry_delay_minutes = 10
# Optional, agents only: a shell command run first; the agent starts only
# when it ends without an error, and gets its output
# check_command = "git fetch -q && git log --oneline HEAD..@{u}"
# Optional, agents only: start each run with the last ten results
# remember = true
# Optional: runs kept in the history (default 30) and none older than this
# many days (default 90); 0 means no limit
# keep_runs = 30
# keep_days = 90
# Optional, with missed = "run-once": catch a missed time up only when it
# is at most this many hours late (default: however late, within a week)
# late_hours = 2
# Optional: a one-line command after a run that went wrong, or after every
# run with after_always = true
# after_command = "~/bin/tell-me.sh"
# after_always = true
# Optional: switch the routine off after this many runs in a row that
# went wrong
# off_after_failures = 5
# Optional, for claude with the Multiaccount plugin: the account's id
# account = "work"
```

Besides five-field cron, `schedule` takes `@once 2026-12-24 08:15`,
`@start`, `@every 2 weeks from 2026-11-09 09:00` (or `days`), cron's
`@hourly`, `@daily`, `@weekly`, `@monthly` and `@yearly`, and in the cron
fields `L` (the last day of the month), `LW` (its last working day),
`mon#1` (the first Monday; up to `#5`) and `friL` (the last Friday).

`prompt.md` holds the prompt, as you would type it to the agent. A shell
routine has `agent = "shell"` and its command in `command.sh` instead; it
runs in your `$SHELL`, line by line like a script. With `runs_in = "cron"`
it is a cron job: `command.sh` is then one line, which the list
writes into your crontab once the routine is approved. A cron job runs in
your home folder, so its `folder` is `~` (use `cd` in the command for
another one).

You, the list or another agent can create and change these files; the
scheduler reads them every few seconds. A routine with a mistake shows up in
the list with what is wrong. A change made outside the list takes effect
once you approve it with `a` in the list (see "Your approval" above). The
folder name is the routine's id: lower case letters, digits and dashes.
The files are plain text files of at most 1 MB each. The numbers have
bounds: a time limit of at most a year, at most 3 retries, at most
100,000 runs and 36,500 days kept. A routine outside them shows the
problem and does not run. While a change waits for approval, the history
is still pruned by the approved `keep_runs` and `keep_days`.

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
`permission_mode` is for, so the list always shows the real mode. Nor may
they hold options known to start other programs, load code or settings,
switch safeguards off or widen where the agent may write (such as
`--mcp-config`, `--plugin-dir`, `--settings`, `--bare`, Codex
`-c mcp_servers.…` or `--add-dir`, Gemini `--extensions`). That list
cannot be complete, since agents gain options: the list shows `args`
under the routine's name (the form calls them "Extra options"), and the
approval view warns whenever a routine has any. Approve extra options
only when you know each of them.

Runs are kept in the plugin's state directory
(`~/.local/state/herdr/plugins/herdr-routines/`), by default the last 30
per routine and none older than 90 days ("Run history" in the form,
`keep_runs` and `keep_days` in the file): under
`runs/<routine id>/`, each run has a record `<run id>.json` (status,
times, result) and next to it its output (`.log` from the tab, `.out` from
a shell command), the prompt it got (`.prompt`) and, for shell commands,
the script. The run id is its start time, such as `20261110-193702-582`.
The routine id is the folder name, made from the first name and never
changed, so renaming a routine keeps its runs. `memory/<routine id>.md`
holds the earlier results of a routine that remembers. The plugin's
folders and files are readable only by you, since prompts, commands and
saved output can hold secrets.

**Autostart** (`A` in the list, or a click on the "autostart" chip;
the list asks once on its first start) writes a launch agent
(`~/Library/LaunchAgents/herdr-routines.herdr-server.plist`) on macOS or a
systemd user service on Linux that runs `herdr server` at login. It takes
effect at the next login; turning it off (asks first) removes the entry
and leaves a running server alone.

## Settings

Optional, in `config.toml` in the plugin's config directory. See
[config.toml.example](config.toml.example): the workspace name, when
finished tabs close, notifications on finish, how many runs start side by
side, icons and where the list opens. How many runs are kept is set per routine (see "Run history").

## Command line

```sh
herdr-routines routines      # all routines, every setting and their last run, as JSON
herdr-routines runs [<id>]   # the run history (of one routine), newest first, as JSON
herdr-routines fire <id>     # run an approved routine now
herdr-routines status        # whether the scheduler runs
herdr-routines release-cron  # hand managed cron jobs back to cron (see Uninstall)
```

`fire` on a cron job does what `r` in the list does: it starts the
job in the background, as cron would run it, and returns at once;
`runs <id>` shows how it went. For a routine Herdr runs, it waits until
the agent is up.

The plugin actions "herdr-routines: restart the scheduler", "stop" and "status" do
the same from Herdr. The scheduler log is `routines.log` in the state
directory; at 2 MB it is moved to `routines.log.1` and started anew.

The scheduler stays with its Herdr server: when the server does not answer
for about ten minutes it keeps trying once a minute instead of giving up, and it
ends when the server is gone. When several Herdr sessions run, one
scheduler serves them all; when its session ends, it hands over to another
session that is still running.

## Licence

MIT, see [LICENSE](LICENSE).
