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
