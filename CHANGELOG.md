# Changelog

All notable changes to this plugin. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Classic cron jobs: a shell command can run from the system's crontab
  instead of Herdr ("Runs in: Cron", or `n` then "Classic cron job"), also
  when Herdr is closed. They have their own tab "Cron jobs", are edited
  like routines, and their runs, output, exit code, retries and alerts
  show in Runs. Each stays a real line in your crontab, edited in place
  by the list; its name and other settings live in the plugin, and edits
  made to the line by hand win. `r` runs one now, in the background.
- The rest of the crontab shows in the Cron jobs tab too; `⏎` keeps a job
  as a cron job managed here (its line stays where it is), moves it to
  Herdr or copies it.
- "Keep" per routine and cron job: how many runs stay in Runs, by default
  the last 30 and none older than 90 days. Replaces the global `history`
  setting.
- "routines on/off" at the top (or `S`) switches all routines and cron jobs
  on or off, after asking; off stays off across Herdr restarts.
- Runs of one routine (`h` or "n runs") show as a filter line in the Runs
  view, with "✕ show all runs"; the tab itself stays "Runs".
- A right click on a routine, cron job or run opens a menu (the list asks
  Herdr to pass right clicks on to it; the entry under the mouse is
  marked) of what can be
  done with it (edit, run, switch, copy, runs, output, delete and more),
  each entry with its key.

- "Tell me by" per routine: a notification (as before), a popup that
  stays in front until OK when a run fails, runs too long or needs you, or
  a popup for every run. The popup can go to the run's tab.
- `c` copies a routine; the copy starts switched off, without runs or
  memory, and waits for approval when the original does.
- In a tall enough window the selected routine's details show under the
  list: schedule, agent, folder, limits, extras, last run and the start of
  the prompt.
- The list asks once whether Herdr should start at login; the header chip
  shows it and a click switches it. Turning it off asks too.

- Approval of routines changed outside the list: a routine runs only as
  saved in the list or approved there with `a`, which shows everything it
  would run, the whole prompt or command included, and approves with `y`
  once read to the end. Routines created by an agent, "Describe it to
  Claude" included, wait for this approval, also when saved again in the
  form. Routines that existed before this version wait for one approval
  too. Herdr notifies once when a waiting routine's time comes.
- Routines whose name, folder, model, arguments or prompt hold hidden
  characters (zero-width, direction overrides and the like) show a problem
  and do not run.
- The list shows a routine's extra arguments under its name, and "run now"
  shows the full options.
- `herdr-routines release-cron` hands every managed cron job back to cron
  as a plain crontab line, for before the plugin or its state folder is
  removed (README, "Uninstall").
- `herdr-routines runs [<id>]` prints the run history as JSON, and
  `routines` now prints every setting of a routine.
- The crontab as it was before the plugin first changed it is kept for
  good (`crontab-backups/crontab-first.txt`), besides the last five
  backups.
- The approval view warns whenever a routine has extra arguments, shows
  how many runs it keeps, and says for a cron job that its line goes into
  the crontab and runs in the home folder, also when Herdr is closed.

### Changed

- The edit form is grouped into parts with headings (routine, schedule,
  long or failed runs, agent extras, prompt), shows up to eight lines of
  the prompt with "…" when it goes on, and says beside the save button
  whether the routine can be saved.
- The lists have no empty row between routines or runs any more; the form
  has one after each part and above the footer.
- "scheduler on/off" at the top is now "routines on/off".
- Deleting a routine asks whether its runs stay (they show in Runs as of a
  deleted routine) or go too; `d` in Runs deletes a single run.
- Extra arguments may no longer set permissions for Claude Code, Codex or
  Gemini; use the permission mode.
- Routine folder names must be lower case letters, digits and dashes.
- The plugin's folders and files are readable only by you (0700 and
  0600), also folders an older version created.
- The build stops when `Cargo.lock` does not match instead of resolving
  newer dependency versions at install time. Building needs Rust 1.89 or
  newer.
- Long lines in the output view wrap instead of being cut off.
- `fire <id>` on a classic cron job starts it in the background as cron
  would run it (as `r` in the list does) instead of opening a tab.
- A routine's numbers have bounds: at most 3 retries at least a minute
  apart, a time limit of at most a year, at most 100,000 runs and 36,500
  days kept. A cron job's folder must be `~`. Routine files must be plain
  text files of at most 1 MB. A routine outside these shows the problem
  and does not run.
- Extra arguments for Claude Code, Codex and Gemini may no longer hold
  options known to start other programs, load code or settings, switch
  safeguards off or widen the sandbox (such as `--mcp-config`,
  `--plugin-dir`, `--bare`, Codex `-c mcp_servers.…`, Gemini
  `--extensions`).
- A routine too long to show in full (more than 65,535 rows) cannot be
  approved in the list.
- "Skipped" and "Nothing to do" notes are kept besides the runs (as many
  again) and no longer count as runs in "Keep".
- The scheduler no longer gives up when Herdr does not answer for ten
  minutes: it tries once a minute until Herdr answers or its server is
  gone, and then hands over to another Herdr session that waits for one.
- `routines.log` is moved to `routines.log.1` at 2 MB. The output view
  shows the end of a very long output.
- A result is kept up to 2,000 characters, a check command's output up to
  8,000; both go into later prompts marked as data.

### Fixed

- A run's prompt goes to the run's own agent by name, and tabs are closed,
  read and shown only when Herdr confirms they are still the run's own, so
  a reused pane or tab id after a Herdr restart cannot hit another tab.
- A prompt handed over late (after a question at start) is the one
  approved when the run started, not a later change.
- Two processes can no longer start the same routine at the same moment.
- Control characters are removed from a prompt before it is typed into the
  agent, and hidden direction and zero-width characters from shown output.
- Run records that name another folder than their own are ignored.
- A shell routine refuses to run from a state folder whose path has a quote
  or backslash, which some shells would read differently.
- A cron job set for one date no longer runs again a year later.
- Crontab lines that are not UTF-8, and Windows line ends, stay as they
  are when the plugin changes the crontab; a cron job whose command is not
  UTF-8 runs; output that is not UTF-8 shows instead of an empty view.
- A cron job's errors are joined to its output without shell text, so a
  crontab `SHELL` such as csh or fish works.
- Text after a `%` in a managed cron job's line reaches the command as its
  input again.
- A time limit stops a cron job or check command together with everything
  it started, not only its shell.
- A huge `keep_days` no longer stops the scheduler.
- Pane ids of the list are kept per Herdr server; "stop at the limit"
  closes a tab only when the run's pane is still in it.
- Calls to `herdr` and `crontab` have a time limit, so one that hangs
  cannot hang the scheduler or the list.

### Security

- A cron job that waits for approval is never written into the crontab,
  whichever key is pressed: its approved line stays until `a` approves the
  change. `space` only switches that line on or off.
- The cron wrapper never approves anything, and uses only the settings
  the owner approved (name, time limit, retries, alerts, runs kept). So do
  alerts and the pruning of the run history; a routine file that no longer
  reads deletes no history.
- Saving in the form or the editor keeps a routine approved only when it
  was approved when opened and unchanged since; "run now" reads the
  routine again after the `y`.
- Two changes to the crontab at the same time can no longer lose one: they
  wait for each other, and a crontab that changed while it was read is
  read again instead of being overwritten.
- A scheduler or cron run is recognised by its pid and start time, not by
  the process name.
- The `fire`, `status` and other commands print text from routine files
  without control characters.
- The unit tests cannot reach a real Herdr any more.

## [0.1.0] - 2026-11-10

### Added

- Scheduler that starts with Herdr and runs routines on five-field cron
  schedules in local time, each run as an interactive agent in its own tab.
- Any agent kind Herdr can start (Claude, Codex, Gemini, OpenCode and more).
- "Needs you" detection with a notification, also for questions before the
  agent is ready; the prompt follows once answered.
- Missed times caught up once or skipped, per routine; no overlapping runs;
  time limit per routine; finished tabs closed after an hour.
- List of routines and runs with a form to create and edit routines, run
  now, on and off, delete, and saved output; the same look and the same
  keys and mouse use as the other Herdr plugins of this author, with all
  keys under `?`.
- Permission mode, model and effort per routine, picked from a list, for
  Claude Code, Codex and Gemini; new routines start in Auto.
- Prompt editor inside the list (arrows, mouse, paste, undo), and `Ctrl+O`
  for your own editor.
- Shell commands as routines, like cron jobs without AI: exit code 0
  finishes the run, anything else marks it failed and notifies.
- On and off at a glance: a green dot for on, a red one for off.
- A model choice for every agent that takes one, from the agent's own list
  where it prints one; long lists filter by typing.
- Create a routine by describing it to Claude.
- Retries for failed runs, stopping a run at its time limit, single runs on
  a date, runs when Herdr starts, and starting Herdr at login.
- Check command before an agent run; the agent's summary saved with each
  run, and optionally given to the next runs.
- Whole output of shell commands in a file; agent tabs saved every minute;
  `f` opens a run's output file; the runs of one routine with "n runs".
- `routines` (JSON) and `fire <id>` commands for scripts and other agents.
