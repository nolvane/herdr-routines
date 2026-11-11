# Decisions

Notable technical decisions: what was decided, why, and when.

## 2026-11-10: The scheduler runs inside Herdr, not as a system service

The scheduler is a background process that Herdr's plugin `[[startup]]` hook
starts. It runs as long as the Herdr server runs and exits when the server
changes or goes away, the same way the Tab Titles plugin does it. It is not a
launchd or systemd service.

There is one scheduler for all Herdr sessions on the computer, since they
share the routines: two schedulers would start every routine twice. The pid
file records the server it serves. A `start` from another session leaves a
running scheduler alone while its server runs; a `start` from the same
socket (that server restarted) takes over. Each run records the server it
was started on (socket and the socket file's inode), because pane ids only
mean something there and start over after a restart; runs of a server that
is gone are marked closed instead of being looked up on another server.

Why: the point of the plugin is that a run happens inside Herdr, in a tab
you can watch and step into, with any agent Herdr knows. Without a Herdr
server there is nowhere to run, so a system service would only add a second
thing to install and keep alive. The Herdr server keeps running when no
client is attached, so closing the terminal window does not stop routines.

## 2026-11-10: Each run is an interactive agent in its own tab

A run opens a new tab (with `--no-focus`) in the workspace named by the
`workspace` setting, default "Routines", which is created when missing. Herdr
starts the agent there (`herdr agent start`) and the prompt goes in through
`herdr agent prompt`, as if typed. There is no headless mode.

Why: the owner's choice. You can watch a run live, answer when the agent
asks something, and keep talking to it after it finished. A headless run
would fail on the first question instead.

## 2026-11-10: Done means one more completed turn

The scheduler looks at each active run every 10 seconds with
`herdr agent get`. A scheduler that hands over to a new one (restart) lets
its launches in progress finish first. `working` means running, `blocked` means it asks something
(a notification goes out once per wait), `idle` or `done` means finished, but
only once the agent's `completion_seq` is higher than just before the prompt
went in, or it was seen working.

Why: a quick agent answers within a second, between two looks, so "seen
working" alone missed it in testing. `completion_seq` counts finished turns
and does not depend on timing. An agent that is still idle three minutes
after the prompt counts as "needs you", since the prompt most likely did not
land.

## 2026-11-10: A question at start does not fail the run

When the agent asks something before it is ready (such as Claude asking
whether to trust a new folder), `agent start` returns `agent_not_ready`. The
run then waits as "needs you" and the scheduler hands over the prompt as soon
as the agent is idle.

Why: these questions are normal the first time an agent runs in a folder.
Failing the run would mean the routine never gets past them.

## 2026-11-10: Missed times are caught up once or skipped, per routine

A due time found at most 3 minutes late runs as scheduled. Anything older
means Herdr or the computer was off or asleep. Then the routine's `missed`
setting decides: `run-once` starts one catch-up run for the latest missed
time (never one per missed time), `skip` waits for the next time. The
scheduler looks back at most 7 days. A new routine, or one that was off,
starts counting from now and never catches up on the time before.

Why: the owner wanted the choice per routine. One catch-up run is what the
Claude desktop app does and avoids a burst of runs after a long break.

## 2026-11-10: A routine never runs twice at the same time

When a routine is due while its previous run is still active, or timed out
but its agent still works, the new run is not started and a "Skipped" run is
recorded instead.

Why: two agents working on the same task in the same folder get in each
other's way, and the history shows why a time was left out.

## 2026-11-10: Finished tabs close after an hour

The tab of a finished run is closed after `close_finished_after_minutes`
(default 60, 0 keeps them). It stays open while its agent works or asks
something, and while the routines workspace is on screen. When the agent
completed more turns since the run finished (you picked up the
conversation), the wait starts again.
Timed out, failed and "needs you" runs are never closed by the plugin. The
output is saved to the run's log before closing.

Why: an hourly routine would otherwise leave dozens of idle agents behind.

## 2026-11-10: Routines are plain files

Each routine is a folder with `routine.toml` (name, schedule, agent, folder,
settings) and `prompt.md`. Run records are JSON files with the output in a
`.log` next to them. The list, a person with an editor and another agent all
edit the same files; the scheduler reads them every tick, so there is no
reload step. A folder that does not load shows up in the list with its
problem instead of disappearing.

Why: a coding agent can create or change a routine without the list, and a
typo stays visible.

## 2026-11-10: Own cron parser

Schedules are five-field cron expressions in local time, parsed by
`src/cron.rs` (about 200 lines with tests) instead of a crate.

Why: the crates looked at either use a six or seven field format, bring their
own time zone handling, or are larger than the need. Local time with daylight
saving comes from `chrono`; the next time is found by stepping through UTC
minutes and skipping hours and days that cannot match. As in classic cron, a
time inside the hour the clock skips in spring does not run that day, and one
inside the hour repeated in autumn can run twice.

## 2026-11-10: Dependencies and licences

`ratatui` (MIT), `chrono`, `serde`, `serde_json`, `toml` and
`unicode-width` (each MIT OR Apache-2.0; `unicode-width` measures text for
the prompt editor and comes with ratatui already). All allow use and redistribution in this MIT-licensed plugin
with the licence notice kept. No outside service is called: the plugin talks
only to the local Herdr server, and the agents use their own logins.

## 2026-11-10: Permission mode, model and effort are fields, not arguments

A routine has `permission_mode`, `model` and `effort`, picked from lists in
the form, and `agent.rs` turns them into the agent's own flags (Claude
Code `--permission-mode`, `--model`, `--effort`; Codex `--full-auto`,
`--sandbox`, `--dangerously-bypass-approvals-and-sandbox`, `--model`;
Gemini `--approval-mode`, `--model`). Only agents whose flags were checked
get these fields; all others keep `args` for their own flags. Claude's
modes and model aliases are the ones its `--help` lists; other models are
typed by name. New Claude and Codex routines start in Auto.

Why: the owner wants to set auto mode and the model per routine without
knowing command line flags. A scheduled run has nobody watching, so the
default that asks before every change would leave most runs waiting.

## 2026-11-10: The prompt is written in the list

The prompt opens in an editor inside the list (`editor.rs`): word wrap,
arrows and Home/End on the lines as shown, mouse placement, paste, undo.
`Ctrl+O` still hands the text to `$VISUAL`/`$EDITOR` for those who prefer
theirs.

Why: the owner found the jump into nano or vim unintuitive. A small text
area keeps the look and keys of the list and needs no editor set up.

Few keys, like a notes app: `Esc` closes the editor and keeps the text,
`Ctrl+Z` and `Ctrl+Y` undo and redo, and the footer shows only these
(written `Ctrl+…`, not `^…`). There is no "discard?" question; undo takes
a change back. Why: the owner found save-or-cancel keys too complicated.

## 2026-11-10: Same look and controls as the other plugins

The list follows the Herdr Git Panel and Multiaccount plugins: title with
tabs in the header, two-line rows with a heading bar, a footer with
clickable keys, rounded dialogs, a keys overlay under `?` with the version
linked in its border, `q` asking in the footer, the mouse for tabs, rows,
lists and buttons, and the same colours.

Why: the owner uses these plugins side by side and wants them to work the
same way.

## 2026-11-10: Shell commands as routines

A routine can run a shell command or script instead of an agent
(`agent = "shell"`, the command in `command.sh` instead of `prompt.md`).
The run opens a tab like any other; the command goes into a script next to
the run record, and the tab's shell runs a small `/bin/sh` wrapper that
runs it in the person's `$SHELL` and writes its exit code to a file. The
scheduler reads that file: exit code 0 is Finished, anything else Failed
with a notification. Time limit, missed times, no overlap and closing
finished tabs work as for agents.

Why: the owner wants any cron job, not only AI ones. Running in a tab of
the routines workspace keeps one place to watch and step in, and the shell
of the tab brings the person's own environment (PATH and so on). The
wrapper works whatever shell the tab has, and the exit code file does not
depend on reading the screen.

## 2026-11-10: On is green, off is red

The mark in front of a routine is a green dot when it is on and a red dot
when it is off; a routine with a problem gets a yellow warning sign.

Why: the owner's choice, so the state reads at a glance.

## 2026-11-10: Name

The plugin's name, id and repository are all `herdr-routines` (owner's call,
2026-11-10): one name everywhere, matching the binary. "Routines" is a plain
English word for what the plugin does, and Herdr is the host it extends. No
agent or vendor name is in it, so no trademark is used as a product name.
Inside the list the tab "Routines" and the default workspace for runs keep
the plain word, since they name the things listed, not the plugin.

## 2026-11-10: Create a routine by describing it to Claude

`n` asks whether to fill in the form or describe the routine to Claude.
The second opens a tab in the list's workspace, in the plugin's config
folder, with Claude in Auto mode. The brief goes in as its first message
(`herdr agent prompt`, since a multi-line start argument is refused), once
Claude takes it, also after a "trust this folder?" question. It holds the
file format and asks Claude to ask what is needed in one message, show a
summary before writing, leave other routines alone, and check the result
with `herdr-routines routines`; the tab gets the list's config and state
folders as environment variables so that check reads the same routines.
The list and scheduler pick up the files as they pick up any edit.

Why: the owner wanted routines made by an agent. Claude writes the same
files a person would, so nothing new has to be kept in sync. Auto mode
lets it write the files and run the check without a question each time
(`acceptEdits` still asked before the check). Claude only, since it is the
agent the owner uses and the brief relies on its flags.

## 2026-11-10: Marking text in the prompt editor

Shift with the arrows, a mouse drag, a double click (a word) and `Ctrl+A`
mark text; typing or `Backspace` replaces it, `Ctrl+C`, `Ctrl+X` and
`Ctrl+V` copy, cut and paste through the system clipboard (`pbcopy`, or
`wl-copy`/`xclip` on Linux, with a copy inside the list as fallback). The
scheduler status chip is green when it runs, red when it does not.

Why: the owner wants to edit prompts faster. The list captures the mouse,
so the terminal's own selection does not reach the text; the editor has to
mark itself.

## 2026-11-10: No cost limit per run

A USD limit next to the time limit was asked for and then dropped (owner's
call). Claude Code only enforces `--max-budget-usd` in headless `--print`
runs, and runs here are interactive tabs on purpose; an estimate from the
transcript would be unreliable, and on a subscription a run spends quota,
not dollars. The time limit and a cheaper model are the levers.

## 2026-11-10: A model choice for every agent

Every agent that takes a model on its command line gets the Model field
(`--model`; all known agents but amp, mastracode, maki and muse, which take
none there). The list offers, in this order: the agent's documented aliases
(Claude, Gemini, Kilo, Qoder), the models the agent prints itself, models
other routines of that agent use, and "Other…" for any name. Agents with a
documented list command (`agent::list_command`: Codex, Cursor, OpenCode,
Kilo, Kiro, Devin, Pi, Grok, Antigravity) are asked in the background in
the person's login shell, at most daily (hourly while they print nothing,
for example before a login), with a 30-second limit; the names go to
`models/<agent>.txt` in the state folder, so the form never waits. The
parser takes JSON objects (`slug`, `id`, `model`, `name`) or the first word
of each line and skips headers. Lists longer than 12 entries are narrowed
by typing.

Why: the plugin is meant for any agent, not only Claude. Model names change
too often to ship as a list; the agents know their own. Sources (checked
2026-11-10): each CLI's official reference, see the research notes in the
pull request. Codex's `--full-auto` is gone from its current reference, so
Auto is now `--sandbox workspace-write --ask-for-approval never`.

## 2026-11-10: Cron jobs in full: retries, stop at limit, once, at start, start at login

Asked for by the owner to make the plugin a full cron replacement:

- **Start at login** (`autostart.rs`, `A` in the list): a launch agent
  (macOS) or systemd user service (Linux) that runs `herdr server` at
  login, with the PATH and shell of the moment it was turned on (launchd's
  PATH lacks Homebrew). The plugin's startup hook then starts the scheduler.
  It is written but not loaded, so the running server is never touched;
  turning it off only removes the entry. Why the server and not the plugin:
  runs open tabs, so they need the server anyway, and this keeps one
  process tree.
- **Output in files**: a shell command's output goes through `tee` into
  `<run>.out`, and the exit code is moved into place only after the output
  is complete. An agent's tab is saved to `<run>.log` every minute while it
  is open (also after the run, while someone keeps talking in it), so a tab
  closed by hand loses at most a minute.
- **Retries** (`retries`, `retry_delay_minutes`): only `Failed` runs (exit
  code not 0, agent did not start, check command broke) are retried; the
  newest run decides, so a success ends the retries.
- **Stop at the time limit** (`stop_at_limit`): the log is saved, then the
  tab is closed, which ends whatever runs in it.
- **Once and at start** (`schedule.rs`): `@once YYYY-MM-DD HH:MM` follows the
  missed-time rule like cron and switches the routine off once started;
  `@start` runs once per Herdr server (recorded in `started.json` against
  `Paths::server_id`, so a scheduler restart after an update does not
  count), and a new `@start` routine waits for the next start.
- **Check first** (`check_command`, agents only): runs in the folder in the
  login shell, 5-minute limit. Exit 0 starts the agent with the output
  (first 8000 characters) added to the prompt; anything else records
  "Nothing to do" without opening a tab.
- **Results and memory**: every agent prompt ends with a request for a
  summary between lines `RESULT START` and `RESULT END`, read from the saved
  log when the run ends (a shell command's result is its last output line).
  With `remember`, the plugin keeps the last ten results in
  `memory/<routine>.md` in the state folder and adds them to the next
  prompt. The plugin, not the agent, writes the file, so no agent needs
  write access outside its folder and any agent works the same.
- **Runs per routine**: the "n runs" link in the list (and `h`) filters
  the Runs view to one routine.

Why markers instead of a result file: writing outside the run's folder
would make agents in stricter modes stop and ask, and parsing the answer
works for every agent. Why routine ids stay slugs (not UUIDs, which the
owner asked about): the folder name is made once from the first name and
never changes on rename, runs and memory hang off it, and it stays readable
for people and agents who edit the files.

## 2026-11-10: A routine changed outside the list waits for approval

A routine runs only in the exact form the owner saved in the list or
approved there with `a`. `approved.json` in the state folder keeps the
approved content of `routine.toml` and the prompt or command file; the
scheduler, `fire` and "run now" compare it byte for byte with the files and
skip a routine that differs (no catch-up for times passed while it waited).
`a` reads the files once and shows every setting, every option and the
whole prompt or command, wrapped so nothing hides past the edge; `y`
works only once the owner has scrolled to the end, and approves exactly
the bytes shown (if the files changed meanwhile, nothing is approved).
Hidden characters are dropped from what is shown, and a routine that has
any in its name, folder, model, arguments or prompt shows a problem and
does not run. Saving the form approves what it saves for a routine that
was approved; one that waits keeps waiting, so `a` stays the one place
that approves a change made elsewhere. Saving one file in the editor
keeps another file's pending change pending; switching on or off reloads
the files and never approves a pending change. A run keeps its approved
prompt next to its record, so a prompt handed over later (after a
question at start) is the one approved at launch. When a waiting
routine's time comes, the scheduler notifies once per wait
(`waiting.json`). Routines from before this version wait for one
approval.

The cron features cover the same ground: the check command, retries,
stop at the limit and memory live in `routine.toml`, so they are part of
what is approved, and the approval view lists them (the check command
runs as a shell command before the agent). An `@once` routine that ran
is switched off with its approval kept, as `space` would. Stopping a run
at its limit closes its tab only when it is still the run's own, and the
memory folder is owner-only like the rest of the state folder.

Why: the security audit of 2026-11-10 found that any writer of the routine
files, such as an agent steered by text in a repository, could raise a
routine's permission mode, add permission flags or turn it into a shell
command that the scheduler then ran unattended. Agents are meant to write
routines ("Routines are plain files", "Create a routine by describing it to
Claude"), so the owner chose that every change made outside the list waits
for approval, Claude-made routines included.

Limits: `approved.json` is a file of the same user. A process that
deliberately writes it can still approve its own routine; such a process
has the owner's account anyway. The check stops agents that create or edit
routines the normal way. It does not cover the agent's own settings in the
routine's folder (such as `.claude/settings.json`), which the agent reads
at start as in any session there.

## 2026-11-10: Extra arguments cannot set permissions

For Claude Code, Codex and Gemini, `args` may not hold the flags that
set permissions (`--permission-mode`, `--dangerously-skip-permissions`,
`--sandbox`, `--approval-mode`, `--yolo`, Codex `-c sandbox_mode=…` and
the like); the routine shows the problem instead. The list shows `args`
under the routine's name, and "run now" and `a` show the full options.

Why: `args` come after the mode flags, so they could change the mode the
list shows without anyone seeing it.

## 2026-11-10: Owner-only files, safe writes

The config and state folders are `0700` and every file the plugin writes
is `0600`; folders an older version created are tightened at start.
`write_atomic` writes a new temporary file that must not exist yet
(`create_new`, so it never writes through a planted link) and renames it.

Why: logs hold up to 2000 lines of agent output, which can include secrets
that tools printed, and scripts and prompts can hold credentials. With the
default umask they were readable by other local accounts wherever the home
folder is.

## 2026-11-10: Runs are bound to their own agent and terminal

A run records the random `terminal_id` Herdr gives its pane. Before the
plugin reads a pane's output, closes its tab, shows it or decides it is
gone, it checks with `herdr pane get` that the pane id still has that
terminal and that tab. Agent calls (`agent get`, `agent prompt`) go to the
run's unique agent name instead of its pane id.

Why: pane and tab ids start over when Herdr restarts. With a reused socket
inode, a stale run could otherwise type its prompt into, copy the output of
or close another tab of the owner.

## 2026-11-10: One start per routine at a time

The scheduler and `fire` hold a lock file (`runs/<id>/.lock`, `flock`)
while they check that a routine is free and record its new run.

Why: the check and the record were two steps; two processes could both
pass the check and start the routine twice.

## 2026-11-10: Routine ids and run records are checked

A routine folder name must be lower case letters, digits and dashes (what
`new_id` makes); another name shows as a problem. A run record counts only
when its routine id and run id match the folder and file it is in.

Why: ids become file paths and part of the line a shell routine's tab runs,
and run records are read back for writes and deletes.

## 2026-11-10: No `--` before text arguments to Herdr

Herdr 0.9 reads the rest of the line as the text for `agent prompt`,
`tab rename` and `notification show`, also when it starts with `-`; a `--`
would become part of the text (tested with a throwaway session). So none is
added. Control characters are removed from a prompt before it is typed in,
and the line a shell routine's tab runs uses single quotes only for paths
without quotes or backslashes, which every common shell reads the same way.

## 2026-11-10: The build uses only the locked dependencies

`scripts/build.sh` runs `cargo build --release --locked` and stops when it
fails, instead of falling back to an unlocked build.

Why: the fallback resolved newer, unreviewed dependency versions on the
user's machine at install time. `rust-version = "1.89"` in `Cargo.toml`
names the oldest Rust that builds it (`File::lock` came in 1.89), so an
older toolchain fails with a clear message.

## 2026-11-10: Runs can outlive their routine

Deleting a routine with runs offers two buttons: "delete, keep runs" (`y`,
the first one) and "delete with runs" (`r`). Kept runs stay in their
folder, show in Runs as "deleted routine", can still be filtered with `h`,
and are deleted one by one with `d` in Runs; they are no longer pruned.
`new_id` skips ids that still have runs, so a new routine of the same name
never takes them over. The routine's memory file goes either way.

Why: the runs are the record of what happened (outputs, results), which
can matter after the routine itself is gone; deleting them stays one key
away.

## 2026-11-10: Classic cron jobs through the crontab

A shell routine can run from the system's crontab (`runs_in = "cron"`)
instead of Herdr's scheduler. Agents stay Herdr-only: an agent run needs a
tab.

*Replaced the same day* (see "Managed cron jobs stay real crontab lines"):
the first version kept these jobs in one marked block of the crontab that
called `herdr-routines cron-run <id>`.

Why: the owner wants to choose between a Herdr routine and a traditional
cron job.

## 2026-11-10: Managed cron jobs stay real crontab lines

A managed cron job is a group of lines in the crontab, at the place where
the job was:

```
# herdr-routines: <id> (its name and runs are in the herdr-routines list)
SHELL=<state>/bin/cron/<id>
<when> <command>                 (or "#off <when> <command>" when off)
SHELL=<the shell before the group, /bin/sh when none>
```

The crontab holds what cron knows (schedule, command, on or off); the
plugin's `routine.toml` holds the rest (name, time limit, retries, alerts,
runs kept). The list writes the group in place when a cron job is saved,
switched, copied, approved or deleted (after a backup in
`crontab-backups/`, last five kept), and reads the crontab back every 15
seconds and when it opens (`crontab::pull`): a line edited by hand wins
over the routine, a group whose routine is gone brings the routine back,
and an approved cron job whose group is gone is switched off. A routine
that waits for approval is not pulled; approving it writes its own
version. Taking over a job of the crontab ("keep as cron job") leaves its
line exactly as written and only adds the marker and the `SHELL` lines.
A cron job's command is one line, run in the home folder (where cron runs
it); a `%` is written as `\%`.

Cron runs a job as `$SHELL -c <command>`. `SHELL` points at a link to a
copy of the plugin binary in the state folder (the plugin's own folder
changes with every update), named after the job. The binary sees that it
was started from a `cron` folder with `-c` and acts as the job's wrapper:
it reads the plugin's folders, socket and Herdr binary from `env.json`
next to the links, runs the command with the group's previous shell,
records the run like any other (a run record with `cron_pid` instead of a
tab, which Herdr's scheduler leaves alone), prints the output for cron and
returns the command's exit code (124 at the time limit), retries and
alerts as the routine says, and skips while routines are switched off.
When its routine is missing it still runs the command, as cron would. `r`
in the list starts the same wrapper with the crontab's command in the
background (`HERDR_ROUTINES_TRIGGER=manual`), in its own process group so
closing the list does not stop it.

Why: the owner wants the cron job to stay where it is and be edited in the
list, with the extra information kept in the plugin, and its runs and
output recorded. `SHELL` is the one place cron offers to put something
between itself and a line without changing the line; a separate marked
block would have moved the job and hidden it behind a plugin command.

## 2026-11-10: Runs kept per routine

Each routine keeps its newest `keep_runs` runs (default 30) and none older
than `keep_days` days (default 90); 0 means no limit for either. Pruning
happens after each run, by the scheduler for Herdr runs and by the wrapper
for cron jobs; a run that is still going is never deleted. The form offers
a few presets ("Keep"), and a value set by hand stays choosable. The
global `history` setting is gone.

Why: the owner wants to set per routine how long results stay; a backup
job that runs every five minutes and a weekly review need different
histories.

## 2026-11-10: Separate tabs for routines and cron jobs

Routines (run by Herdr) and cron jobs (run by cron: the plugin's own and
the rest of the crontab) are two tabs, next to Runs. Each has its own
cursor; `n` in the Cron jobs tab preselects "Classic cron job".

Why: the two run differently (a tab while Herdr runs, or cron always) and
the crontab's other jobs are not managed here until taken over; mixing
them in one list made it unclear what runs where.

## 2026-11-10: One switch for all routines

The "routines on/off" chip (or `S`) switches all of them, after asking.
Off writes `paused` in the state folder and stops the scheduler; `start`
and `restart` then do not start one (also not when Herdr starts), and
the cron wrapper skips its times. On removes the marker and starts the
scheduler. Each routine keeps its own on or off.

Why: one place to stop everything, for example before travelling, that
holds across restarts; a stopped scheduler alone came back with the next
Herdr start and never covered cron jobs.

## 2026-11-10: Cron jobs move in as approved shell routines

`C` reads `crontab -l` and turns each job into a shell routine in the home
folder (where cron runs it), with the crontab's variables exported at the
top of the script and missed times skipped as cron does. Moving comments
the job's line out of the crontab (marked "moved to herdr-routines") after
a backup in the state folder; copying adds the routines switched off and
leaves the crontab alone. The routines are saved approved.

Why: commenting out instead of deleting keeps the crontab readable and
easy to undo, and a job must not run twice. The owner chose the jobs in the
list after seeing their lines, so they need no second approval; the
crontab is the owner's own file.

## 2026-11-10: Alerts as a popup that stays

A routine can tell its owner by a popup instead of a notification
(`alert` in `routine.toml`: `notification`, `popup-on-problems` or
`popup`). The popup is the plugin pane `alert`, opened with Herdr's
`popup` placement over whatever is in front (`herdr plugin pane open
--placement popup`, sized to its text, as the Multiaccount plugin does for
its reminder). It stays until OK; "Go to its tab" loads the run record and
uses `runner::show`, which checks the tab is still the run's own. When the
popup cannot open (no workspace, older Herdr), the notification goes out
instead. The texts reach the pane in one environment variable, with
control and hidden characters removed.

Why: a notification disappears after a few seconds, and a failed backup or
a question from an agent can wait unseen for hours. A popup in Herdr, not a
system dialog, keeps it inside the tool and needs nothing outside it.

## 2026-11-10: A copy starts off and keeps the approval state

`c` copies a routine's files as they are on disk (not the list's copy) into
a new folder, named "<name> copy", switched off, without runs or memory.
The copy is approved only when the original is.

Why: a copy that runs at once would double every run, and copying must not
turn a routine that waits for approval into an approved one.

## 2026-11-10: The list asks Herdr for right clicks

Herdr keeps right clicks in a pane for its own pane menu unless the pane
routes them to its program. When the list starts in a Herdr pane, it runs
`herdr pane input <its pane> --right-click pane` so its own menu opens;
when it closes, it routes them back to Herdr (the pane may be a shell the
list was started in by hand).

Why: without it, a right click never reached the list and the menu could
not open in Herdr, only in the tests.

## 2026-11-10: macOS and Linux only

The plugin declares `platforms = ["linux", "macos"]`, like the owner's
other Herdr plugins. It builds and passes clippy for Linux
(`cargo clippy --target x86_64-unknown-linux-gnu`) but has not been run
there. Windows is not supported: the code uses Unix files and processes
(links, file modes, process groups, `/bin/sh`) and cron, and Herdr's
plugin build runs `sh`.

Why: Herdr runs on Windows, but a Windows version would need the Task
Scheduler instead of cron and PowerShell instead of `sh`, which is a
separate piece of work for the owner to decide on.

## 2026-11-11: Only an approved cron job reaches the crontab

`crontab::write_job` writes nothing for a routine that waits for approval,
whoever calls it (switching, the form, the editor, a copy). Its group in
the crontab stays the one the owner approved until `a` approves the
change. `space` on such a cron job calls `crontab::switch`, which only
puts `#off ` in front of the approved line or takes it away. The cron
wrapper reads the routine as it was approved (`Approvals::routine`, from
the approved file contents in `approved.json`), never the files on disk,
and never approves: after a single run on a date it saves the approved
routine switched off only when the files are still the approved ones, and
otherwise just switches the crontab line off. Alerts and the pruning of
the history use the approved settings too; a routine file that no longer
reads prunes nothing.

Why: the security review of 2026-11-11 found that keys which never ask
(`space`, `Esc` in the editor) wrote a changed command into the crontab,
and that the wrapper approved whatever was on disk when a dated job ran.
Both let something with file access run a command the owner never saw.

## 2026-11-11: Saving is judged against what was opened

The form and the editor remember the routine's files as read when they
opened and whether those were approved (`App::opened`). What they save
stays approved only when that was so and the files are unchanged since.
"Run now" reads the routine again after the `y` (`App::fire`,
`crontab::run_now`, the `fire` command).

Why: judged at the moment of saving, text that came from a change made
elsewhere could be approved by saving it, if the files were put back in
between.

## 2026-11-11: A routine too long to show cannot be approved in the list

The approval and output views scroll over at most 65,535 rows (the type
ratatui scrolls with). An approval view with more rows shows its end,
says so and does not offer `y`. The output view shows the end of a long
output (the last 2 MB), and `f` opens the whole file.

Why: `y` is offered once the view is scrolled to its end, and beyond
65,535 rows the end could not be reached, so text nobody could scroll to
would have been approved. Refusing is one small change in `draw_log`;
scrolling with a wider type would have touched the whole view while the
list is being reworked. A prompt of that length is not one a person
reads in a list anyway.

## 2026-11-11: The crontab is changed as bytes, under a lock, on a fresh read

The crontab is read and written as lines of bytes (`crontab::Tab`). A
change replaces the lines of one group and leaves all others as they are:
text that is not UTF-8 and Windows line ends survive, and a managed job
whose time and command did not change keeps its own line byte for byte.
Every change goes through `crontab::change`: under a lock file in the
state folder it reads the crontab, works out the new one, reads again and
installs only when nothing changed in between, else it starts over (three
times, then an error). The first backup ever (`crontab-first.txt`) is
never deleted; of the later ones the last five stay.

Why: reading the crontab as text replaced bytes that are not UTF-8 in
other people's lines; two changes at once (a second list, a wrapper
switching a dated job off) could lose one; and five rolling backups were
gone after five changes, the original crontab with them. `crontab` has no
"replace only if unchanged", so a short moment between the last read and
the install remains; the lock closes it for the plugin's own processes.

## 2026-11-11: Managed cron jobs can be handed back; no uninstall hook

`herdr-routines release-cron` turns every managed group back into the
job's own plain line (commented out when it was off). The README says to
run it before the plugin or its state folder is removed.

Why: a managed job is started through `SHELL=<state>/bin/cron/<id>`.
Once that folder is gone, cron cannot start the job and reports nothing.
Herdr's plugin manifest has build, startup, pane and action entries but
no entry that runs when a plugin is removed, so this cannot be done for
the owner; it is a command and a warning in the README instead.

## 2026-11-11: A dated cron job is checked against its date

The wrapper runs a cron job set for one date (`@once`) only on that date,
as long as its crontab line is still the one written for it.

Why: a crontab line has no year. If the run on the date did not happen or
the job was not switched off afterwards, cron would start it again every
year.

## 2026-11-11: Commands run in their own process group, without shell text

Check commands, cron jobs and the tools the plugin calls are started as
the leader of a new process group, and the time limit stops the whole
group (`killpg`). Standard output and errors are joined through one pipe
that this process makes, not by putting `exec 2>&1;` in front of the
command. A cron job's output goes to its file and on to cron as it comes;
in memory stay its first 256 KB and its last 16 KB. A cron job gets cron's
input on its first try.

Why: stopping only the shell left what it had started running after the
limit. `exec 2>&1` is not understood by every shell a crontab can name
(csh, fish). Holding the whole output in memory twice had no bound. This
adds `libc` (MIT OR Apache-2.0, already a dependency of the other crates)
as a direct dependency for the one `killpg` call; the standard library has
no way to signal a process group.

## 2026-11-11: A process is known by its pid and start time

The pid file and a cron run's record hold the process's start time as
`ps -o lstart=` prints it, next to the pid. A scheduler or cron run counts
as alive only when both match. Records without a start time (written by
an older version) fall back to the process name.

Why: the name alone depends on where the plugin is installed, is cut
short on Linux, and matches any other process of the plugin; a pid is
handed out again.

## 2026-11-11: The scheduler stays while its server is there

Every `herdr` and `crontab` call has a time limit (20 seconds, 90 for
`agent start`). A round first asks Herdr something cheap and stops when
that fails. After about ten minutes of failures the scheduler no longer
exits: while the server's socket is still its own and something listens
on it, it tries once a minute. It exits when the socket changed (a new
server starts its own scheduler) or when nobody listens on it any more.
A `start` from another Herdr session that finds this scheduler notes its
socket in `waiting-servers`; when this scheduler's server goes, it starts
a scheduler for the first of those that still listens.

Why: only Herdr's startup hook starts the scheduler (see "The scheduler
runs inside Herdr"). One that gave up while the server was only slow left
no scheduler until the next Herdr restart, without a word; and a session
whose `start` had been turned away never got one after the other session
ended.

## 2026-11-11: Limits on what is read and kept

Routine files, the memory file and state files a routine can influence
are read only when they are plain files of at most 1 MB
(`paths::read_limited`), so a pipe or a device in their place cannot hang
or fill the reader. `Routine::check` bounds the numbers (3 retries at
least a minute apart, a year of time limit, 100,000 runs, 36,500 days).
`routines.log` is moved aside at 2 MB. Histories are pruned when a run
ends and otherwise every 60 rounds, not in every round. "Skipped" and
"Nothing to do" records are counted apart from real runs.

Why: a `keep_days` beyond the calendar stopped the scheduler with a
panic, and unbounded retries, files and logs could fill the disk or keep
the scheduler busy. Notes of skipped times pushed real runs and their
output out of a short history.

## 2026-11-11: Text from runs goes into prompts as data

A check command's output and remembered results are cut (8,000 and 2,000
characters), lose characters that do not show, have code fences replaced
and are put into the prompt inside a fenced block with the sentence that
they are information, not instructions (`runner::as_data`).

Why: this text can hold anything the run read. Unmarked and unbounded, it
was the easiest way for outside text to steer a later run. This lowers
that risk and cannot remove it: the agent still reads the text.

## 2026-11-11: More options are closed to extra arguments, and all are flagged

Added to "Extra arguments cannot set permissions": options that start
other programs or load code or settings (Claude Code `--mcp-config`,
`--plugin-dir`, `--plugin-url`, `--agents`, `--agent`,
`--setting-sources`; Codex `-c mcp_servers.…`, `-c notify=…`, hooks,
`-c model_provider=…`; Gemini `--extensions`,
`--allowed-mcp-server-names`), that switch safeguards off (Claude Code
`--bare`, `--safe-mode`; Codex `--enable`, `--disable`; Gemini
`--sandbox`, `--no-sandbox`), that widen where the agent may write (Codex
`--add-dir`, `-C`; Gemini `--include-directories`) or send the session
elsewhere or give it more to act on (Claude Code `--remote-control`,
`--cloud`, `--environment`, `--teleport`, `--desktop`, `--bg`, `--chrome`,
`--debug-file`). The approval view also shows a warning line whenever a
routine has extra arguments at all.

Why: a list of forbidden options can never be complete, because agents
gain options, so the warning is the part that holds. Claude Code's
options were checked against its `--help` on this date; Codex and Gemini
were not installed here, so theirs are the ones known from their
documentation, not checked against an installed version, and should be
checked by someone who has them. Claude Code's
`--add-dir` stays allowed: the README names it as the example of a useful
extra argument, and the approval view shows it.

## 2026-11-11: Tests cannot reach Herdr

In tests, `herdr::program` returns a stand-in set per thread with
`herdr::use_fake`, and none by default, like `crontab::use_fake`.

Why: tests ran the `herdr` in `HERDR_BIN_PATH` or on the PATH, so a test
run inside a Herdr session could send notifications to it or open panes.

## 2026-11-11: ratatui without its default features

`ratatui` is built with `crossterm`, `layout-cache` and `underline-color`
only. `scripts/check.sh` runs `cargo audit` when it is installed and says
so when it is not.

Why: the default features also bring the macros crate and the calendar
widget, which the list does not use; fewer dependencies are less to
audit. The checks never install tools themselves.

## 2026-11-11: The footer and the mouse belong to what is open

`ui::draw` draws the page (list, form or editor), then clears the click
areas when a dialog is open, then draws the dialog and last the footer. So
under a dialog only the dialog's own areas and its keys in the footer can be
clicked. The footer's keys come from one place (`footer_keys`), by what is
open: the list (by tab and by what the row can do), the form (by the kind of
field), the editor, or the dialog. Every dialog is drawn through `dialog()`,
which records its place, and a click beside it closes it like Esc. Dialogs
stay between the header and the footer and scroll or drop their gaps when
the window is smaller than they are.

Why: the footer kept the list's keys under a dialog. With the delete
question open, a click where "r run now" was sent `r`, which there means
"delete with all its runs".

## 2026-11-11: Lists keep a scroll offset, and the cursor its item

Each list (routines, cron jobs, runs) has its own offset in `App::tops`.
Drawing moves it only when a key moved the cursor out of the window
(`App::follow`); the wheel changes the offset and leaves the cursor, and a
click never changes it. A reload puts each cursor back on the routine, cron
job or run it was on (by id, not by row number), and a scrolled list moves
its offset along with rows that came in above. Lists, lists to choose from,
the form, the help and the output view say which part shows.

Why: the offset was worked out from the cursor on every draw, so a click in
a scrolled list moved the rows under the mouse and the second click of a
double click landed on another row. A new run on top moved the cursor to
another run.

## 2026-11-11: One mouse rule, no double click

A click marks a row or a field; a click on what is marked opens it. What
does only one thing acts on the first click: tabs, header chips, footer
keys, buttons, `‹` `›`, the entries of a list to choose from, "More
settings", the prompt row and the "n runs ›" text (only the text, not its
cell). The row under the mouse is lit. There is no timing: the second click
may come at any time.

Why: rows needed a double click, lists to choose from a click and then
Enter, some cells acted on one click and others not. A rule without timing
also works over slow connections.

## 2026-11-11: A short form, the rest under "More settings"

The form shows name, "Runs with", the prompt, when and where (for a shell
command also "Runs in"), then one row "More settings" that shows or hides
everything else: permission mode, model, effort, check, earlier results,
extra options, on or off, missed times, time limit, retries, alerts and run
history. The hidden fields keep the defaults they had, for new routines and
when editing; the row sums up in a few words what is set there. The
save button is outside the scrolling fields, so it always shows. Text
fields have a cursor (`Form::cursor`, counted in characters).

Why: a new routine needs four things, and the form asked for twenty. The
schedule fields and their logic are unchanged.

## 2026-11-11: The list takes the shape that fits

`routine_shape` picks one of three shapes by width: all columns (from 80
columns), the last run in the routine's second line, or the routine alone
with last run and next time in its second line. Column widths come from
what the rows hold, within limits; when room is short the schedule gives
way first, down to a floor, and the name takes the rest.
Every cell is cut with "…" (`clip_line`), never at the edge. The details
show whenever rows are free below the list, not only in a tall window.

Why: below 94 columns the "Last run" column was dropped, so a failed run
was invisible in a split pane, where the list is used most.

## 2026-11-11: The header is a list of chips with a defined order

`chips()` returns the header's chips left to right, each with a long and a
short label, a style, its key and whether it is a warning (`keep`).
`plans()` lists the ways to fit them, tried in order: all long, without the
title, the chips that are no warning short, those dropped one by one from
the left, the tabs without counts, the tabs with short names, and at last
the warning short. A tab is never cut and a warning never goes. A new chip
is one more entry in `chips()`.

Why: the header cut the last tab in a narrow window and dropped "routines
off", the one thing that must be seen, before the title.

## 2026-11-11: Problems stay in the footer until the next key

A message takes the room beside the keys when it is short and the whole
footer, over up to three rows, when it is long. Red and yellow messages
stay until the next key or click; the others go after five seconds as
before. One that is longer than three rows ends in "…".

Why: messages were cut at the footer's width without a sign, and an error
went away after five seconds whether it was read or not.

## 2026-11-11: Which questions Enter answers

`y` always answers yes and has a button named after what it does. Enter
answers yes only where the same key undoes it: all routines on or off,
autostart, taking over cron jobs. Deleting, discarding (form or editor) and
running by hand need `y` itself, as the project's rules ask. Keeping or
moving cron jobs now asks first, since both change the crontab; copying
does not.

Why: some questions took Enter and others not, with nothing on screen to
tell them apart, and `C` then `m` changed the crontab without asking.

## 2026-11-11: One name per thing

The list, the form, the details, the menu, the help and the README use one
word each: "cron job" (not "classic cron job"), "Runs with" (the agent or
shell command), "Extra options", "Alerts", "Run history", "Switched" on or
off, "duplicate", "take over" (a job that is "only in your crontab"),
"autostart", "output" of a run. File keys (`args`, `keep_runs`, `alert`)
keep their names.

Why: the same thing had up to three names in different places, so the help
did not match the form and the README matched neither.
