//! Drawing: the routines and runs tables, the form, the prompt editor,
//! dialogs and a footer, in the look of the other Herdr plugins of this
//! author. Records clickable areas for the mouse.

use crate::agent;
use crate::app::{App, Hit, LogView, Mode, View, INTRO};
use crate::cron::{short_time, Cron};
use crate::form::{Field, Form};
use crate::routine::Routine;
use crate::store::Run;
use crate::theme::{
    self, Icons, ACCENT, BAR_BG, BORDER, FAINT, GREEN, MUTED, RED, SELECTED_BG, TEXT, YELLOW,
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
            View::Routines => draw_routines(frame, app, &icons, body),
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
        let title = format!("{} Agent routines   ", icons.title());
        let mut x = line.x + title.width() as u16;
        spans.push(Span::styled(title, bold(ACCENT)));
        for view in View::ORDER {
            let count = match view {
                View::Routines => app.routines.len(),
                View::Runs => app.runs.len(),
            };
            let label = format!(" {} ({count}) ", view.name());
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

    let (chip, style) = match app.scheduler {
        Some(_) => (
            format!("{} scheduler on", icons.running()),
            Style::new().fg(FAINT),
        ),
        None => (
            format!("{} scheduler off · S starts it", icons.warning()),
            Style::new().fg(RED),
        ),
    };
    let w = chip.width();
    if used + w + 3 <= line.width as usize {
        if app.scheduler.is_none() && in_list {
            let x = line.x + line.width - w as u16;
            app.hits
                .push((Rect::new(x, line.y, w as u16, 1), Hit::Key('S')));
        }
        frame.render_widget(
            Paragraph::new(Span::styled(chip, style)).alignment(Alignment::Right),
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
    let row_h = 3u16;
    let visible = (area.height.saturating_sub(1) / row_h).max(1) as usize;
    let first = offset(selected, rows.len(), visible);
    let mut table_rows = Vec::new();
    for (i, cells) in rows.into_iter().enumerate().skip(first).take(visible) {
        let mut row = Row::new(cells.into_iter().map(Cell::from))
            .height(2)
            .bottom_margin(1);
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
    let f = &r.file;
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
    if app.routines.is_empty() {
        let path = crate::paths::shorten(&app.paths.routines_dir());
        empty_note(
            frame,
            area,
            &[
                ("No routines yet.", TEXT),
                ("", TEXT),
                (
                    "n creates one: what the agent should do, which agent, in which folder and when.",
                    MUTED,
                ),
                (&format!("Routines are plain files in {path}."), FAINT),
            ],
        );
        return;
    }
    let cols = fit(
        vec![
            (RCol::Mark, "", Constraint::Length(2)),
            (RCol::Routine, "Routine", Constraint::Fill(1)),
            (RCol::Schedule, "Schedule", Constraint::Length(26)),
            (RCol::Last, "Last run", Constraint::Length(24)),
        ],
        &[RCol::Last, RCol::Schedule],
        area.width.saturating_sub(2),
        24,
    );
    let now = Local::now();
    let routines = app.routines.clone();
    let rows: Vec<Vec<Text<'static>>> = routines
        .iter()
        .map(|r| {
            let on = r.file.enabled && r.problem.is_none();
            cols.iter()
                .map(|(c, _, _)| match c {
                    RCol::Mark => {
                        let (icon, color) = icons.routine(r.file.enabled, r.problem.is_some());
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
                        } else if !r.file.enabled {
                            faint("off")
                        } else {
                            match r.cron().and_then(|c| c.next_after(&now)) {
                                Some(t) => faint(format!("next {}", crate::app::until(&t, &now))),
                                None => faint("never"),
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
                        Some(run) => last_run(icons, run, &now),
                        None => Text::from(vec![faint("–"), faint("no run yet")]),
                    },
                })
                .collect()
        })
        .collect();
    let head = cols.iter().map(|(_, h, _)| *h).collect();
    let widths = cols.iter().map(|(_, _, w)| *w).collect();
    let sel = app.sel_routine;
    draw_table(frame, app, area, head, widths, rows, sel);
}

fn last_run(icons: &Icons, run: &Run, now: &chrono::DateTime<Local>) -> Text<'static> {
    let (icon, color) = icons.status(run.status);
    let when = crate::app::ago(&run.started, now);
    let second = if run.status.active() {
        format!("{when} · for {}", crate::app::took(run))
    } else {
        format!("{when} · took {}", crate::app::took(run))
    };
    Text::from(vec![
        Line::from(Span::styled(
            format!("{icon} {}", run.status.label()),
            Style::new().fg(color),
        )),
        faint(second),
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
                        let open = !run.tab_closed
                            && !run.pane_id.is_empty()
                            && run.status != crate::store::Status::Closed;
                        if open {
                            first.push(Span::styled(
                                format!("  {} tab open", icons.tab_open()),
                                Style::new().fg(ACCENT),
                            ));
                        }
                        let second = if run.message.is_empty() {
                            format!("{} · {}", run.agent, run.trigger.label().to_lowercase())
                        } else {
                            run.message.clone()
                        };
                        Text::from(vec![Line::from(first), faint(second)])
                    }
                    NCol::Started => Text::from(vec![
                        Line::from(Span::styled(
                            short_time(&run.started, &now),
                            Style::new().fg(TEXT),
                        )),
                        faint(run.trigger.label()),
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
            let mut v = vec![text(form.shown(f))];
            let detail = form.detail(f);
            if !detail.is_empty() {
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
                _ => String::new(),
            };
            let mut v = Vec::new();
            if value.is_empty() && !selected {
                let empty = match f {
                    Field::Name => "required",
                    Field::Args => "none",
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
    let hint = wrap(current.hint(), inner.width.saturating_sub(2) as usize);
    // Below the fields: the schedule in words and the current field's hint.
    let below = if inner.height >= 12 {
        2 + hint.len().min(3) as u16
    } else {
        1
    };
    let list_h = inner.height.saturating_sub(below);
    let first = offset(form.field, fields.len(), list_h as usize);
    for (i, f) in fields.iter().enumerate().skip(first).take(list_h as usize) {
        let y = inner.y + (i - first) as u16;
        let row = Rect::new(inner.x, y, inner.width, 1);
        let selected = i == form.field;
        if selected {
            frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
        }
        app.hits.push((row, Hit::Field(i)));
        let label = format!(" {:<w$}", f.label(form.repeat), w = LABEL_W);
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
    let shown = (fields.len() - first).min(list_h as usize) as u16;
    let y = inner.y + shown + if below > 1 { 1 } else { 0 };
    let now = Local::now();
    let mut summary = match form.schedule() {
        Ok(c) => {
            let cron = Cron::parse(&c).expect("checked");
            let next = cron
                .next_after(&now)
                .map(|t| format!(" · next {}", short_time(&t, &now)))
                .unwrap_or_default();
            vec![Span::styled(
                format!("Runs {}{next}", cron.describe().to_lowercase()),
                Style::new().fg(GREEN),
            )]
        }
        Err(e) => vec![Span::styled(format!("Schedule: {e}"), Style::new().fg(RED))],
    };
    if form.schedule().is_ok() {
        if let Err(e) = form.build() {
            summary.push(Span::styled(
                format!(" · not ready: {e}"),
                Style::new().fg(YELLOW),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(summary)),
        Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
    );
    if below > 2 {
        let lines: Vec<Line> = hint
            .into_iter()
            .take(3)
            .map(|l| Line::from(Span::styled(l, Style::new().fg(MUTED))))
            .collect();
        frame.render_widget(
            Paragraph::new(Text::from(lines)),
            Rect::new(inner.x + 1, y + 1, inner.width.saturating_sub(2), below - 2),
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
    let lines: Vec<Line> = rows
        .iter()
        .skip(ed.text.top)
        .take(height)
        .map(|(l, s, e)| {
            let s: String = ed.text.lines[*l][*s..*e]
                .iter()
                .map(|c| {
                    if *c == '\t' {
                        "    ".to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            Line::from(Span::styled(s, Style::new().fg(TEXT)))
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
            ("^S", "save", Some(Hit::Ctrl('s'))),
            ("Esc", "cancel", Some(Hit::Key('\u{1b}'))),
            ("^Z", "undo", Some(Hit::Ctrl('z'))),
            ("^O", "your editor", Some(Hit::Ctrl('o'))),
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
            ("^S", "save", Some(Hit::Ctrl('s'))),
            ("Esc", "back", Some(Hit::Key('\u{1b}'))),
        ]);
        if !f.is_text() {
            v.push(("?", "all keys", Some(Hit::Key('?'))));
        }
        v
    } else {
        match app.view {
            View::Routines => vec![
                ("⏎", "edit", Some(Hit::Key('\n'))),
                ("n", "new", Some(Hit::Key('n'))),
                ("r", "run now", Some(Hit::Key('r'))),
                ("space", "on/off", Some(Hit::Key(' '))),
                ("?", "all keys", Some(Hit::Key('?'))),
                ("q", "close", Some(Hit::Key('q'))),
            ],
            View::Runs => vec![
                ("⏎", "go to its tab", Some(Hit::Key('\n'))),
                ("l", "output", Some(Hit::Key('l'))),
                ("?", "all keys", Some(Hit::Key('?'))),
                ("q", "close", Some(Hit::Key('q'))),
            ],
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
    let two = p.items.iter().any(|i| !i.detail.is_empty());
    let per = if two { 2u16 } else { 1 };
    let want_h = p.items.len() as u16 * per + 4;
    let area = centered(frame.area(), 72, want_h);
    let inner = dialog(frame, area, &p.title);
    let list_h = inner.height.saturating_sub(2);
    let visible = (list_h / per).max(1) as usize;
    let first = offset(p.sel, p.items.len(), visible);
    for (i, item) in p.items.iter().enumerate().skip(first).take(visible) {
        let y = inner.y + ((i - first) as u16) * per;
        let r = Rect::new(inner.x, y, inner.width, per.min(inner.y + list_h - y));
        let style = if i == p.sel {
            Style::new().bg(SELECTED_BG)
        } else {
            Style::new()
        };
        let number = if i < 9 {
            format!("{} ", i + 1)
        } else {
            "  ".into()
        };
        let mut lines = vec![Line::from(vec![
            Span::styled(number, Style::new().fg(FAINT)),
            Span::styled(item.label.clone(), bold(TEXT)),
        ])];
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
        .flat_map(|l| wrap(l, width.saturating_sub(6) as usize))
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " y yes ",
                Style::new()
                    .fg(theme::on(GREEN))
                    .bg(GREEN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(" n no ", Style::new().fg(TEXT).bg(BORDER)),
        ])),
        Rect::new(inner.x, y, inner.width, 1),
    );
    app.hits.push((Rect::new(inner.x, y, 7, 1), Hit::Key('y')));
    app.hits
        .push((Rect::new(inner.x + 8, y, 6, 1), Hit::Key('n')));
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

fn draw_log(frame: &mut Frame, app: &mut App, mut log: LogView) {
    let screen = frame.area();
    let area = centered(
        screen,
        screen.width.saturating_sub(4).min(140),
        screen.height.saturating_sub(2),
    );
    let inner = dialog(frame, area, &log.title);
    let visible = inner.height.saturating_sub(1);
    log.max = (log.lines.len() as u16).saturating_sub(visible);
    log.scroll = log.scroll.min(log.max);
    let lines: Vec<Line> = log
        .lines
        .iter()
        .skip(log.scroll as usize)
        .take(visible as usize)
        .map(|l| Line::from(Span::styled(l.replace('\t', "    "), Style::new().fg(TEXT))))
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(inner.x, inner.y, inner.width, visible),
    );
    let mut foot = vec![
        Span::styled("Esc", bold(MUTED)),
        Span::styled(" close  ", Style::new().fg(FAINT)),
    ];
    if log.max > 0 {
        foot.push(Span::styled("↑ ↓", bold(MUTED)));
        foot.push(Span::styled(
            format!(
                " scroll · lines {}–{} of {}",
                log.scroll + 1,
                (log.scroll + visible).min(log.lines.len() as u16),
                log.lines.len()
            ),
            Style::new().fg(FAINT),
        ));
    }
    let y = inner.y + inner.height.saturating_sub(1);
    app.hits
        .push((Rect::new(inner.x, y, 3, 1), Hit::Key('\u{1b}')));
    frame.render_widget(
        Paragraph::new(Line::from(foot)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    app.mode = Mode::Log(log);
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
                ("n", "new routine"),
                ("p", "write the prompt"),
                ("r", "run now (asks first)"),
                ("Space", "on / off"),
                ("o", "go to the tab of its last run"),
                ("l", "what its last run wrote"),
                ("d", "delete (asks first)"),
            ],
        ),
        (
            "Runs",
            &[
                ("⏎", "go to the run's tab, or what it wrote once closed"),
                ("l", "what the run wrote"),
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
                ("arrows  Home End", "move; ⏎ starts a new line"),
                ("Ctrl+S", "keep the prompt"),
                ("Esc", "cancel (asks when something changed)"),
                ("Ctrl+Z  Ctrl+Y", "undo / redo"),
                ("Ctrl+O", "open in your own editor ($VISUAL, $EDITOR)"),
            ],
        ),
        (
            "List",
            &[
                ("↑ ↓  j k", "move"),
                ("← →  Tab  1 2", "routines / runs"),
                ("Mouse", "click to choose, double click to open"),
                ("S", "start the scheduler"),
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
