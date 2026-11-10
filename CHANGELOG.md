# Changelog

All notable changes to this plugin. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
- `routines` (JSON) and `fire <id>` commands for scripts and other agents.
