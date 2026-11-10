//! Drawing: the routines and runs tables, the form, the prompt editor,
//! dialogs and a footer, in the look of the other Herdr plugins of this
//! author. Records clickable areas for the mouse.

use crate::agent;
use crate::app::{App, Hit, LogView, Mode, View, INTRO};
use crate::cron::short_time;
use crate::form::{Field, Form};
use crate::routine::Routine;
use crate::schedule::Schedule;
use crate::store::Run;
use crate::theme::{
    self, Icons, ACCENT, BAR_BG, BLUE, BORDER, FAINT, GREEN, MARK_BG, MUTED, RED, SELECTED_BG,
    TEXT, YELLOW,
};
use chrono::Local;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    let height = frame.area().height;
    let spacer = if height >= 14 { 1 } else { 0 };
    let [header, _, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(spacer),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let icons = Icons {
        nerd: app.icons.nerd,
    };
    draw_header(frame, app, &icons, header);
    if app.editor.is_some() {
        draw_editor(frame, app, body);
    } else if app.form.is_some() {
        draw_form(frame, app, body);
    } else {
        match app.view {
            View::Routines | View::Cron => draw_routines(frame, app, &icons, body),
            View::Runs => draw_runs(frame, app, &icons, body),
        }
    }
    draw_footer(frame, app, footer);
    match app.mode.clone() {
        Mode::Help => draw_help(frame, app),
        Mode::Pick(p) => draw_picker(frame, app, &p),
        Mode::Input(i) => draw_input(frame, &i),
        Mode::Confirm(c) => draw_confirm(frame, app, &c),
        Mode::Message { title, lines } => draw_message(frame, app, &title, &lines),
        Mode::Log(l) => draw_log(frame, app, l),
        Mode::Cron(jobs) => draw_cron(frame, app, &jobs),
        Mode::Normal | Mode::AskQuit => {}
    }
}

fn bold(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

fn draw_header(frame: &mut Frame, app: &mut App, icons: &Icons, area: Rect) {
    let line = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    let mut spans = Vec::new();
    let in_list = app.editor.is_none() && app.form.is_none();
    if let Some(ed) = &app.editor {
        spans.push(Span::styled(
            format!("{} {}", icons.title(), ed.title),
            bold(ACCENT),
        ));
    } else if let Some(form) = &app.form {
        let title = match &form.id {
            Some(_) => format!("Edit {}", form.name.trim()),
            None => "New routine".to_string(),
        };
        spans.push(Span::styled(
            format!("{} {title}", icons.title()),
            bold(ACCENT),
        ));
    } else {
        let title = format!("{} Scheduled routines   ", icons.title());
        let mut x = line.x + title.width() as u16;
        spans.push(Span::styled(title, bold(ACCENT)));
        for view in View::ORDER {
            let count = match view {
                View::Routines => app.routines.iter().filter(|r| !r.is_cron()).count(),
                View::Cron => {
                    app.routines.iter().filter(|r| r.is_cron()).count() + app.crontab_jobs.len()
                }
                View::Runs => app.runs.len(),
            };
            let label = match (view, app.filter_name()) {
                (View::Runs, Some(name)) => {
                    let name: String = name.chars().take(24).collect();
                    format!(" Runs of {name} ({count}) ")
                }
                _ => format!(" {} ({count}) ", view.name()),
            };
            let style = if app.view == view {
                Style::new()
                    .fg(TEXT)
                    .bg(SELECTED_BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(MUTED)
            };
            let w = label.width() as u16;
            app.hits.push((Rect::new(x, line.y, w, 1), Hit::View(view)));
            x += w + 1;
            spans.push(Span::styled(label, style));
            spans.push(Span::raw(" "));
        }
    }
    let left = Line::from(spans);
    let used = left.width();
    frame.render_widget(Paragraph::new(left), line);

    // Whether routines run at all (the scheduler); a click switches them.
    let (chip, style) = match app.scheduler {
        Some(_) => (
            format!("{} routines on", icons.running()),
            Style::new().fg(GREEN),
        ),
        None => ("○ routines off".to_string(), Style::new().fg(RED)),
    };
    // Left of it, whether Herdr starts at login (A switches it).
    let login = if !in_list || !crate::autostart::supported() {
        None
    } else if crate::autostart::is_on() {
        Some((
            format!("{} autostart on", icons.running()),
            Style::new().fg(GREEN),
        ))
    } else {
        Some(("○ autostart off".to_string(), Style::new().fg(FAINT)))
    };
    let w = chip.width();
    let login_w = login.as_ref().map_or(0, |(t, _)| t.width() + 3);
    let right = line.x + line.width;
    if used + w + 3 <= line.width as usize {
        if in_list {
            let x = right - w as u16;
            app.hits
                .push((Rect::new(x, line.y, w as u16, 1), Hit::Key('S')));
        }
        let mut spans = Vec::new();
        if let Some((text, style)) = login.filter(|_| used + w + login_w + 3 <= line.width as usize)
        {
            let lw = text.width() as u16;
            let x = right - w as u16 - 3 - lw;
            app.hits.push((Rect::new(x, line.y, lw, 1), Hit::Key('A')));
            spans.push(Span::styled(text, style));
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(chip, style));
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
            line,
        );
    }
}

// ---- tables --------------------------------------------------------------

/// Room between columns: wider on wide panes.
fn spacing(width: u16) -> u16 {
    if width >= 80 {
        3
    } else {
        1
    }
}

/// The column headings on a bar of their own.
fn heading_row<'a>(names: impl IntoIterator<Item = &'a str>) -> Row<'a> {
    Row::new(names.into_iter().map(Cell::from)).style(
        Style::new()
            .fg(MUTED)
            .bg(BAR_BG)
            .add_modifier(Modifier::BOLD),
    )
}

/// The first row to show so the selected one is visible.
fn offset(selected: usize, rows: usize, visible: usize) -> usize {
    if visible == 0 || rows <= visible {
        return 0;
    }
    selected.saturating_sub(visible - 1).min(rows - visible)
}

/// Columns that fit: the fixed ones drop from the right before the main
/// column gets narrower than `min`.
fn fit<C: Copy + PartialEq>(
    mut cols: Vec<(C, &'static str, Constraint)>,
    drop: &[C],
    width: u16,
    min: u16,
) -> Vec<(C, &'static str, Constraint)> {
    let gap = spacing(width);
    for d in drop {
        let fixed: u16 = cols
            .iter()
            .map(|(_, _, c)| match c {
                Constraint::Length(n) => n + gap,
                _ => gap,
            })
            .sum();
        if width >= fixed + min {
            break;
        }
        cols.retain(|(c, _, _)| c != d);
    }
    cols
}

fn empty_note(frame: &mut Frame, area: Rect, lines: &[(&str, Color)]) {
    let text: Vec<Line> = lines
        .iter()
        .map(|(t, c)| Line::from(Span::styled(t.to_string(), Style::new().fg(*c))))
        .collect();
    frame.render_widget(
        Paragraph::new(text).wrap(ratatui::widgets::Wrap { trim: true }),
        Rect::new(
            area.x + 2,
            area.y,
            area.width.saturating_sub(4),
            area.height,
        ),
    );
}

/// Draws a table of two-line rows with a gap, and records each row.
fn draw_table(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    head: Vec<&str>,
    widths: Vec<Constraint>,
    rows: Vec<Vec<Text<'static>>>,
    selected: usize,
) {
    let row_h = ROW_H;
    let visible = (area.height.saturating_sub(1) / row_h).max(1) as usize;
    let first = offset(selected, rows.len(), visible);
    let mut table_rows = Vec::new();
    for (i, cells) in rows.into_iter().enumerate().skip(first).take(visible) {
        let mut row = Row::new(cells.into_iter().map(Cell::from)).height(2);
        if i == selected {
            row = row.style(Style::new().bg(SELECTED_BG));
        }
        table_rows.push(row);
        let y = area.y + 1 + ((i - first) as u16) * row_h;
        if y + 2 <= area.y + area.height {
            app.hits
                .push((Rect::new(area.x, y, area.width, 2), Hit::Row(i)));
        }
    }
    let table = Table::new(table_rows, widths)
        .header(heading_row(head))
        .column_spacing(spacing(area.width.saturating_sub(2)));
    frame.render_widget(
        table,
        Rect::new(
            area.x + 1,
            area.y,
            area.width.saturating_sub(2),
            area.height,
        ),
    );
}

fn faint(t: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(t.into(), Style::new().fg(FAINT)))
}

/// The second line under a routine's name: its agent settings and folder,
/// or what keeps it from running.
fn routine_detail(r: &Routine) -> Line<'static> {
    if let Some(p) = &r.problem {
        return Line::from(Span::styled(p.clone(), Style::new().fg(RED)));
    }
    if !r.approved {
        return Line::from(Span::styled(
            "Changed outside the list · a approves",
            Style::new().fg(YELLOW),
        ));
    }
    let f = &r.file;
    if r.is_shell() {
        let first = r
            .prompt
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let kind = if r.is_cron() { "cron job" } else { "shell" };
        return faint(format!(
            "{kind} · {} · {}",
            clip(first.trim(), 40),
            f.folder
        ));
    }
    let mut parts = vec![f.agent.clone()];
    if !f.model.is_empty() {
        parts.push(agent::label(agent::models(&f.agent), &f.model));
    }
    if !f.permission_mode.is_empty() {
        parts.push(agent::label(agent::modes(&f.agent), &f.permission_mode));
    }
    if !f.effort.is_empty() {
        parts.push(format!(
            "{} effort",
            agent::label(agent::efforts(&f.agent), &f.effort).to_lowercase()
        ));
    }
    // Extra arguments can change what the agent does: always in sight.
    if !f.args.is_empty() {
        parts.push(clip(&f.args.join(" "), 40));
    }
    parts.push(f.folder.clone());
    faint(parts.join(" · "))
}

#[derive(Clone, Copy, PartialEq)]
enum RCol {
    Mark,
    Routine,
    Schedule,
    Last,
}

fn draw_routines(frame: &mut Frame, app: &mut App, icons: &Icons, area: Rect) {
    let cron_view = app.view == View::Cron;
    let routines: Vec<Routine> = app.listed().into_iter().cloned().collect();
    let crontab_jobs = if cron_view {
        app.crontab_jobs.clone()
    } else {
        Vec::new()
    };
    if routines.is_empty() && crontab_jobs.is_empty() {
        let path = crate::paths::shorten(&app.paths.routines_dir());
        let lines: Vec<(String, Color)> = if cron_view {
            vec![
                ("No cron jobs.".into(), TEXT),
                (String::new(), TEXT),
                (
                    "n, then \"Classic cron job\", creates one: a command cron runs, also when Herdr is closed.".into(),
                    MUTED,
                ),
                (
                    "Jobs already in your crontab show here too.".into(),
                    FAINT,
                ),
            ]
        } else {
            vec![
                ("No routines yet.".into(), TEXT),
                (String::new(), TEXT),
                (
                    "n creates one: fill in a short form, or describe it to Claude.".into(),
                    MUTED,
                ),
                (format!("Routines are plain files in {path}."), FAINT),
            ]
        };
        let refs: Vec<(&str, Color)> = lines.iter().map(|(t, c)| (t.as_str(), *c)).collect();
        empty_note(frame, area, &refs);
        return;
    }
    let cols = fit(
        vec![
            (RCol::Mark, "", Constraint::Length(2)),
            (
                RCol::Routine,
                if cron_view { "Cron job" } else { "Routine" },
                Constraint::Fill(1),
            ),
            (RCol::Schedule, "Schedule", Constraint::Length(26)),
            (RCol::Last, "Last run", Constraint::Length(LAST_W)),
        ],
        &[RCol::Last, RCol::Schedule],
        area.width.saturating_sub(2),
        24,
    );
    let now = Local::now();
    let rows: Vec<Vec<Text<'static>>> = routines
        .iter()
        .map(|r| {
            let on = r.file.enabled && r.can_run();
            cols.iter()
                .map(|(c, _, _)| match c {
                    RCol::Mark => {
                        let (icon, color) = icons.routine(r.file.enabled, !r.can_run());
                        Text::from(Span::styled(icon, Style::new().fg(color)))
                    }
                    RCol::Routine => Text::from(vec![
                        Line::from(Span::styled(
                            r.file.name.clone(),
                            bold(if r.file.enabled { TEXT } else { MUTED }),
                        )),
                        routine_detail(r),
                    ]),
                    RCol::Schedule => {
                        let second = if r.problem.is_some() {
                            faint("–")
                        } else if !r.approved {
                            Line::from(Span::styled("waits for approval", Style::new().fg(YELLOW)))
                        } else if !r.file.enabled {
                            Line::from(Span::styled("off", Style::new().fg(RED)))
                        } else {
                            match r.schedule() {
                                Some(crate::schedule::Schedule::AtStart) if r.is_cron() => {
                                    faint("next computer start")
                                }
                                Some(crate::schedule::Schedule::AtStart) => {
                                    faint("next Herdr start")
                                }
                                s => match s.and_then(|c| c.next_after(&now)) {
                                    Some(t) => {
                                        faint(format!("next {}", crate::app::until(&t, &now)))
                                    }
                                    None => faint("done"),
                                },
                            }
                        };
                        Text::from(vec![
                            Line::from(Span::styled(
                                r.schedule_text(),
                                Style::new().fg(if on { TEXT } else { MUTED }),
                            )),
                            second,
                        ])
                    }
                    RCol::Last => match app.latest_run(&r.id) {
                        Some(run) => last_run(icons, run, &now, app.run_count(&r.id)),
                        None => Text::from(vec![faint("–"), faint("no run yet")]),
                    },
                })
                .collect()
        })
        .collect();
    // The person's own cron jobs follow: cron runs them, not this plugin.
    let mut rows = rows;
    for job in &crontab_jobs {
        let when = Schedule::parse(&job.schedule).ok();
        let next = match &when {
            Some(Schedule::AtStart) => faint("next computer start"),
            Some(s) => match s.next_after(&now) {
                Some(t) => faint(format!("next {}", crate::app::until(&t, &now))),
                None => faint("–"),
            },
            None => faint("–"),
        };
        let words = match &when {
            Some(Schedule::AtStart) => "When the computer starts".to_string(),
            Some(s) => s.describe(),
            None => job.schedule.clone(),
        };
        rows.push(
            cols.iter()
                .map(|(c, _, _)| match c {
                    RCol::Mark => Text::from(Span::styled("◇", Style::new().fg(BLUE))),
                    RCol::Routine => Text::from(vec![
                        Line::from(Span::styled(
                            crate::crontab::name_for(&job.command),
                            bold(TEXT),
                        )),
                        faint(format!("in your crontab · {}", clip(&job.command, 50))),
                    ]),
                    RCol::Schedule => Text::from(vec![
                        Line::from(Span::styled(words.clone(), Style::new().fg(TEXT))),
                        next.clone(),
                    ]),
                    RCol::Last => Text::from(vec![
                        Line::from(Span::styled("not followed here", Style::new().fg(MUTED))),
                        faint("⏎ manage it here"),
                    ]),
                })
                .collect(),
        );
    }
    let head = cols.iter().map(|(_, h, _)| *h).collect();
    let widths = cols.iter().map(|(_, _, w)| *w).collect();
    let sel = app.list_sel();
    let n = rows.len();
    // In a tall enough window the selected routine's details sit right
    // under the list.
    let table_h = 1 + ROW_H * n as u16;
    let (list, details) = if area.height >= DETAILS_FROM && area.width >= 50 {
        let list_h = table_h.min(area.height - DETAILS_MIN);
        let details = Rect::new(
            area.x,
            area.y + list_h + 1,
            area.width,
            area.height - list_h - 1,
        );
        (
            Rect {
                height: list_h,
                ..area
            },
            Some(details),
        )
    } else {
        (area, None)
    };
    draw_table(frame, app, list, head, widths, rows, sel);
    // The last-run cell links to the routine's runs: the column is the
    // table's last one, LAST_W wide, flush right inside the side margins.
    if cols.last().is_some_and(|(c, _, _)| *c == RCol::Last) {
        let x = list.x + list.width.saturating_sub(1 + LAST_W);
        let visible = (list.height.saturating_sub(1) / ROW_H).max(1) as usize;
        let first = offset(sel, n, visible);
        for (i, r) in routines.iter().enumerate().skip(first).take(visible) {
            let y = list.y + 1 + ((i - first) as u16) * ROW_H;
            if app.latest_run(&r.id).is_some() && y < list.y + list.height {
                app.hits.push((Rect::new(x, y, LAST_W, 1), Hit::RunsOf(i)));
            }
        }
    }
    if let Some(area) = details {
        if let Some(r) = routines.get(sel) {
            draw_details(frame, app, icons, r, area);
        } else if let Some(job) = app.selected_cron_job().cloned() {
            draw_cron_details(frame, &job, area);
        }
    }
}

/// The person's own cron job in full, and what ⏎ does with it.
fn draw_cron_details(frame: &mut Frame, job: &crate::crontab::CronJob, area: Rect) {
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let w = inner.width as usize;
    let title = format!(" {}", crate::crontab::name_for(&job.command));
    let rule = w.saturating_sub(title.width() + 2);
    let mut lines = vec![Line::from(vec![
        Span::styled(title, bold(ACCENT)),
        Span::styled(format!(" {}", "─".repeat(rule)), Style::new().fg(BORDER)),
    ])];
    const LW: usize = 15;
    let room = w.saturating_sub(LW + 2).max(10);
    let row = |label: &str, text: String, color: Color| {
        Line::from(vec![
            Span::styled(format!(" {label:<LW$}"), Style::new().fg(MUTED)),
            Span::styled(text, Style::new().fg(color)),
        ])
    };
    lines.push(row("Line", clip(job.line.trim(), room), TEXT));
    if !job.env.is_empty() {
        let vars: Vec<String> = job.env.iter().map(|(k, _)| k.clone()).collect();
        lines.push(row("Variables", clip(&vars.join(", "), room), TEXT));
    }
    if let Some(note) = &job.note {
        lines.push(row("Note", clip(note, room), YELLOW));
    }
    lines.push(Line::from(""));
    for l in wrap(
        "Cron runs this job, not Herdr, so this list does not see its runs. ⏎ manages it here: keep it as a cron job (it still runs when Herdr is closed) or move it to Herdr, and see its output, result and failures in Runs.",
        w.saturating_sub(2),
    ) {
        lines.push(Line::from(Span::styled(
            format!(" {l}"),
            Style::new().fg(FAINT),
        )));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Rows per routine or run in the lists: name and details, no gap.
const ROW_H: u16 = 2;
/// The details of the selected routine show from this list height on,
/// with at least DETAILS_MIN rows for them.
const DETAILS_FROM: u16 = 24;
const DETAILS_MIN: u16 = 11;

/// The selected routine in full: everything the form sets, its last run
/// and the start of its prompt or command.
fn draw_details(frame: &mut Frame, app: &App, icons: &Icons, r: &Routine, area: Rect) {
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let w = inner.width as usize;
    let f = &r.file;
    let now = Local::now();
    let mut lines: Vec<Line> = Vec::new();
    let title = format!(" {}", r.file.name);
    let rule = w.saturating_sub(title.width() + 2);
    lines.push(Line::from(vec![
        Span::styled(title, bold(ACCENT)),
        Span::styled(format!(" {}", "─".repeat(rule)), Style::new().fg(BORDER)),
    ]));
    const LW: usize = 15;
    let row = |label: &str, value: Vec<Span<'static>>| {
        let mut spans = vec![Span::styled(
            format!(" {label:<LW$}"),
            Style::new().fg(MUTED),
        )];
        spans.extend(value);
        Line::from(spans)
    };
    let text = |s: String| vec![Span::styled(s, Style::new().fg(TEXT))];
    let room = w.saturating_sub(LW + 2);
    if let Some(p) = &r.problem {
        lines.push(row(
            "Problem",
            vec![Span::styled(clip(p, room), Style::new().fg(RED))],
        ));
    } else if !r.approved {
        lines.push(row(
            "Approval",
            vec![Span::styled(
                "changed outside the list · a shows it and approves",
                Style::new().fg(YELLOW),
            )],
        ));
    }
    // Schedule and when it runs next.
    let mut when = vec![Span::styled(r.schedule_text(), Style::new().fg(TEXT))];
    let next = if !f.enabled {
        Some(Span::styled("off", Style::new().fg(RED)))
    } else {
        match r.schedule() {
            Some(crate::schedule::Schedule::AtStart) => None,
            s => s.and_then(|c| c.next_after(&now)).map(|t| {
                Span::styled(
                    format!("next {}", short_time(&t, &now)),
                    Style::new().fg(FAINT),
                )
            }),
        }
    };
    if let Some(next) = next {
        when.push(Span::styled(" · ", Style::new().fg(FAINT)));
        when.push(next);
    }
    lines.push(row("Schedule", when));
    if r.is_shell() {
        lines.push(row(
            "Runs",
            text(if r.is_cron() {
                "a classic cron job, from your crontab (also when Herdr is closed)".into()
            } else {
                "a shell command, in a Herdr tab".into()
            }),
        ));
    } else {
        let mode = if f.permission_mode.is_empty() {
            "agent's own permissions".to_string()
        } else {
            agent::label(agent::modes(&f.agent), &f.permission_mode)
        };
        let model = if f.model.is_empty() {
            "default model".to_string()
        } else {
            agent::label(agent::models(&f.agent), &f.model)
        };
        let mut agent_words = format!("{} · {mode} · {model}", f.agent);
        if !f.effort.is_empty() {
            agent_words.push_str(&format!(
                " · {} effort",
                agent::label(agent::efforts(&f.agent), &f.effort).to_lowercase()
            ));
        }
        lines.push(row("Agent", text(clip(&agent_words, room))));
    }
    lines.push(row("Folder", text(clip(&f.folder, room))));
    let mut limits = format!(
        "{} minute{}, then {}",
        f.time_limit_minutes,
        if f.time_limit_minutes == 1 { "" } else { "s" },
        if f.stop_at_limit {
            "stops"
        } else {
            "keeps running"
        }
    );
    limits.push_str(&match f.retries {
        0 => " · no retry".to_string(),
        1 => format!(" · retries once after {} min", f.retry_delay_minutes),
        n => format!(" · retries {n}× after {} min", f.retry_delay_minutes),
    });
    if f.alert != crate::routine::Alert::Notification {
        limits.push_str(&format!(" · {}", f.alert.label().to_lowercase()));
    }
    lines.push(row("Time limit", text(clip(&limits, room))));
    if !r.is_shell() {
        let mut extras = Vec::new();
        if !f.check_command.is_empty() {
            extras.push(format!("checks first: {}", f.check_command));
        }
        if f.remember {
            extras.push("uses earlier results".into());
        }
        if !f.args.is_empty() {
            extras.push(format!("options: {}", f.args.join(" ")));
        }
        if !extras.is_empty() {
            lines.push(row("Extras", text(clip(&extras.join(" · "), room))));
        }
    }
    let last = match app.latest_run(&r.id) {
        Some(run) => {
            let (icon, color) = icons.status(run.status);
            let mut v = vec![
                Span::styled(
                    format!("{icon} {}", run.status.label()),
                    Style::new().fg(color),
                ),
                Span::styled(
                    format!(" · {}", crate::app::ago(&run.started, &now)),
                    Style::new().fg(FAINT),
                ),
            ];
            let used = Line::from(v.clone()).width() + 3;
            if let Some(first) = run.result.lines().find(|l| !l.trim().is_empty()) {
                v.push(Span::styled(
                    format!(" · {}", clip(first.trim(), room.saturating_sub(used))),
                    Style::new().fg(TEXT),
                ));
            }
            v
        }
        None => vec![Span::styled("none yet", Style::new().fg(FAINT))],
    };
    lines.push(row("Last run", last));
    // The prompt or command fills the rest.
    let label = if r.is_shell() { "Command" } else { "Prompt" };
    let left = (inner.height as usize).saturating_sub(lines.len());
    if left > 0 {
        let mut body: Vec<String> = Vec::new();
        for l in r.prompt.lines() {
            if body.len() >= left {
                break;
            }
            if l.trim().is_empty() && body.is_empty() {
                continue;
            }
            body.extend(wrap(l, room.max(10)));
        }
        let more = body.len() > left;
        body.truncate(left);
        if more {
            if let Some(last) = body.last_mut() {
                last.push('…');
            }
        }
        if body.is_empty() {
            lines.push(row(
                label,
                vec![Span::styled("empty", Style::new().fg(FAINT))],
            ));
        }
        for (i, b) in body.into_iter().enumerate() {
            let shown = if i == 0 { label } else { "" };
            lines.push(row(shown, vec![Span::styled(b, Style::new().fg(MUTED))]));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Width of the last-run column.
const LAST_W: u16 = 28;

/// The last run of a routine and how many runs it has (a link to them).
fn last_run(
    icons: &Icons,
    run: &Run,
    now: &chrono::DateTime<Local>,
    count: usize,
) -> Text<'static> {
    let (icon, color) = icons.status(run.status);
    let when = crate::app::ago(&run.started, now);
    let runs = if count == 1 {
        "1 run".to_string()
    } else {
        format!("{count} runs")
    };
    Text::from(vec![
        Line::from(vec![
            Span::styled(
                format!("{icon} {}", run.status.label()),
                Style::new().fg(color),
            ),
            Span::raw("  "),
            Span::styled(format!("{runs} ›"), Style::new().fg(ACCENT)),
        ]),
        faint(if run.status.active() {
            format!("{when} · for {}", crate::app::took(run))
        } else {
            format!("{when} · took {}", crate::app::took(run))
        }),
    ])
}

#[derive(Clone, Copy, PartialEq)]
enum NCol {
    Mark,
    Run,
    Started,
    Status,
}

fn draw_runs(frame: &mut Frame, app: &mut App, icons: &Icons, area: Rect) {
    if app.runs.is_empty() {
        empty_note(
            frame,
            area,
            &[
                ("No runs yet.", TEXT),
                ("", TEXT),
                (
                    "Runs show up here when a routine is due, or when you start one with r.",
                    MUTED,
                ),
            ],
        );
        return;
    }
    let cols = fit(
        vec![
            (NCol::Mark, "", Constraint::Length(2)),
            (NCol::Run, "Run", Constraint::Fill(1)),
            (NCol::Started, "Started", Constraint::Length(14)),
            (NCol::Status, "Status", Constraint::Length(18)),
        ],
        &[NCol::Started, NCol::Status],
        area.width.saturating_sub(2),
        24,
    );
    let now = Local::now();
    let runs = app.runs.clone();
    let rows: Vec<Vec<Text<'static>>> = runs
        .iter()
        .map(|run| {
            let (icon, color) = icons.status(run.status);
            cols.iter()
                .map(|(c, _, _)| match c {
                    NCol::Mark => Text::from(Span::styled(icon, Style::new().fg(color))),
                    NCol::Run => {
                        let mut first = vec![Span::styled(run.routine_name.clone(), bold(TEXT))];
                        // Runs kept when their routine was deleted.
                        if !app.routines.iter().any(|r| r.id == run.routine_id) {
                            first.push(Span::styled("  deleted routine", Style::new().fg(FAINT)));
                        }
                        let open = !run.tab_closed
                            && !run.pane_id.is_empty()
                            && run.status != crate::store::Status::Closed;
                        if open {
                            first.push(Span::styled(
                                format!("  {} tab open", icons.tab_open()),
                                Style::new().fg(ACCENT),
                            ));
                        }
                        // What it came to, else what went wrong, else how
                        // it started.
                        let result = run.result.lines().find(|l| !l.trim().is_empty());
                        let second = match result {
                            Some(r) => r.trim().to_string(),
                            None if !run.message.is_empty() => run.message.clone(),
                            None => {
                                format!("{} · {}", run.agent, run.trigger.label().to_lowercase())
                            }
                        };
                        Text::from(vec![Line::from(first), faint(second)])
                    }
                    NCol::Started => Text::from(vec![
                        Line::from(Span::styled(
                            short_time(&run.started, &now),
                            Style::new().fg(TEXT),
                        )),
                        faint(if run.attempt > 0 {
                            format!("{} {}", run.trigger.label(), run.attempt)
                        } else {
                            run.trigger.label().to_string()
                        }),
                    ]),
                    NCol::Status => Text::from(vec![
                        Line::from(Span::styled(run.status.label(), Style::new().fg(color))),
                        faint(if run.status.active() {
                            format!("for {}", crate::app::took(run))
                        } else {
                            format!("took {}", crate::app::took(run))
                        }),
                    ]),
                })
                .collect()
        })
        .collect();
    let head = cols.iter().map(|(_, h, _)| *h).collect();
    let widths = cols.iter().map(|(_, _, w)| *w).collect();
    let sel = app.sel_run;
    draw_table(frame, app, area, head, widths, rows, sel);
}

// ---- form ----------------------------------------------------------------

const LABEL_W: usize = 18;

/// `s` cut to `width` columns, with an ellipsis when cut.
fn clip(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// A field's value; `room` is the width it has.
fn field_value(form: &Form, f: Field, selected: bool, room: usize) -> Vec<Span<'static>> {
    let text = |s: String| Span::styled(s, Style::new().fg(TEXT));
    let cursor = || Span::styled("▏", Style::new().fg(ACCENT));
    match f {
        Field::Prompt => {
            let first = form
                .prompt
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .to_string();
            if first.is_empty() {
                return vec![Span::styled("empty · ⏎ writes it", Style::new().fg(YELLOW))];
            }
            let n = form.prompt.trim_end().lines().count();
            let more = if n > 1 {
                format!("  ({n} lines)")
            } else {
                String::new()
            };
            vec![
                text(clip(&first, room.saturating_sub(more.width()).max(10))),
                Span::styled(more, Style::new().fg(FAINT)),
            ]
        }
        f if f.is_choice() => {
            // ‹ › show that ← → step through the choices.
            let arrow = Style::new().fg(if selected { ACCENT } else { FAINT });
            let value = if f == Field::Enabled {
                Span::styled(
                    form.shown(f),
                    Style::new().fg(if form.enabled { GREEN } else { RED }),
                )
            } else {
                text(form.shown(f))
            };
            let mut v = vec![Span::styled("‹ ", arrow), value, Span::styled(" ›", arrow)];
            let detail = form.detail(f);
            // The short detail only when it fits whole; the explanation
            // below the form says more.
            let left = room.saturating_sub(form.shown(f).width() + 6);
            if !detail.is_empty() && detail.width() <= left {
                v.push(Span::styled(format!("  {detail}"), Style::new().fg(FAINT)));
            }
            v
        }
        f => {
            let value = match f {
                Field::Name => form.name.clone(),
                Field::Args => form.args.clone(),
                Field::Folder => form.folder.clone(),
                Field::Time => form.time.clone(),
                Field::Cron => form.cron.clone(),
                Field::Limit => form.limit.clone(),
                Field::Date => form.date.clone(),
                Field::Check => form.check.clone(),
                _ => String::new(),
            };
            let value = if f == Field::Check {
                clip(&value, room.saturating_sub(1).max(10))
            } else {
                value
            };
            let mut v = Vec::new();
            if value.is_empty() && !selected {
                let empty = match f {
                    Field::Name => "required",
                    Field::Args => "none",
                    Field::Check => "none, always runs",
                    _ => "",
                };
                v.push(Span::styled(empty, Style::new().fg(FAINT)));
            } else {
                v.push(text(value));
            }
            if selected {
                v.push(cursor());
            }
            if f == Field::Limit {
                v.push(Span::styled(" minutes", Style::new().fg(FAINT)));
            }
            v
        }
    }
}

fn button(label: &str, on: bool) -> Span<'static> {
    if on {
        Span::styled(
            format!(" {label} "),
            Style::new()
                .fg(theme::on(ACCENT))
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!(" {label} "), Style::new().fg(TEXT).bg(BORDER))
    }
}

/// A row of the form: a part's heading, an empty row, or a field (by its
/// place in `Form::fields`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum FormRow {
    Head(&'static str),
    Blank,
    Field(usize),
    /// A further line of the prompt's preview (1 is its second line).
    More(usize),
}

/// The schedule in words for the Schedule heading, such as "runs weekdays
/// at 09:00 · next Wed 09:00", or what is wrong with it.
fn schedule_summary(form: &crate::form::Form) -> Span<'static> {
    let now = Local::now();
    match form.schedule() {
        Ok(c) => {
            let schedule = Schedule::parse(&c).expect("checked");
            let next = schedule
                .next_after(&now)
                .map(|t| format!(" · next {}", short_time(&t, &now)))
                .unwrap_or_default();
            // "Weekdays at 09:00" reads "runs weekdays at 09:00".
            let words = schedule.describe();
            let mut chars = words.chars();
            let words = match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect::<String>(),
                None => words,
            };
            Span::styled(format!("runs {words}{next}"), Style::new().fg(GREEN))
        }
        Err(e) => Span::styled(e, Style::new().fg(RED)),
    }
}

fn draw_form(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(form) = app.form.as_deref().cloned() else {
        return;
    };
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let fields = form.fields();
    let current = form.current();
    // The rows: a heading at the start of each part of the form, and an
    // empty row before the save button.
    // An empty row closes each part, so the parts read apart; in a window
    // too low for everything, only the one before the save button stays.
    let layout = |spaced: bool| {
        let mut rows: Vec<FormRow> = Vec::new();
        let mut last = "";
        for (i, f) in fields.iter().enumerate() {
            let s = form.section(*f);
            if s != last {
                if s.is_empty() || (spaced && !rows.is_empty()) {
                    rows.push(FormRow::Blank);
                }
                if !s.is_empty() {
                    rows.push(FormRow::Head(s));
                }
                last = s;
            }
            rows.push(FormRow::Field(i));
        }
        rows
    };
    let mut rows = layout(true);
    // Room needed below the fields: gaps and the explanation (5 rows).
    if rows.len() + 5 > inner.height as usize {
        rows = layout(false);
    }
    // The prompt shows its first lines, as many as fit (up to eight), and
    // ends in "…" when it goes on.
    let value_w = (inner.width as usize).saturating_sub(LABEL_W + 2).max(10);
    let mut preview: Vec<String> = Vec::new();
    for l in form.prompt.trim_end().lines() {
        if preview.is_empty() && l.trim().is_empty() {
            continue;
        }
        preview.extend(wrap(&l.replace('\t', "    "), value_w));
    }
    let prompt_at = fields.iter().position(|f| *f == Field::Prompt);
    let free = (inner.height as usize).saturating_sub(rows.len() + 3 + 2);
    let extra = preview.len().saturating_sub(1).min(free).min(7);
    if let Some(p) = prompt_at {
        if let Some(at) = rows.iter().position(|r| *r == FormRow::Field(p)) {
            for j in (1..=extra).rev() {
                rows.insert(at + 1, FormRow::More(j));
            }
        }
    }
    let cut = preview.len() > extra + 1;
    let preview_line = |j: usize| -> Vec<Span<'static>> {
        let mut line = preview.get(j).cloned().unwrap_or_default();
        if cut && j == extra {
            // Room for the "…" that says it goes on.
            let w = line.width();
            if w + 2 > value_w {
                line = clip(&line, value_w.saturating_sub(2));
                line = line.trim_end_matches('…').to_string();
            }
            return vec![
                Span::styled(line, Style::new().fg(TEXT)),
                Span::styled(" …", Style::new().fg(ACCENT)),
            ];
        }
        vec![Span::styled(line, Style::new().fg(TEXT))]
    };
    // Below the fields: what the current field means, set off by an accent
    // bar like a tooltip. A click on a field selects it and so shows its
    // explanation.
    let mut hint = wrap(form.hint(current), inner.width.saturating_sub(4) as usize);
    // Up to four lines, fewer when the fields would no longer all fit.
    let spare = (inner.height as usize).saturating_sub(rows.len() + 3);
    let hint_rows = spare.clamp(2, 4);
    if hint.len() > hint_rows {
        hint.truncate(hint_rows);
        if let Some(last) = hint.last_mut() {
            last.push('…');
        }
    }
    let hint_shown = hint.len().min(hint_rows) as u16;
    // gap, title, text, and a gap before the footer
    let below = if inner.height >= 12 {
        3 + hint_shown
    } else {
        0
    };
    let list_h = inner.height.saturating_sub(below);
    let sel = rows
        .iter()
        .position(|r| *r == FormRow::Field(form.field))
        .unwrap_or(0);
    let mut first = offset(sel, rows.len(), list_h as usize);
    // Keep the heading of the selected field's part in view.
    if first == sel && sel > 0 && matches!(rows[sel - 1], FormRow::Head(_)) {
        first -= 1;
    }
    let schedule_words = schedule_summary(&form);
    for (r, layout_row) in rows.iter().enumerate() {
        if r < first || r >= first + list_h as usize {
            continue;
        }
        let y = inner.y + (r - first) as u16;
        let row = Rect::new(inner.x, y, inner.width, 1);
        let i = match layout_row {
            FormRow::Blank => continue,
            FormRow::Head(name) => {
                let mut spans = vec![Span::styled(format!(" {name}"), bold(ACCENT))];
                if *name == "Schedule" {
                    spans.push(Span::raw("  "));
                    spans.push(schedule_words.clone());
                }
                let used = Line::from(spans.clone()).width();
                let rule = (inner.width as usize).saturating_sub(used + 2);
                if rule > 2 {
                    spans.push(Span::styled(
                        format!(" {}", "─".repeat(rule)),
                        Style::new().fg(BORDER),
                    ));
                }
                frame.render_widget(Paragraph::new(Line::from(spans)), row);
                continue;
            }
            FormRow::More(j) => {
                // A further line of the prompt; it belongs to the field.
                let Some(p) = prompt_at else { continue };
                if p == form.field {
                    frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
                }
                app.hits.push((row, Hit::Field(p)));
                let mut spans = vec![Span::raw(" ".repeat(LABEL_W + 1))];
                spans.extend(preview_line(*j));
                frame.render_widget(Paragraph::new(Line::from(spans)), row);
                continue;
            }
            FormRow::Field(i) => *i,
        };
        let f = &fields[i];
        let selected = i == form.field;
        if selected {
            frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
        }
        app.hits.push((row, Hit::Field(i)));
        let label = format!(" {:<w$}", form.label(*f), w = LABEL_W);
        let label_style = if selected {
            bold(TEXT)
        } else {
            Style::new().fg(MUTED)
        };
        let mut spans = vec![Span::styled(label, label_style)];
        if *f == Field::Save {
            let x = inner.x + 1 + LABEL_W as u16;
            spans = vec![Span::raw(" ".repeat(LABEL_W + 1))];
            spans.push(button("⏎ save", selected));
            spans.push(Span::raw("  "));
            spans.push(button("Esc back", false));
            app.hits
                .push((Rect::new(x + 10, y, 10, 1), Hit::Key('\u{1b}')));
            // Whether it can be saved, right beside the button.
            spans.push(Span::raw("   "));
            spans.push(match form.build() {
                Ok(_) => Span::styled("ready to save", Style::new().fg(GREEN)),
                Err(e) => Span::styled(format!("not ready: {e}"), Style::new().fg(YELLOW)),
            });
        } else if *f == Field::Prompt && !preview.is_empty() {
            spans.extend(preview_line(0));
        } else {
            let room = (inner.width as usize).saturating_sub(LABEL_W + 2);
            spans.extend(field_value(&form, *f, selected, room));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }
    if below == 0 || inner.height <= list_h {
        return;
    }
    // Right under the fields, so it reads as part of the form.
    let shown = (rows.len() - first).min(list_h as usize) as u16;
    let y = inner.y + shown - 1;
    if below > 2 {
        let bar = || Span::styled("▎ ", Style::new().fg(ACCENT));
        let mut lines = vec![Line::from(vec![
            bar(),
            Span::styled(form.label(current), bold(TEXT)),
        ])];
        lines.extend(
            hint.into_iter()
                .take(hint_rows)
                .map(|l| Line::from(vec![bar(), Span::styled(l, Style::new().fg(MUTED))])),
        );
        frame.render_widget(
            Paragraph::new(Text::from(lines)),
            Rect::new(inner.x + 1, y + 2, inner.width.saturating_sub(2), below - 2),
        );
    }
}

// ---- editor --------------------------------------------------------------

fn draw_editor(frame: &mut Frame, app: &mut App, area: Rect) {
    let typing = matches!(app.mode, Mode::Normal);
    let Some(ed) = app.editor.as_mut() else {
        return;
    };
    let outer = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if typing { ACCENT } else { BORDER }));
    let inner = block.inner(outer);
    frame.render_widget(block, outer);
    let text = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let width = text.width.saturating_sub(1).max(1) as usize;
    let height = text.height.max(1) as usize;
    if ed.text.width != width || ed.text.height != height {
        ed.text.width = width;
        ed.text.height = height;
        ed.text.follow();
    }
    app.hits.push((text, Hit::Text));
    let rows = ed.text.rows();
    if ed.text.lines.len() == 1 && ed.text.lines[0].is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "Write what the agent should do, as you would type it to the agent.",
                Style::new().fg(FAINT),
            )),
            Rect::new(text.x + 1, text.y, text.width.saturating_sub(1), 1),
        );
    }
    let marked = ed.text.selection();
    let plain = Style::new().fg(TEXT);
    let mark = Style::new().fg(TEXT).bg(MARK_BG);
    let lines: Vec<Line> = rows
        .iter()
        .skip(ed.text.top)
        .take(height)
        .map(|&(l, s, e)| {
            // Runs of marked and unmarked characters.
            let mut spans: Vec<Span> = Vec::new();
            let mut run = String::new();
            let mut run_marked = false;
            for col in s..e {
                let c = ed.text.lines[l][col];
                let m = marked.is_some_and(|(a, b)| (l, col) >= a && (l, col) < b);
                if m != run_marked && !run.is_empty() {
                    let style = if run_marked { mark } else { plain };
                    spans.push(Span::styled(std::mem::take(&mut run), style));
                }
                run_marked = m;
                if c == '\t' {
                    run.push_str("    ");
                } else {
                    run.push(c);
                }
            }
            // A marked line break shows as one marked space.
            let next_marked = marked.is_some_and(|(a, b)| (l, e) >= a && (l, e) < b);
            let last_of_line = e == ed.text.lines[l].len();
            if !run.is_empty() {
                let style = if run_marked { mark } else { plain };
                spans.push(Span::styled(run, style));
            }
            if last_of_line && next_marked {
                spans.push(Span::styled(" ", mark));
            }
            Line::from(spans)
        })
        .collect();
    if !(ed.text.lines.len() == 1 && ed.text.lines[0].is_empty()) {
        frame.render_widget(Paragraph::new(Text::from(lines)), text);
    }
    let (cy, cx) = ed.text.cursor();
    if typing && cy >= ed.text.top && cy < ed.text.top + height {
        frame.set_cursor_position((text.x + cx as u16, text.y + (cy - ed.text.top) as u16));
    }
    // More below or above: say so in the border.
    if rows.len() > height && outer.height >= 3 {
        let pct = (ed.text.top + height).min(rows.len()) * 100 / rows.len();
        let label = format!(" {pct}% ");
        let w = label.width() as u16;
        if outer.width > w + 4 {
            frame.render_widget(
                Paragraph::new(Span::styled(label, Style::new().fg(FAINT))),
                Rect::new(outer.x + outer.width - w - 2, outer.bottom() - 1, w, 1),
            );
        }
    }
}

// ---- footer ----------------------------------------------------------------

fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.render_widget(Block::new().style(Style::new().bg(BAR_BG)), area);
    if app.mode == Mode::AskQuit {
        let question = " Close the list? Routines keep running. ";
        let qw = question.width() as u16;
        let line = Line::from(vec![
            Span::styled(question, bold(TEXT)),
            Span::styled(
                " y close ",
                Style::new()
                    .fg(theme::on(RED))
                    .bg(RED)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(" any other key stays ", Style::new().fg(TEXT).bg(BORDER)),
        ]);
        app.hits
            .push((Rect::new(area.x + qw, area.y, 9, 1), Hit::Key('y')));
        app.hits
            .push((Rect::new(area.x + qw + 10, area.y, 21, 1), Hit::Key('n')));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let right = if let Some((text, color, _)) = &app.flash {
        Span::styled(format!("{text} "), Style::new().fg(*color))
    } else if let Some(ed) = &app.editor {
        Span::styled(
            format!("line {} of {} ", ed.text.row + 1, ed.text.lines.len()),
            Style::new().fg(FAINT),
        )
    } else {
        Span::raw("")
    };
    let right_w = (right.width() as u16).min(area.width / 2);
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(right_w)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        right_area,
    );

    let keys: Vec<(&str, &str, Option<Hit>)> = if app.editor.is_some() {
        vec![
            ("Esc", "done", Some(Hit::Key('\u{1b}'))),
            ("Ctrl+Z", "undo", Some(Hit::Ctrl('z'))),
            ("Ctrl+Y", "redo", Some(Hit::Ctrl('y'))),
        ]
    } else if let Some(form) = &app.form {
        let f = form.current();
        let enter = match f {
            Field::Prompt => "edit prompt",
            Field::Save => "save",
            f if f.is_choice() => "choose",
            _ => "next",
        };
        let mut v = vec![("⏎", enter, Some(Hit::Key('\n')))];
        if f.is_choice() {
            v.push(("← →", "change", None));
        }
        v.extend([
            ("↑ ↓", "field", None),
            ("Ctrl+S", "save", Some(Hit::Ctrl('s'))),
            ("Esc", "back", Some(Hit::Key('\u{1b}'))),
        ]);
        if !f.is_text() {
            v.push(("?", "all keys", Some(Hit::Key('?'))));
        }
        v
    } else {
        let pending = app
            .selected_routine()
            .is_some_and(|r| !r.approved && !r.unreadable);
        match app.view {
            View::Cron if app.selected_cron_job().is_some() => vec![
                ("⏎", "manage it here", Some(Hit::Key('\n'))),
                ("n", "new", Some(Hit::Key('n'))),
                ("C", "all cron jobs", Some(Hit::Key('C'))),
                ("?", "all keys", Some(Hit::Key('?'))),
                ("q", "close", Some(Hit::Key('q'))),
            ],
            View::Routines | View::Cron => vec![
                ("⏎", "edit", Some(Hit::Key('\n'))),
                ("n", "new", Some(Hit::Key('n'))),
                if pending {
                    ("a", "approve", Some(Hit::Key('a')))
                } else {
                    ("r", "run now", Some(Hit::Key('r')))
                },
                ("space", "on/off", Some(Hit::Key(' '))),
                ("?", "all keys", Some(Hit::Key('?'))),
                ("q", "close", Some(Hit::Key('q'))),
            ],
            View::Runs => {
                let mut v = vec![
                    ("⏎", "go to its tab", Some(Hit::Key('\n'))),
                    ("l", "output", Some(Hit::Key('l'))),
                ];
                if app.run_filter.is_some() {
                    v.push(("Esc", "all runs", Some(Hit::Key('\u{1b}'))));
                } else {
                    v.push(("h", "only this routine", Some(Hit::Key('h'))));
                }
                v.extend([
                    ("?", "all keys", Some(Hit::Key('?'))),
                    ("q", "close", Some(Hit::Key('q'))),
                ]);
                v
            }
        }
    };
    let mut spans = vec![Span::raw(" ")];
    let mut x = left_area.x + 1;
    for (key, what, click) in keys {
        let text = format!(" {what}  ");
        let width = (key.width() + text.width()) as u16;
        if x + width > left_area.x + left_area.width {
            break;
        }
        if let Some(hit) = click {
            app.hits.push((Rect::new(x, area.y, width - 2, 1), hit));
        }
        x += width;
        spans.push(Span::styled(key, bold(MUTED)));
        spans.push(Span::styled(text, Style::new().fg(FAINT)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), left_area);
}

// ---- dialogs ---------------------------------------------------------------

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width
        .min(area.width.saturating_sub(2))
        .max(10.min(area.width));
    let h = height
        .min(area.height.saturating_sub(2))
        .max(3.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn dialog(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .title(Span::styled(format!(" {title} "), bold(ACCENT)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    )
}

fn hints(spans: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut out = Vec::new();
    for (k, w) in spans {
        out.push(Span::styled(*k, bold(MUTED)));
        out.push(Span::styled(*w, Style::new().fg(FAINT)));
    }
    Line::from(out)
}

fn draw_picker(frame: &mut Frame, app: &mut App, p: &crate::app::Picker) {
    // Details on a line of their own, except in long lists (the agents),
    // where they follow the name.
    let any = p.items.iter().any(|i| !i.detail.is_empty());
    let two = any && p.items.len() <= 8;
    let per = if two { 2u16 } else { 1 };
    // Long lists get a filter line on top instead of number keys.
    let filter_h = u16::from(p.filters()) * 2;
    let want_h = p.all.len() as u16 * per + 4 + filter_h;
    let area = centered(frame.area(), 72, want_h);
    let mut inner = dialog(frame, area, &p.title);
    if p.filters() {
        let line = Line::from(vec![
            Span::styled("Filter ", Style::new().fg(MUTED)),
            Span::styled(p.filter.clone(), bold(TEXT)),
            Span::styled("▏", Style::new().fg(ACCENT)),
            Span::styled(
                if p.filter.is_empty() {
                    "type to narrow the list"
                } else {
                    ""
                },
                Style::new().fg(FAINT),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        inner = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(2),
        );
    }
    let list_h = inner.height.saturating_sub(2);
    let visible = (list_h / per).max(1) as usize;
    let first = offset(p.sel, p.items.len(), visible);
    if p.items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("Nothing matches", Style::new().fg(FAINT))),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
    }
    for (i, item) in p.items.iter().enumerate().skip(first).take(visible) {
        let y = inner.y + ((i - first) as u16) * per;
        let r = Rect::new(inner.x, y, inner.width, per.min(inner.y + list_h - y));
        let style = if i == p.sel {
            Style::new().bg(SELECTED_BG)
        } else {
            Style::new()
        };
        let number = if i < 9 && !p.filters() {
            format!("{} ", i + 1)
        } else {
            "  ".into()
        };
        let mut first = vec![
            Span::styled(number, Style::new().fg(FAINT)),
            Span::styled(item.label.clone(), bold(TEXT)),
        ];
        if !two && !item.detail.is_empty() {
            first.push(Span::styled(
                format!("  {}", item.detail),
                Style::new().fg(MUTED),
            ));
        }
        let mut lines = vec![Line::from(first)];
        if two {
            lines.push(Line::from(Span::styled(
                format!("  {}", item.detail),
                Style::new().fg(MUTED),
            )));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)).style(style), r);
        app.hits.push((r, Hit::PickRow(i)));
    }
    let foot = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    frame.render_widget(
        Paragraph::new(hints(&[("⏎", " choose  "), ("Esc", " back")])),
        foot,
    );
}

fn draw_input(frame: &mut Frame, i: &crate::app::Input) {
    let area = centered(frame.area(), 72, 9);
    let inner = dialog(frame, area, &i.title);
    let field = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), field);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {}", i.text), Style::new().fg(TEXT)),
            Span::styled("▏", Style::new().fg(ACCENT)),
        ])),
        field,
    );
    if inner.height > 3 {
        frame.render_widget(
            Paragraph::new(Text::from(
                wrap(&i.hint, inner.width as usize)
                    .into_iter()
                    .map(|l| Line::from(Span::styled(l, Style::new().fg(MUTED))))
                    .collect::<Vec<_>>(),
            )),
            Rect::new(
                inner.x,
                inner.y + 3,
                inner.width,
                inner.height.saturating_sub(4),
            ),
        );
    }
    let foot = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    frame.render_widget(
        Paragraph::new(hints(&[("⏎", " ok  "), ("Esc", " cancel")])),
        foot,
    );
}

fn draw_confirm(frame: &mut Frame, app: &mut App, c: &crate::app::Confirm) {
    let width = centered(frame.area(), 76, 1).width;
    let wrapped: Vec<Line> = c
        .lines
        .iter()
        .flat_map(|l| {
            // An empty line stays as a gap between parts.
            if l.trim().is_empty() {
                vec![String::new()]
            } else {
                wrap(l, width.saturating_sub(6) as usize)
            }
        })
        .map(|l| Line::from(Span::styled(l, Style::new().fg(MUTED))))
        .collect();
    let mut area = centered(frame.area(), width, wrapped.len() as u16 + 5);
    if area.height < wrapped.len() as u16 + 5 {
        area.y = frame.area().y;
        area.height = frame.area().height;
    }
    let inner = dialog(frame, area, &c.question);
    let (top, rest) = if inner.height < wrapped.len() as u16 + 3 {
        (0, 1)
    } else {
        (1, 3)
    };
    frame.render_widget(
        Paragraph::new(Text::from(wrapped)),
        Rect::new(
            inner.x,
            inner.y + top,
            inner.width,
            inner.height.saturating_sub(rest),
        ),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    // Deleting a routine with runs: keep them (y) or delete them too (r).
    let with_runs = matches!(c.purpose, crate::app::ConfirmFor::Delete(_, true));
    let mut buttons: Vec<(String, char, Style)> = vec![(
        if with_runs {
            " y delete, keep runs ".into()
        } else {
            " y yes ".into()
        },
        'y',
        Style::new()
            .fg(theme::on(GREEN))
            .bg(GREEN)
            .add_modifier(Modifier::BOLD),
    )];
    if with_runs {
        buttons.push((
            " r delete with runs ".into(),
            'r',
            Style::new().fg(TEXT).bg(BORDER),
        ));
    }
    buttons.push((" n no ".into(), 'n', Style::new().fg(TEXT).bg(BORDER)));
    let mut spans = Vec::new();
    let mut x = inner.x;
    for (text, key, style) in buttons {
        let w = text.width() as u16;
        app.hits.push((Rect::new(x, y, w, 1), Hit::Key(key)));
        x += w + 1;
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, y, inner.width, 1),
    );
}

fn draw_message(frame: &mut Frame, app: &mut App, title: &str, text: &[String]) {
    let width = 78u16.min(frame.area().width.saturating_sub(2)).max(24);
    let mut lines: Vec<Line> = Vec::new();
    for (i, t) in text.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        let style = Style::new().fg(if i == 0 { TEXT } else { MUTED });
        for l in wrap(t, width.saturating_sub(6) as usize) {
            lines.push(Line::from(Span::styled(l, style)));
        }
    }
    let area = centered(frame.area(), width, lines.len() as u16 + 5);
    let inner = dialog(frame, area, title);
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(3),
        ),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " ⏎ ok ",
            Style::new()
                .fg(theme::on(ACCENT))
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        )])),
        Rect::new(inner.x, y, inner.width, 1),
    );
    app.hits.push((Rect::new(inner.x, y, 6, 1), Hit::Key('\n')));
}

/// The jobs in the crontab, and the choice to move or copy them here.
fn draw_cron(frame: &mut Frame, app: &mut App, jobs: &[crate::crontab::CronJob]) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(110);
    let text_w = width.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for job in jobs {
        let when = match Schedule::parse(&job.schedule) {
            // In the crontab, `@reboot` means the computer's start.
            Ok(Schedule::AtStart) => "When the computer starts".to_string(),
            Ok(s) => s.describe(),
            Err(_) => job.schedule.clone(),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{when}  "), bold(TEXT)),
            Span::styled(
                clip(
                    &job.command,
                    text_w.saturating_sub(when.width() + 2).max(10),
                ),
                Style::new().fg(MUTED),
            ),
        ]));
        if let Some(note) = &job.note {
            lines.push(Line::from(Span::styled(
                format!("  {note}"),
                Style::new().fg(YELLOW),
            )));
        }
    }
    lines.push(Line::from(""));
    for l in wrap(
        "Keep as cron job: cron still runs it, also when Herdr is closed, but you see, edit and switch it here, with its output and result in Runs. Move to Herdr: Herdr runs it in a tab instead. Both comment the old line in your crontab out (a backup is kept), so nothing runs twice. Copy: added to Herdr switched off; your crontab stays as it is.",
        text_w,
    ) {
        lines.push(Line::from(Span::styled(l, Style::new().fg(FAINT))));
    }
    if !crate::autostart::is_on() {
        for l in wrap(
            "Autostart is off: jobs moved to Herdr run only while Herdr runs; kept as cron jobs they always run. Turn autostart on at the top for Herdr to start at login.",
            text_w,
        ) {
            lines.push(Line::from(Span::styled(l, Style::new().fg(YELLOW))));
        }
    }
    let height = (lines.len() as u16 + 4).min(screen.height.saturating_sub(2));
    let area = centered(screen, width, height);
    let title = if jobs.len() == 1 {
        "1 cron job in your crontab".to_string()
    } else {
        format!("{} cron jobs in your crontab", jobs.len())
    };
    let inner = dialog(frame, area, &title);
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(2),
        ),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    let mut x = inner.x;
    let mut spans = Vec::new();
    for (key, what) in [
        ("k", "keep as cron job"),
        ("m", "move to Herdr"),
        ("c", "copy, off"),
        ("Esc", "back"),
    ] {
        let w = (key.width() + what.width() + 3) as u16;
        let hit = if key == "Esc" {
            '\u{1b}'
        } else {
            key.chars().next().unwrap_or(' ')
        };
        app.hits.push((Rect::new(x, y, w, 1), Hit::Key(hit)));
        x += w;
        spans.push(Span::styled(key, bold(MUTED)));
        spans.push(Span::styled(format!(" {what}  "), Style::new().fg(FAINT)));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, y, inner.width, 1),
    );
}

fn draw_log(frame: &mut Frame, app: &mut App, mut log: LogView) {
    let screen = frame.area();
    let area = centered(
        screen,
        screen.width.saturating_sub(4).min(140),
        screen.height.saturating_sub(2),
    );
    let inner = dialog(frame, area, &log.title);
    let visible = inner.height.saturating_sub(1);
    // Long lines wrap, so nothing hides past the right edge.
    let rows: Vec<String> = log
        .lines
        .iter()
        .flat_map(|l| hard_wrap(&l.replace('\t', "    "), inner.width as usize))
        .collect();
    let total = rows.len().min(u16::MAX as usize) as u16;
    log.max = total.saturating_sub(visible);
    log.scroll = log.scroll.min(log.max);
    let lines: Vec<Line> = rows
        .into_iter()
        .skip(log.scroll as usize)
        .take(visible as usize)
        .map(|l| Line::from(Span::styled(l, Style::new().fg(TEXT))))
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(inner.x, inner.y, inner.width, visible),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    let mut foot = Vec::new();
    let mut x = inner.x;
    if log.approve.is_some() {
        if log.scroll >= log.max {
            foot.push(Span::styled(
                " y approve ",
                Style::new()
                    .fg(theme::on(GREEN))
                    .bg(GREEN)
                    .add_modifier(Modifier::BOLD),
            ));
            foot.push(Span::raw("  "));
            app.hits.push((Rect::new(x, y, 11, 1), Hit::Key('y')));
            x += 13;
        } else {
            foot.push(Span::styled("↓", bold(MUTED)));
            foot.push(Span::styled(
                " read to the end to approve  ",
                Style::new().fg(FAINT),
            ));
            x += 30;
        }
    }
    foot.push(Span::styled("Esc", bold(MUTED)));
    foot.push(Span::styled(" close  ", Style::new().fg(FAINT)));
    app.hits.push((Rect::new(x, y, 3, 1), Hit::Key('\u{1b}')));
    if log.file.is_some() {
        app.hits.push((Rect::new(x + 9, y, 13, 1), Hit::Key('f')));
        foot.push(Span::styled("f", bold(MUTED)));
        foot.push(Span::styled(" open file  ", Style::new().fg(FAINT)));
    }
    if log.max > 0 {
        foot.push(Span::styled("↑ ↓", bold(MUTED)));
        foot.push(Span::styled(
            format!(
                " scroll · lines {}–{} of {}",
                log.scroll + 1,
                (log.scroll + visible).min(total),
                total
            ),
            Style::new().fg(FAINT),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(foot)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    app.mode = Mode::Log(log);
}

/// Splits a line into rows of at most `width` columns and keeps every
/// character (unlike `wrap`, which joins words): it breaks after the last
/// space that fits, or inside a word that is longer than a row.
pub fn hard_wrap(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let width = width.max(1);
    let mut rest: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    loop {
        let (mut n, mut used) = (0, 0);
        while n < rest.len() {
            let w = rest[n].width().unwrap_or(0);
            if used + w > width {
                break;
            }
            used += w;
            n += 1;
        }
        if n == rest.len() {
            out.push(rest.into_iter().collect());
            return out;
        }
        // A character wider than the row still takes one.
        let n = n.max(1);
        let cut = rest[..n]
            .iter()
            .rposition(|c| *c == ' ')
            .filter(|&i| i > 0)
            .map_or(n, |i| i + 1);
        out.push(rest.drain(..cut).collect());
    }
}

/// Word wrap for plain text.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.width() + 1 + word.width() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        // A word longer than the line (a path) is split, never cut off.
        for c in word.chars() {
            if line.width() + c.to_string().width() > width {
                out.push(std::mem::take(&mut line));
            }
            line.push(c);
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn draw_help(frame: &mut Frame, app: &mut App) {
    let groups: [(&str, &[(&str, &str)]); 5] = [
        (
            "Routines",
            &[
                ("⏎  e", "edit the routine"),
                (
                    "n",
                    "new: Herdr routine, classic cron job, or describe it to Claude",
                ),
                ("p", "write the prompt or command"),
                ("r", "run now (asks first)"),
                ("a", "approve what changed outside the list (asks first)"),
                ("Space", "on / off"),
                ("c", "copy it (the copy starts switched off)"),
                ("o", "go to the tab of its last run"),
                ("l", "result and output of its last run"),
                ("h", "its runs (or click \"n runs\")"),
("d", "delete (asks: keep its runs, or delete them too)"),
            ],
        ),
        (
            "Runs",
            &[
                ("⏎", "go to the run's tab, or its output once closed"),
                ("l", "result and output of the run"),
                ("f", "open the output file"),
                ("h", "only this routine's runs; Esc shows all"),
                ("d", "delete the run (asks first)"),
            ],
        ),
        (
            "Form",
            &[
                ("↑ ↓  Tab", "field"),
                ("⏎  Space", "choose from the list, edit the prompt, save"),
                ("← →", "next / previous choice"),
                ("Ctrl+S", "save the routine"),
                ("Ctrl+U", "clear the field"),
                ("Esc", "back (asks when something changed)"),
            ],
        ),
        (
            "Prompt editor",
            &[
                ("Esc", "done, keeps what you wrote"),
                ("Ctrl+Z  Ctrl+Y", "undo / redo"),
                ("Shift+arrows", "mark text; or drag with the mouse"),
                ("Double click", "mark a word"),
                ("Ctrl+A", "mark everything"),
                ("Ctrl+C  X  V", "copy, cut, paste"),
                ("arrows  Mouse", "move; ⏎ starts a new line"),
                ("Ctrl+O", "open in another editor ($VISUAL, $EDITOR)"),
            ],
        ),
        (
            "List",
            &[
                ("↑ ↓  j k", "move"),
                ("← →  Tab  1 2 3", "routines / cron jobs / runs"),
                ("Mouse", "click to choose, double click to open"),
                (
                    "S",
                    "all routines and cron jobs on or off (or click \"routines\" at the top)",
                ),
                (
                    "A",
                    "autostart: Herdr starts at login (routines run after a restart), or not",
                ),
                (
                    "C",
                    "your crontab's jobs (◇ under Cron jobs): keep as cron job, move to Herdr or copy",
                ),
                ("i", "Nerd Font icons / plain symbols"),
                ("Esc", "close a dialog (never the list)"),
                ("q", "close the list (asks first)"),
            ],
        ),
    ];
    let mut lines: Vec<Line> = Vec::new();
    for (gi, (title, keys)) in groups.iter().enumerate() {
        if gi > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(*title, bold(ACCENT))));
        for (k, what) in keys.iter() {
            lines.push(Line::from(vec![
                Span::styled(format!("{k:<18}"), bold(TEXT)),
                Span::styled(*what, Style::new().fg(MUTED)),
            ]));
        }
    }
    let how = |width: usize| -> Vec<Line<'static>> {
        let mut out = vec![Line::from(Span::styled("How it works", bold(ACCENT)))];
        for (i, p) in INTRO.iter().enumerate() {
            if i > 0 {
                out.push(Line::from(""));
            }
            for l in wrap(p, width) {
                out.push(Line::from(Span::styled(l, Style::new().fg(MUTED))));
            }
        }
        out
    };
    let screen = frame.area();
    let width = 78u16.min(screen.width.saturating_sub(2)).max(24);
    let one_column = how(width.saturating_sub(6) as usize);
    let two_columns =
        screen.width >= 124 && (screen.height as usize) < lines.len() + 1 + one_column.len() + 3;
    let (area, columns): (Rect, Vec<Vec<Line<'static>>>) = if two_columns {
        let right = how(52);
        let height = lines.len().max(right.len()) as u16 + 3;
        (centered(screen, 124, height), vec![lines, right])
    } else {
        lines.push(Line::from(""));
        lines.extend(one_column);
        let height = lines.len() as u16 + 3;
        (centered(screen, width, height), vec![lines])
    };
    let inner = dialog(frame, area, "Keys");
    app.hits.push((area, Hit::Key('?')));
    // The version in the bottom border, linked to the repository.
    if area.height >= 3 {
        let version = env!("CARGO_PKG_VERSION");
        let address = env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://");
        let room = area.width.saturating_sub(4) as usize;
        let shown = [
            format!("herdr-routines {version} · {address}"),
            format!("herdr-routines {version}"),
            version.to_string(),
        ]
        .into_iter()
        .map(|t| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    t,
                    Style::new().fg(ACCENT).add_modifier(Modifier::UNDERLINED),
                ),
                Span::raw(" "),
            ])
        })
        .find(|l| l.width() <= room);
        if let Some(shown) = shown {
            let width = shown.width() as u16;
            let y = area.bottom() - 1;
            frame.render_widget(shown, Rect::new(area.x + 2, y, width, 1));
            app.hits
                .push((Rect::new(area.x + 3, y, width - 2, 1), Hit::Link));
        }
    }
    let longest = columns.iter().map(Vec::len).max().unwrap_or(0) as u16;
    let overflow = longest > inner.height;
    let visible = if overflow {
        inner.height.saturating_sub(1)
    } else {
        inner.height
    };
    app.help_max = longest.saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(app.help_max);
    let body = Rect::new(inner.x, inner.y, inner.width, visible);
    let areas: Vec<Rect> = if columns.len() == 2 {
        let [l, _, r] = Layout::horizontal([
            Constraint::Length(64),
            Constraint::Length(2),
            Constraint::Fill(1),
        ])
        .areas(body);
        vec![l, r]
    } else {
        vec![body]
    };
    for (col, a) in columns.into_iter().zip(areas) {
        frame.render_widget(
            Paragraph::new(Text::from(col)).scroll((app.help_scroll, 0)),
            a,
        );
    }
    if overflow {
        let hint = if app.help_scroll >= app.help_max {
            "↑ ↓ scroll · end".to_string()
        } else {
            format!("↑ ↓ scroll · {} more lines", app.help_max - app.help_scroll)
        };
        frame.render_widget(
            Paragraph::new(Span::styled(hint, Style::new().fg(FAINT))),
            Rect::new(inner.x, inner.y + visible, inner.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_and_wrap() {
        assert_eq!(offset(0, 10, 3), 0);
        assert_eq!(offset(5, 10, 3), 3);
        assert_eq!(offset(9, 10, 3), 7);
        assert_eq!(wrap("a bb ccc dddd", 10), vec!["a bb ccc", "dddd"]);
    }

    #[test]
    fn hard_wrap_keeps_every_character() {
        assert_eq!(hard_wrap("", 5), vec![""]);
        assert_eq!(hard_wrap("ab  cd ef", 5), vec!["ab  ", "cd ef"]);
        assert_eq!(hard_wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(hard_wrap("  rm -rf x", 6), vec!["  rm ", "-rf x"]);
        let long = "x".repeat(50) + " hidden";
        assert_eq!(hard_wrap(&long, 20).concat(), long);
    }

    #[test]
    fn columns_drop_when_narrow() {
        let cols = vec![
            (1, "", Constraint::Length(2)),
            (2, "", Constraint::Fill(1)),
            (3, "", Constraint::Length(26)),
            (4, "", Constraint::Length(24)),
        ];
        assert_eq!(fit(cols.clone(), &[4, 3], 120, 24).len(), 4);
        assert_eq!(fit(cols, &[4, 3], 50, 24).len(), 2);
    }
}
