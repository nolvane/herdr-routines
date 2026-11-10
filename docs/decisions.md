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
