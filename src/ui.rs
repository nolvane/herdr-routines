//! Drawing: the routines and runs lists, the form, the prompt editor,
//! dialogs and a footer, in the look of the other Herdr plugins of this
//! author. Records clickable areas for the mouse.
//!
//! How it is put together:
//! - `draw` lays out header, body and footer. While a dialog is open it
//!   drops every clickable area of the header and body, so only the dialog
//!   and its keys in the footer can be clicked.
//! - The header is a title, the tabs and `chips` (a list of `Chip`s); it
//!   gets shorter in a fixed order when the window is narrow (`plans`).
//! - Lists go through `draw_list`: a scroll offset per list (`App::tops`),
//!   a position such as "1–10 of 23", the marked row and the row under the
//!   mouse. Every cell is cut with "…" (`clip_line`).
//! - The footer shows the keys of what is in front (`footer_keys`), each
//!   with a rank: the least important go first when it gets narrow. A
//!   message takes the room it needs, up to the whole footer.

use crate::agent;
use crate::app::{
    App, Attention, EditTarget, Hit, LogView, Mode, RunFilter, RunPage, Sort, View, INTRO, TAKES,
};
use crate::cron::short_time;
use crate::form::{Field, Form};
use crate::routine::Routine;
use crate::schedule::Schedule;
use crate::store::{Run, Status};
use crate::theme::{
    self, Icons, Mark, ACCENT, BAR_BG, BLUE, BORDER, FAINT, GREEN, HOVER_BG, MARK_BG, MUTED, RED,
    SELECTED_BG, TEXT, YELLOW,
};
use chrono::{DateTime, Local};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.hits.clear();
    app.dialog = None;
    let screen = frame.area();
    if app.size != (screen.width, screen.height) {
        // A window of another size: bring the cursor back into view.
        app.size = (screen.width, screen.height);
        app.follow = true;
    }
    let spacer = if screen.height >= 14 { 1 } else { 0 };
    let foot = footer_height(app, screen.width)
        .min(screen.height.saturating_sub(4))
        .max(1);
    let [header, _, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(spacer),
        Constraint::Min(3),
        Constraint::Length(foot),
    ])
    .areas(screen);
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
    // A dialog owns the mouse: nothing of the header or the list under it
    // can be clicked, only the dialog itself and its keys in the footer.
    if !matches!(app.mode, Mode::Normal) {
        app.hits.clear();
    }
    // Dialogs stay between the header and the footer.
    let stage = Rect::new(
        screen.x,
        header.bottom(),
        screen.width,
        footer.y.saturating_sub(header.bottom()),
    );
    match app.mode.clone() {
        Mode::Help => draw_help(frame, app, stage),
        Mode::RunPage(p) => draw_run_page(frame, app, &icons, stage, &p),
        Mode::Pick(p) => draw_picker(frame, app, stage, p),
        Mode::Input(i) => draw_input(frame, app, stage, &i),
        Mode::Confirm(c) => draw_confirm(frame, app, stage, &c),
        Mode::Message { title, lines } => draw_message(frame, app, stage, &title, &lines),
        Mode::Log(l) => draw_log(frame, app, stage, l),
        Mode::Cron(jobs) => draw_take(frame, app, stage, &jobs),
        Mode::Menu(m) => draw_menu(frame, app, stage, &m),
        Mode::Normal | Mode::AskQuit => {}
    }
    draw_footer(frame, app, footer);
}

fn bold(color: Color) -> Style {
    Style::new().fg(color).add_modifier(Modifier::BOLD)
}

fn plain(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color))
}

fn faint(t: impl Into<String>) -> Line<'static> {
    Line::from(plain(t, FAINT))
}

// ---- header ----------------------------------------------------------------

/// A switch in the header, such as "routines on". To add one, push it in
/// `chips`: the header lays the list out by itself.
pub struct Chip {
    /// With its state, such as "● autostart on".
    pub long: String,
    /// What shows when the header is narrow. It still says what matters:
    /// a switch whose off state is a warning keeps "off" in it.
    pub short: String,
    pub style: Style,
    /// The key a click stands for.
    pub key: Option<char>,
    /// It is a warning: it stays when the other chips go.
    pub keep: bool,
}

/// The header's chips, left to right. When room runs out, those without
/// `keep` go first, from the left.
pub fn chips(app: &App, icons: &Icons) -> Vec<Chip> {
    let mut v = Vec::new();
    // When the computer is kept awake (W chooses).
    if crate::awake::available() {
        let on = app.awake != crate::awake::Mode::Off;
        let sign = if on { icons.running() } else { icons.stopped() };
        v.push(Chip {
            long: format!("{sign} {}", app.awake.label()),
            short: format!("{sign} awake"),
            style: Style::new().fg(if on { GREEN } else { MUTED }),
            key: Some('W'),
            keep: false,
        });
    }
    // Whether Herdr starts at login (A switches it).
    if crate::autostart::supported() {
        let on = crate::autostart::is_on();
        let sign = if on { icons.running() } else { icons.stopped() };
        v.push(Chip {
            long: format!("{sign} autostart {}", if on { "on" } else { "off" }),
            short: format!("{sign} autostart"),
            style: Style::new().fg(if on { GREEN } else { MUTED }),
            key: Some('A'),
            keep: false,
        });
    }
    // Whether routines run at all (S switches them). Off is a warning:
    // nothing runs, so it always shows.
    let on = app.scheduler.is_some();
    v.push(if on {
        Chip {
            long: format!("{} routines on", icons.running()),
            short: format!("{} routines", icons.running()),
            style: Style::new().fg(GREEN),
            key: Some('S'),
            keep: false,
        }
    } else {
        Chip {
            long: format!("{} routines off", icons.stopped()),
            short: format!("{} off", icons.stopped()),
            style: bold(RED),
            key: Some('S'),
            keep: true,
        }
    });
    v
}

/// One way to fit the header: how much of the title (2 whole, 1 its icon,
/// 0 none), how short the tabs (0 with counts, 1 without, 2 short names)
/// and which chips, each long or short.
struct Plan {
    title: u8,
    tabs: u8,
    chips: Vec<(usize, bool)>,
}

/// The ways to fit the header, from everything to the least: the title
/// goes first, then the chips that are no warning get short and go (from
/// the left), then the tabs get shorter, and at last the warning gets
/// short. A tab is never cut and a warning never goes.
fn plans(all: &[Chip]) -> Vec<Plan> {
    // A warning keeps its long label as long as anything else can give.
    let every = |short: bool| -> Vec<(usize, bool)> {
        (0..all.len()).map(|i| (i, short && !all[i].keep)).collect()
    };
    let kept = |short: bool| -> Vec<(usize, bool)> {
        (0..all.len())
            .filter(|i| all[*i].keep)
            .map(|i| (i, short))
            .collect()
    };
    let plan = |title: u8, tabs: u8, chips: Vec<(usize, bool)>| Plan { title, tabs, chips };
    let mut v = vec![
        plan(2, 0, every(false)),
        plan(1, 0, every(false)),
        plan(1, 0, every(true)),
    ];
    let mut rest = every(true);
    while let Some(at) = rest.iter().position(|(i, _)| !all[*i].keep) {
        rest.remove(at);
        v.push(plan(1, 0, rest.clone()));
    }
    for (title, tabs) in [(1, 1), (1, 2), (0, 2)] {
        v.push(plan(title, tabs, kept(false)));
    }
    v.push(plan(0, 2, kept(true)));
    v
}

fn tab_label(view: View, count: usize, level: u8) -> String {
    match (level, view) {
        (0, _) => format!(" {} ({count}) ", view.name()),
        (2, View::Cron) => " Cron ".to_string(),
        _ => format!(" {} ", view.name()),
    }
}

fn draw_header(frame: &mut Frame, app: &mut App, icons: &Icons, area: Rect) {
    let line = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    let room = line.width as usize;
    let all = chips(app, icons);
    let in_list = app.editor.is_none() && app.form.is_none();
    if !in_list {
        // The form and the editor: what is edited, and the warning chips.
        let title = if let Some(ed) = &app.editor {
            ed.title.clone()
        } else {
            match app.form.as_ref().map(|f| (f.id.is_some(), f.name.trim())) {
                Some((true, name)) => format!("Edit {name}"),
                Some((false, _)) if app.form.as_ref().is_some_and(|f| f.is_cron()) => {
                    "New cron job".to_string()
                }
                _ => "New routine".to_string(),
            }
        };
        // Long where the title still has room beside it, else short.
        let chip = all.iter().rfind(|c| c.keep).and_then(|c| {
            [&c.long, &c.short]
                .into_iter()
                .find(|text| room >= text.width() + 2 + 12)
                .map(|text| Span::styled(text.clone(), c.style))
        });
        let chip_w = chip.as_ref().map_or(0, |c| c.width() + 2);
        let title_room = room.saturating_sub(if chip.is_some() { chip_w } else { 0 });
        frame.render_widget(
            Paragraph::new(Span::styled(
                clip(&format!("{} {title}", icons.title()), title_room),
                bold(ACCENT),
            )),
            line,
        );
        if let Some(chip) = chip {
            frame.render_widget(
                Paragraph::new(Line::from(chip)).alignment(Alignment::Right),
                line,
            );
        }
        return;
    }
    let count = |view: View| match view {
        View::Routines => app.routines.iter().filter(|r| !r.is_cron()).count(),
        View::Cron => app.routines.iter().filter(|r| r.is_cron()).count() + app.crontab_jobs.len(),
        // All runs: a filter only narrows the list in the view.
        View::Runs => app.all_runs.len(),
    };
    let title = |level: u8| match level {
        2 => format!("{} Scheduled routines   ", icons.title()),
        1 => format!("{}  ", icons.title()),
        _ => String::new(),
    };
    let chips_width = |picked: &[(usize, bool)]| -> usize {
        let texts: usize = picked
            .iter()
            .map(|(i, short)| {
                if *short {
                    all[*i].short.width()
                } else {
                    all[*i].long.width()
                }
            })
            .sum();
        texts + 3 * picked.len().saturating_sub(1)
    };
    let width = |p: &Plan| -> usize {
        let tabs: usize = View::ORDER
            .iter()
            .map(|v| tab_label(*v, count(*v), p.tabs).width() + 1)
            .sum::<usize>()
            - 1;
        let chips = chips_width(&p.chips);
        title(p.title).width() + tabs + if chips > 0 { chips + 2 } else { 0 }
    };
    let plans = plans(&all);
    let plan = plans
        .iter()
        .find(|p| width(p) <= room)
        .unwrap_or_else(|| plans.last().expect("there is always a plan"));

    let mut spans = Vec::new();
    let head = title(plan.title);
    let mut x = line.x + head.width() as u16;
    spans.push(Span::styled(head, bold(ACCENT)));
    for view in View::ORDER {
        let label = tab_label(view, count(view), plan.tabs);
        let style = if app.view == view {
            Style::new()
                .fg(TEXT)
                .bg(SELECTED_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(MUTED)
        };
        let w = label.width() as u16;
        if x + w <= line.right() {
            app.hits.push((Rect::new(x, line.y, w, 1), Hit::View(view)));
        }
        x += w + 1;
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), line);

    // The chips sit at the right edge.
    let mut x = line.right().saturating_sub(chips_width(&plan.chips) as u16);
    let mut spans = Vec::new();
    for (n, (i, short)) in plan.chips.iter().enumerate() {
        let chip = &all[*i];
        let text = if *short { &chip.short } else { &chip.long };
        if n > 0 {
            spans.push(Span::raw("   "));
            x += 3;
        }
        let w = text.width() as u16;
        if let Some(key) = chip.key {
            app.hits.push((Rect::new(x, line.y, w, 1), Hit::Key(key)));
        }
        x += w;
        spans.push(Span::styled(text.clone(), chip.style));
    }
    if !spans.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
            line,
        );
    }
}

// ---- lists -------------------------------------------------------------------

/// Rows per routine or run in the lists: name and details, no gap.
const ROW_H: u16 = 2;
/// The least a list's main column gets before other columns go.
const NAME_MIN: u16 = 24;

/// Room between columns: wider on wide panes.
fn spacing(width: u16) -> u16 {
    if width >= 80 {
        3
    } else {
        1
    }
}

/// `s` cut to `width` columns, with an ellipsis when cut.
fn clip(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + c.width().unwrap_or(0) + 1 > width {
            break;
        }
        out.push(c);
    }
    if width > 0 {
        out.push('…');
    }
    out
}

/// `s` cut at its start to `width` columns, with an ellipsis in front:
/// for folders, whose end says the most.
fn clip_start(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 1;
    for c in s.chars().rev() {
        used += c.width().unwrap_or(0);
        if used > width {
            break;
        }
        tail.push(c);
    }
    let mut out = String::from("…");
    out.extend(tail.into_iter().rev());
    out
}

/// A line of several colours cut to `width` columns, with an ellipsis in
/// the colour of the part that was cut. Nothing in a list or a dialog ends
/// without one when it does not fit.
fn clip_line(line: Line<'static>, width: usize) -> Line<'static> {
    if line.width() <= width {
        return line;
    }
    let room = width.saturating_sub(1);
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    let mut cut_style = Style::new();
    for span in line.spans {
        cut_style = span.style;
        let w = span.content.width();
        if used + w <= room {
            used += w;
            out.push(span);
            continue;
        }
        let mut text = String::new();
        for c in span.content.chars() {
            let cw = c.width().unwrap_or(0);
            if used + cw > room {
                break;
            }
            used += cw;
            text.push(c);
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span.style));
        }
        break;
    }
    if width > 0 {
        out.push(Span::styled("…", cut_style));
    }
    Line::from(out)
}

/// Faint text reads badly on the marked row's background: there it gets
/// the next brighter grey.
fn relight(mut line: Line<'static>) -> Line<'static> {
    for span in &mut line.spans {
        if span.style.fg == Some(FAINT) {
            span.style.fg = Some(MUTED);
        }
    }
    line
}

fn empty_note(frame: &mut Frame, area: Rect, lines: &[(String, Color)]) {
    let width = area.width.saturating_sub(4) as usize;
    let mut text: Vec<Line> = Vec::new();
    for (t, c) in lines {
        if t.is_empty() {
            text.push(Line::from(""));
        }
        for l in wrap(t, width) {
            text.push(Line::from(plain(l, *c)));
        }
    }
    frame.render_widget(
        Paragraph::new(text),
        Rect::new(area.x + 2, area.y, width as u16, area.height),
    );
}

/// A column of a list: its heading and its width.
struct Col {
    head: &'static str,
    width: u16,
}

/// One row of a list: two lines per column.
type Cells = Vec<[Line<'static>; 2]>;

/// Which list to draw (its index in `App::tops`), its columns and the gap
/// between them.
struct Table<'a> {
    list: usize,
    gap: u16,
    cols: &'a [Col],
}

/// Where a list was drawn: the left edge of each column, and each visible
/// row with its top line.
struct Drawn {
    xs: Vec<u16>,
    rows: Vec<(usize, u16)>,
}

/// Draws a list of two-line rows under a bar with the headings and
/// records each visible row as clickable.
///
/// The list has its own scroll offset (`App::tops[list]`): the wheel moves
/// it, a click never does, and the cursor moves it only when a key took it
/// out of view (`App::follow`). When not every row fits, the bar says
/// which ones show, such as "1–10 of 23".
fn draw_list(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    table: Table,
    rows: Vec<Cells>,
    selected: usize,
) -> Drawn {
    let Table { list, gap, cols } = table;
    let x0 = area.x + 1;
    let width = area.width.saturating_sub(2);
    let mut xs = Vec::new();
    let mut x = x0;
    for col in cols {
        xs.push(x);
        x += col.width + gap;
    }
    let visible = (area.height.saturating_sub(1) / ROW_H).max(1) as usize;
    let n = rows.len();
    let mut top = app.tops[list].min(n.saturating_sub(visible));
    if app.follow {
        if selected < top {
            top = selected;
        } else if selected >= top + visible {
            top = selected + 1 - visible;
        }
        app.follow = false;
    }
    app.tops[list] = top;

    // The headings on a bar of their own.
    let bar = Rect::new(x0, area.y, width, 1);
    frame.render_widget(Block::new().style(Style::new().bg(BAR_BG)), bar);
    let position = (n > visible).then(|| format!("{}–{} of {n} ", top + 1, (top + visible).min(n)));
    let position_w = position.as_ref().map_or(0, |p| p.width() as u16 + 1);
    for (col, x) in cols.iter().zip(&xs) {
        // A heading gives way to the position when both do not fit.
        let end = x + col.head.width() as u16;
        if col.width == 0 || end > bar.right().saturating_sub(position_w) {
            continue;
        }
        frame.render_widget(
            Paragraph::new(Span::styled(col.head, bold(MUTED))),
            Rect::new(*x, area.y, col.width.min(bar.right().saturating_sub(*x)), 1),
        );
    }
    if let Some(position) = position {
        frame.render_widget(
            Paragraph::new(Span::styled(position, Style::new().fg(MUTED)))
                .alignment(Alignment::Right),
            bar,
        );
    }

    let mut drawn = Vec::new();
    let hover = matches!(app.mode, Mode::Normal)
        .then_some(app.mouse)
        .flatten();
    for (i, cells) in rows.into_iter().enumerate().skip(top).take(visible) {
        let y = area.y + 1 + ((i - top) as u16) * ROW_H;
        if y + ROW_H > area.bottom() {
            break;
        }
        let row = Rect::new(area.x, y, area.width, ROW_H);
        let under_mouse = hover.is_some_and(|(mx, my)| {
            mx >= row.x && mx < row.right() && my >= row.y && my < row.bottom()
        });
        let lit = i == selected || under_mouse;
        if lit {
            let bg = if i == selected { SELECTED_BG } else { HOVER_BG };
            frame.render_widget(
                Block::new().style(Style::new().bg(bg)),
                Rect::new(x0, y, width, ROW_H),
            );
        }
        for ((col, x), [first, second]) in cols.iter().zip(&xs).zip(cells) {
            let w = col.width.min(bar.right().saturating_sub(*x));
            if w == 0 {
                continue;
            }
            let mut lines = vec![clip_line(first, w as usize), clip_line(second, w as usize)];
            if lit {
                lines = lines.into_iter().map(relight).collect();
            }
            frame.render_widget(
                Paragraph::new(Text::from(lines)),
                Rect::new(*x, y, w, ROW_H),
            );
        }
        app.hits.push((row, Hit::Row(i)));
        drawn.push((i, y));
    }
    Drawn { xs, rows: drawn }
}

/// A routine's (or cron job's) row, in pieces the list puts together by
/// how wide it is.
struct RoutineRow {
    mark: Span<'static>,
    name: Line<'static>,
    /// What it runs with and where, or what keeps it from running.
    about: Line<'static>,
    /// `about` says what keeps it from running: nothing goes before it.
    blocked: bool,
    /// The schedule in words, and the next time.
    when: Line<'static>,
    next: Line<'static>,
    /// The last run, for a column of its own: status and the link to its
    /// runs, then when and how long.
    last: [Line<'static>; 2],
    /// The link's place in the first of these lines: offset and width.
    link: Option<(u16, u16)>,
    /// The last run in a few words, for windows without that column.
    brief: Option<Line<'static>>,
}

/// What a routine runs with and where, as the list and the details say
/// it, always in this order: agent, model, permission mode, effort.
fn runs_with(r: &Routine) -> String {
    let f = &r.file;
    if r.is_cron() {
        return "a cron job, from your crontab (also when Herdr is closed)".into();
    }
    if r.is_shell() {
        return "a shell command, in a Herdr tab".into();
    }
    let mut parts = vec![f.agent.clone()];
    parts.push(if f.model.is_empty() {
        "default model".to_string()
    } else {
        agent::label(agent::models(&f.agent), &f.model)
    });
    parts.push(if f.permission_mode.is_empty() {
        "agent's own permissions".to_string()
    } else {
        agent::label(agent::modes(&f.agent), &f.permission_mode)
    });
    if !f.effort.is_empty() {
        parts.push(format!(
            "{} effort",
            agent::label(agent::efforts(&f.agent), &f.effort).to_lowercase()
        ));
    }
    parts.join(" · ")
}

/// The second line under a routine's name: what it runs with and its
/// folder, or what keeps it from running.
fn routine_about(app: &App, r: &Routine) -> (Line<'static>, bool) {
    if let Some(p) = &r.problem {
        return (Line::from(plain(p.clone(), RED)), true);
    }
    if !r.approved {
        let why = if app.known.contains(&r.id) {
            "Changed outside the list"
        } else {
            "New, not approved yet"
        };
        return (
            Line::from(plain(format!("{why} · a shows it for approval"), YELLOW)),
            true,
        );
    }
    let f = &r.file;
    if r.is_shell() {
        let first = r
            .prompt
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        // Cron starts its jobs in the home folder: nothing to show.
        return (
            faint(if r.is_cron() {
                format!("cron job · {first}")
            } else {
                format!("shell command · {} · {}", clip(first, 40), f.folder)
            }),
            false,
        );
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
    // Extra options can change what the agent does: always in sight.
    if !f.args.is_empty() {
        parts.push(clip(&f.args.join(" "), 40));
    }
    parts.push(f.folder.clone());
    (faint(parts.join(" · ")), false)
}

/// When a routine runs next, in the words the list and the details share:
/// "next in 9 min", or why there is no next time.
fn next_words(app: &App, r: &Routine, now: &DateTime<Local>) -> Span<'static> {
    if r.problem.is_some() {
        return plain("–", FAINT);
    }
    if !r.approved {
        return plain("waits for approval", YELLOW);
    }
    if !r.file.enabled {
        return plain("off", MUTED);
    }
    // Its time has come, but as many runs as may go at once are going.
    if app.waiting.contains_key(&r.id) {
        return plain("waits for a free place", YELLOW);
    }
    match r.schedule() {
        Some(Schedule::AtStart) if r.is_cron() => plain("next computer start", FAINT),
        Some(Schedule::AtStart) => plain("next Herdr start", FAINT),
        s => match s.and_then(|c| c.next_after(now)) {
            Some(t) => plain(format!("next {}", crate::app::until(&t, now)), FAINT),
            None => plain("done", FAINT),
        },
    }
}

fn routine_row(app: &App, icons: &Icons, r: &Routine, now: &DateTime<Local>) -> RoutineRow {
    let mark = if r.problem.is_some() {
        Mark::Problem
    } else if !r.approved {
        Mark::Waiting
    } else if r.file.enabled {
        Mark::On
    } else {
        Mark::Off
    };
    let (icon, color) = icons.routine(mark);
    let on = r.file.enabled && r.can_run();
    let (about, blocked) = routine_about(app, r);
    let when = if r.unreadable {
        "–".to_string()
    } else {
        r.schedule_text()
    };
    let (last, link, brief) = match app.latest_run(&r.id) {
        Some(run) => {
            let (icon, color) = icons.status(run.status);
            let status = format!("{icon} {}", run.status.label());
            let count = app.run_count(&r.id);
            let runs = if count == 1 {
                "1 run ›".to_string()
            } else {
                format!("{count} runs ›")
            };
            let ago = crate::app::ago(&run.started, now);
            let link = (status.width() as u16 + 2, runs.width() as u16);
            let (second, short) = if run.status.active() {
                let t = crate::app::took(run);
                (faint(format!("{ago} · for {t}")), format!(" for {t}"))
            } else if let Some(n) = app.failing.get(&r.id) {
                // Not the first that went wrong: that says more than how
                // long it took.
                (
                    Line::from(vec![
                        plain(format!("{ago} · "), FAINT),
                        plain(format!("{n} times in a row"), RED),
                    ]),
                    format!(" {ago} · {n} in a row"),
                )
            } else if run.status == Status::Skipped {
                // Nothing ran at that time.
                (faint(ago.clone()), format!(" {ago}"))
            } else {
                (
                    faint(format!("{ago} · took {}", crate::app::took(run))),
                    format!(" {ago}"),
                )
            };
            (
                [
                    Line::from(vec![
                        plain(status.clone(), color),
                        Span::raw("  "),
                        plain(runs, ACCENT),
                    ]),
                    second,
                ],
                Some(link),
                Some(Line::from(vec![plain(status, color), plain(short, FAINT)])),
            )
        }
        None => ([faint("–"), faint("no run yet")], None, None),
    };
    RoutineRow {
        mark: plain(icon, color),
        name: Line::from(Span::styled(
            r.file.name.clone(),
            bold(if r.file.enabled { TEXT } else { MUTED }),
        )),
        about,
        blocked,
        when: Line::from(plain(when, if on { TEXT } else { MUTED })),
        next: Line::from(next_words(app, r, now)),
        last,
        link,
        brief,
    }
}

/// A job that is only in the person's crontab: cron runs it, not this
/// plugin, until it is taken over.
fn job_row(job: &crate::crontab::CronJob, now: &DateTime<Local>) -> RoutineRow {
    let when = Schedule::parse(&job.schedule).ok();
    let next = match &when {
        Some(Schedule::AtStart) => faint("next computer start"),
        Some(s) => match s.next_after(now) {
            Some(t) => faint(format!("next {}", crate::app::until(&t, now))),
            None => faint("–"),
        },
        None => faint("–"),
    };
    let words = match &when {
        Some(Schedule::AtStart) => "When the computer starts".to_string(),
        Some(s) => s.describe(),
        None => job.schedule.clone(),
    };
    RoutineRow {
        mark: plain("◇", BLUE),
        name: Line::from(Span::styled(
            crate::crontab::name_for(&job.command),
            bold(TEXT),
        )),
        about: faint(job.command.clone()),
        blocked: false,
        when: Line::from(plain(words, TEXT)),
        next,
        last: [
            Line::from(plain("only in your crontab", MUTED)),
            faint("⏎ takes it over"),
        ],
        link: None,
        brief: Some(Line::from(plain("only in your crontab", MUTED))),
    }
}

/// `first`, a dot and `second` on one line.
fn joined(first: Line<'static>, second: Line<'static>) -> Line<'static> {
    let mut spans = first.spans;
    spans.push(plain(" · ", FAINT));
    spans.extend(second.spans);
    Line::from(spans)
}

/// How a list of routines is laid out at a width.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    /// Routine, schedule and last run side by side.
    Full,
    /// Routine and schedule; the last run goes in front of the second line.
    Compact,
    /// The routine alone; last run and next time on its second line.
    Narrow,
}

/// The shape a list of routines takes in `inner` columns, with the gap
/// between columns and the widths of the name and schedule columns.
/// `name`, `sched` and `last` are what the rows would like.
fn routine_shape(inner: u16, name: u16, sched: u16, last: u16) -> (Shape, u16, u16, u16) {
    let floor = sched.min(22);
    let mut gaps = vec![1];
    if inner >= 80 {
        gaps.insert(0, 3);
    }
    for gap in gaps {
        // What is left for the name and the schedule.
        let room = inner.saturating_sub(2 + 3 * gap + last);
        if room >= NAME_MIN + floor {
            let mut sched_w = sched;
            if room.saturating_sub(sched_w) < name {
                // The schedule gives way first, down to its floor.
                sched_w = room.saturating_sub(name).clamp(floor, sched);
            }
            return (Shape::Full, gap, room - sched_w, sched_w);
        }
    }
    let gap = 1;
    let room = inner.saturating_sub(2 + 2 * gap);
    let floor = sched.min(18);
    if room >= NAME_MIN + floor {
        let sched_w = room.saturating_sub(28).clamp(floor, sched.min(30));
        return (Shape::Compact, gap, room - sched_w, sched_w);
    }
    (Shape::Narrow, gap, inner.saturating_sub(2 + gap), 0)
}

/// The selected routine's details show from this list height on when not
/// every row fits, with at least DETAILS_MIN rows for them. When every row
/// fits, they show in whatever is left below, from DETAILS_FREE rows on.
const DETAILS_FROM: u16 = 24;
const DETAILS_MIN: u16 = 11;
const DETAILS_FREE: u16 = 5;

/// The line above the routines: what needs the owner, what is going and
/// what comes next, each part a link. Returns the room below it.
fn draw_attention(frame: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if app.view != View::Routines || area.height < 10 {
        return area;
    }
    let mut parts = app.attention_parts();
    if parts.is_empty() {
        return area;
    }
    let bar = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
    let room = bar.width as usize;
    let width = |parts: &[(Attention, String, Color)]| -> usize {
        1 + parts.iter().map(|p| p.1.width()).sum::<usize>() + 3 * parts.len().saturating_sub(1)
    };
    // What comes next gives way first: without the name, then not at all.
    if width(&parts) > room {
        if let Some(at) = parts.iter().position(|p| p.0 == Attention::Waiting) {
            match app.next_up() {
                Some((_, when)) => parts[at].1 = format!("next {when}"),
                None => {
                    parts.remove(at);
                }
            }
            if width(&parts) > room {
                parts.retain(|p| p.0 != Attention::Waiting);
            }
        }
    }
    while width(&parts) > room && parts.len() > 1 {
        parts.pop();
    }
    let mut spans = vec![Span::raw(" ")];
    let mut x = bar.x + 1;
    for (i, (kind, text, color)) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(plain(" · ", FAINT));
            x += 3;
        }
        let w = text.width() as u16;
        if x + w <= bar.right() {
            app.hits
                .push((Rect::new(x, bar.y, w, 1), Hit::Attention(kind)));
        }
        x += w;
        let style = match kind {
            Attention::NeedsYou | Attention::WentWrong => bold(color),
            _ => Style::new().fg(color),
        };
        spans.push(Span::styled(text, style));
    }
    frame.render_widget(Paragraph::new(clip_line(Line::from(spans), room)), bar);
    Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    )
}

fn draw_routines(frame: &mut Frame, app: &mut App, icons: &Icons, area: Rect) {
    let area = draw_attention(frame, app, area);
    let cron_view = app.view == View::Cron;
    let routines: Vec<Routine> = app.listed().into_iter().cloned().collect();
    let jobs = if cron_view {
        app.crontab_jobs.clone()
    } else {
        Vec::new()
    };
    if routines.is_empty() && jobs.is_empty() {
        let path = crate::paths::shorten(&app.paths.routines_dir());
        let lines: Vec<(String, Color)> = if cron_view {
            vec![
                ("No cron jobs.".into(), TEXT),
                (String::new(), TEXT),
                (
                    "n, then \"Cron job\", creates one: a command that cron runs, also when Herdr is closed.".into(),
                    MUTED,
                ),
                (
                    "Jobs that are already in your crontab show here too.".into(),
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
        empty_note(frame, area, &lines);
        return;
    }
    let now = Local::now();
    let mut rows: Vec<RoutineRow> = routines
        .iter()
        .map(|r| routine_row(app, icons, r, &now))
        .collect();
    rows.extend(jobs.iter().map(|j| job_row(j, &now)));

    // The columns are as wide as what they hold, within limits; the name
    // takes the rest.
    let most = |f: &dyn Fn(&RoutineRow) -> usize| rows.iter().map(f).max().unwrap_or(0) as u16;
    let name = most(&|r| r.name.width()).clamp(NAME_MIN, 34);
    let sched = most(&|r| r.when.width().max(r.next.width())).clamp(8, 46);
    let last = most(&|r| r.last[0].width().max(r.last[1].width())).clamp(8, 30);
    let inner = area.width.saturating_sub(2);
    let (shape, gap, name_w, sched_w) = routine_shape(inner, name, sched, last);
    let title = if cron_view { "Cron job" } else { "Routine" };
    let cols: Vec<Col> = match shape {
        Shape::Full => vec![
            Col { head: "", width: 2 },
            Col {
                head: title,
                width: name_w,
            },
            Col {
                head: "Schedule",
                width: sched_w,
            },
            Col {
                head: "Last run",
                width: last,
            },
        ],
        Shape::Compact => vec![
            Col { head: "", width: 2 },
            Col {
                head: title,
                width: name_w,
            },
            Col {
                head: "Schedule",
                width: sched_w,
            },
        ],
        Shape::Narrow => vec![
            Col { head: "", width: 2 },
            Col {
                head: title,
                width: name_w,
            },
        ],
    };
    let links: Vec<Option<(u16, u16)>> = rows.iter().map(|r| r.link).collect();
    let cells: Vec<Cells> = rows
        .into_iter()
        .map(|r| {
            let mark = [Line::from(r.mark), Line::from("")];
            // Without a column for the last run, it leads the second line.
            let with_brief = |about: Line<'static>| match r.brief.clone() {
                Some(brief) if !r.blocked => joined(brief, about),
                _ => about,
            };
            match shape {
                Shape::Full => vec![mark, [r.name, r.about], [r.when, r.next], r.last],
                Shape::Compact => vec![mark, [r.name, with_brief(r.about)], [r.when, r.next]],
                Shape::Narrow => {
                    let second = if r.blocked {
                        r.about
                    } else {
                        with_brief(r.next)
                    };
                    vec![mark, [r.name, second]]
                }
            }
        })
        .collect();

    let sel = app.list_sel();
    let n = cells.len() as u16;
    // The selected routine's details sit under the list: in what is left
    // when every row fits, else in a tall enough window.
    let table_h = 1 + ROW_H * n;
    let free = area.height.saturating_sub(table_h + 1);
    let list_h = if area.width < 50 {
        area.height
    } else if free >= DETAILS_FREE {
        table_h
    } else if area.height >= DETAILS_FROM {
        1 + ROW_H * ((area.height - DETAILS_MIN - 1) / ROW_H)
    } else {
        area.height
    };
    let details = (list_h < area.height).then(|| {
        Rect::new(
            area.x,
            area.y + list_h + 1,
            area.width,
            area.height - list_h - 1,
        )
    });
    let list = Rect {
        height: list_h,
        ..area
    };
    let index = App::list_index(app.view);
    let drawn = draw_list(
        frame,
        app,
        list,
        Table {
            list: index,
            gap,
            cols: &cols,
        },
        cells,
        sel,
    );
    // Only the "n runs ›" text links to the routine's runs.
    if shape == Shape::Full {
        for (i, y) in &drawn.rows {
            if let Some((offset, width)) = links[*i] {
                if offset + width <= last {
                    app.hits.push((
                        Rect::new(drawn.xs[3] + offset, *y, width, 1),
                        Hit::RunsOf(*i),
                    ));
                }
            }
        }
    }
    if let Some(area) = details {
        if let Some(r) = routines.get(sel) {
            draw_details(frame, app, icons, r, area);
        } else if let Some(job) = app.selected_cron_job().cloned() {
            draw_job_details(frame, &job, area);
        }
    }
}

/// The heading of a details panel: the name and a rule.
fn details_title(name: &str, width: usize) -> Line<'static> {
    let title = format!(" {}", clip(name, width.saturating_sub(4)));
    let rule = width.saturating_sub(title.width() + 2);
    Line::from(vec![
        Span::styled(title, bold(ACCENT)),
        plain(format!(" {}", "─".repeat(rule)), BORDER),
    ])
}

/// The width of the labels in a details panel.
const DETAIL_LABEL: usize = 16;

fn detail_row(label: &str, value: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut spans = vec![plain(format!(" {label:<DETAIL_LABEL$}"), MUTED)];
    spans.extend(value);
    clip_line(Line::from(spans), width)
}

/// A crontab-only job in full, and what ⏎ does with it.
fn draw_job_details(frame: &mut Frame, job: &crate::crontab::CronJob, area: Rect) {
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let w = inner.width as usize;
    let mut lines = vec![details_title(&crate::crontab::name_for(&job.command), w)];
    lines.push(detail_row(
        "Crontab line",
        vec![plain(job.line.trim().to_string(), TEXT)],
        w,
    ));
    if !job.env.is_empty() {
        let vars: Vec<String> = job.env.iter().map(|(k, _)| k.clone()).collect();
        lines.push(detail_row(
            "Variables",
            vec![plain(vars.join(", "), TEXT)],
            w,
        ));
    }
    if let Some(note) = &job.note {
        lines.push(detail_row("Note", vec![plain(note.clone(), YELLOW)], w));
    }
    let note = wrap(
        "Only in your crontab: cron runs it, and this list does not see its runs. ⏎ takes it over, as a cron job or as a Herdr routine, with its output and failures in Runs.",
        w.saturating_sub(2),
    );
    // The gap above the note goes first when the rows are short.
    if lines.len() + 1 + note.len() <= area.height as usize {
        lines.push(Line::from(""));
    }
    for l in note {
        lines.push(Line::from(plain(format!(" {l}"), MUTED)));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// A run in a few coloured words: status, when, why and what it came to.
fn run_words(icons: &Icons, run: &Run, now: &DateTime<Local>) -> Vec<Span<'static>> {
    let (icon, color) = icons.status(run.status);
    let mut v = vec![
        plain(format!("{icon} {}", run.status.label()), color),
        plain(format!(" · {}", crate::app::ago(&run.started, now)), FAINT),
    ];
    if !run.message.trim().is_empty() {
        v.push(plain(format!(" · {}", run.message.trim()), MUTED));
    }
    if let Some(first) = run.result.lines().find(|l| !l.trim().is_empty()) {
        v.push(plain(format!(" · {}", first.trim()), TEXT));
    }
    v
}

/// The selected routine in full: when it runs, how its last run went and
/// why, what it runs with, its limits and the start of its prompt or
/// command. It says what the list says: no next time for a routine that
/// cannot run.
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
    let mut lines: Vec<Line> = vec![details_title(&r.file.name, w)];
    let text = |s: String| vec![plain(s, TEXT)];
    let room = w.saturating_sub(DETAIL_LABEL + 2);
    if r.unreadable {
        // Nothing of it could be read: only what is wrong and what helps.
        lines.push(detail_row(
            "Problem",
            vec![plain(r.problem.clone().unwrap_or_default(), RED)],
            w,
        ));
        lines.push(Line::from(""));
        for l in wrap(
            "Its routine.toml cannot be read, so it never runs. ⏎ opens the file to fix it; d deletes the routine.",
            w.saturating_sub(2),
        ) {
            lines.push(Line::from(plain(format!(" {l}"), FAINT)));
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), inner);
        return;
    }
    if let Some(p) = &r.problem {
        lines.push(detail_row("Problem", vec![plain(p.clone(), RED)], w));
    } else if !r.approved {
        let why = if app.known.contains(&r.id) {
            "changed outside the list"
        } else {
            "new, not approved yet"
        };
        lines.push(detail_row(
            "Approval",
            vec![plain(format!("{why} · a shows it for approval"), YELLOW)],
            w,
        ));
    }
    // The schedule, and when it runs next (or why it does not).
    let mut when = vec![plain(r.schedule_text(), TEXT)];
    let next = next_words(app, r, &now);
    let at_start = matches!(r.schedule(), Some(Schedule::AtStart)) && r.can_run() && f.enabled;
    if !at_start && next.content != "–" {
        when.push(plain(" · ", FAINT));
        when.push(next);
    }
    lines.push(detail_row("Schedule", when, w));
    let last = match app.latest_run(&r.id) {
        Some(run) => {
            let mut words = run_words(icons, run, &now);
            if let Some(n) = app.failing.get(&r.id) {
                // Right behind the status: it is not the first.
                words.insert(1, plain(format!(" · went wrong {n} times in a row"), RED));
            }
            words
        }
        None => vec![plain("no run yet", FAINT)],
    };
    lines.push(detail_row("Last run", last, w));
    lines.push(detail_row("Runs with", text(runs_with(r)), w));
    if r.is_cron() {
        // The schedule as the crontab line has it; the command is below.
        let when = crate::crontab::when(r).unwrap_or_default();
        lines.push(detail_row(
            "Crontab line",
            vec![plain(when, TEXT), plain(" and the command below", FAINT)],
            w,
        ));
    } else {
        lines.push(detail_row("Folder", text(clip_start(&f.folder, room)), w));
    }
    lines.push(detail_row(
        "Time limit",
        text(format!(
            "{} minute{}, then the run {}",
            f.time_limit_minutes,
            if f.time_limit_minutes == 1 { "" } else { "s" },
            if f.stop_at_limit || r.is_cron() {
                "is stopped"
            } else {
                "keeps running"
            }
        )),
        w,
    ));
    if f.retries > 0 {
        lines.push(detail_row(
            "If it fails",
            text(match f.retries {
                1 => format!("tries once more after {} minutes", f.retry_delay_minutes),
                n => format!(
                    "tries up to {n} more times, {} minutes apart",
                    f.retry_delay_minutes
                ),
            }),
            w,
        ));
    }
    if f.missed == crate::routine::Missed::RunOnce && f.more.late_hours > 0 {
        lines.push(detail_row(
            "If missed",
            text(
                crate::form::late_text(f.more.late_hours)
                    .0
                    .to_lowercase()
                    .replacen("run once", "runs once", 1),
            ),
            w,
        ));
    }
    if f.more.off_after_failures > 0 {
        lines.push(detail_row(
            "Switches off",
            text(format!(
                "after {}",
                crate::form::off_after_text(f.more.off_after_failures).0
            )),
            w,
        ));
    }
    if !f.more.after_command.trim().is_empty() {
        let when = if f.more.after_always {
            "after every run"
        } else {
            "after a run that went wrong"
        };
        lines.push(detail_row(
            "After the run",
            vec![
                plain(f.more.after_command.trim().to_string(), TEXT),
                plain(format!(" · {when}"), FAINT),
            ],
            w,
        ));
    }
    if !f.more.account.is_empty() && !r.is_shell() {
        lines.push(detail_row(
            "Account",
            text(app.account_name(&f.more.account)),
            w,
        ));
    }
    if f.alert != crate::routine::Alert::Notification {
        lines.push(detail_row(
            "Alerts",
            text(f.alert.label().to_lowercase()),
            w,
        ));
    }
    lines.push(detail_row(
        "Run history",
        text(crate::form::keep_text(f.keep_runs, f.keep_days).1),
        w,
    ));
    if !r.is_shell() {
        let mut extras = Vec::new();
        if !f.check_command.is_empty() {
            extras.push(format!("checks first: {}", f.check_command));
        }
        if f.remember {
            extras.push("gets earlier results".into());
        }
        if !f.args.is_empty() {
            extras.push(format!("extra options: {}", f.args.join(" ")));
        }
        if !extras.is_empty() {
            lines.push(detail_row("Agent settings", text(extras.join(" · ")), w));
        }
    }
    // The prompt or command fills the rest.
    let label = if r.is_shell() { "Command" } else { "Prompt" };
    let left = (inner.height as usize).saturating_sub(lines.len());
    if left > 0 {
        let mut body: Vec<String> = Vec::new();
        for l in r.prompt.lines() {
            if body.len() > left {
                break;
            }
            if l.trim().is_empty() {
                continue;
            }
            body.extend(wrap(l, room.max(10)));
        }
        let more = body.len() > left;
        body.truncate(left);
        if more {
            if let Some(last) = body.last_mut() {
                *last = format!("{} …", clip(last, room.saturating_sub(2)));
            }
        }
        if body.is_empty() {
            lines.push(detail_row(label, vec![plain("empty", FAINT)], w));
        }
        for (i, b) in body.into_iter().enumerate() {
            let shown = if i == 0 { label } else { "" };
            lines.push(detail_row(shown, vec![plain(b, MUTED)], w));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// The second line of a run's row: what went wrong and what it came to,
/// else which agent ran. How it was started shows once, under its time.
fn run_about(run: &Run) -> Line<'static> {
    let result = run
        .result
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string());
    let message = run.message.trim();
    let failed = run.status.problem() || run.status == Status::Skipped;
    match (failed && !message.is_empty(), result) {
        (true, Some(r)) => Line::from(vec![
            plain(message.to_string(), MUTED),
            plain(format!(" · {r}"), FAINT),
        ]),
        (true, None) => Line::from(plain(message.to_string(), MUTED)),
        (false, Some(r)) => faint(r),
        (false, None) if !message.is_empty() => faint(message.to_string()),
        (false, None) if run.agent == crate::routine::SHELL => faint("shell command"),
        (false, None) => faint(run.agent.clone()),
    }
}

fn draw_runs(frame: &mut Frame, app: &mut App, icons: &Icons, area: Rect) {
    // The filters show as a line on top, each with a ✕ that removes it.
    let mut area = area;
    if app.runs_filtered() {
        let bar = Rect::new(area.x + 1, area.y, area.width.saturating_sub(2), 1);
        let room = bar.width as usize;
        let mut lead = " Only";
        let tail = format!("  {} of {}  ", app.runs.len(), app.all_runs.len());
        let name = app.filter_name().map(str::to_string);
        let search = app.runs_search.trim().to_string();
        // Esc goes back to the routine the filter came from; without one
        // it shows all runs, which `h` always does.
        let mut buttons: Vec<(&str, char)> = match (app.filter_from, &name) {
            (Some(_), Some(_)) => vec![
                (" Esc back to the routine ", '\u{1b}'),
                (" h all runs ", 'h'),
            ],
            _ => vec![(" Esc all runs ", '\u{1b}')],
        };
        if app.runs.is_empty() {
            // `h` needs a run to stand on.
            buttons.truncate(1);
        }
        let chips = |name_room: usize, short: bool, text_room: usize| {
            let mut v: Vec<(String, RunFilter)> = Vec::new();
            if let Some(name) = &name {
                v.push((
                    format!(" runs of {} ✕ ", clip(name, name_room)),
                    RunFilter::Routine,
                ));
            }
            if app.runs_problems {
                let text = if short {
                    " problems ✕ "
                } else {
                    " problems only ✕ "
                };
                v.push((text.to_string(), RunFilter::Problems));
            }
            if !search.is_empty() {
                v.push((
                    format!(" “{}” ✕ ", clip(&search, text_room)),
                    RunFilter::Search,
                ));
            }
            v
        };
        let width = |lead: &str, c: &[(String, RunFilter)], b: &[(&str, char)]| {
            lead.width()
                + c.iter().map(|(t, _)| t.width() + 1).sum::<usize>()
                + tail.width()
                + b.iter().map(|(t, _)| t.width() + 1).sum::<usize>()
        };
        // When the line is too long: the buttons go (Esc is in the footer
        // too), then the words get shorter, the word in front goes, and
        // at last the name and the searched text are cut. The count stays.
        let mut shown = chips(usize::MAX, false, usize::MAX);
        while width(lead, &shown, &buttons) > room && !buttons.is_empty() {
            buttons.pop();
        }
        if width(lead, &shown, &buttons) > room {
            shown = chips(usize::MAX, true, usize::MAX);
        }
        if width(lead, &shown, &buttons) > room {
            lead = "";
        }
        if width(lead, &shown, &buttons) > room {
            let over = width(lead, &shown, &buttons) - room;
            let name_w = name.as_ref().map_or(0, |n| n.width());
            let name_room = name_w.saturating_sub(over).max(8);
            shown = chips(name_room, true, usize::MAX);
            if width(lead, &shown, &buttons) > room {
                let over = width(lead, &shown, &buttons) - room;
                shown = chips(name_room, true, search.width().saturating_sub(over).max(4));
            }
        }
        let mut spans = vec![plain(lead, MUTED)];
        let mut x = bar.x + lead.width() as u16;
        for (text, which) in shown {
            spans.push(Span::raw(" "));
            x += 1;
            let w = text.width() as u16;
            if x + w <= bar.right() {
                app.hits
                    .push((Rect::new(x, bar.y, w, 1), Hit::Unfilter(which)));
            }
            x += w;
            spans.push(Span::styled(
                text,
                Style::new()
                    .fg(TEXT)
                    .bg(SELECTED_BG)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        x += tail.width() as u16;
        spans.push(plain(tail, MUTED));
        for (text, key) in buttons {
            let w = text.width() as u16;
            if x + w <= bar.right() {
                app.hits.push((Rect::new(x, bar.y, w, 1), Hit::Key(key)));
            }
            x += w + 1;
            spans.push(Span::styled(text, Style::new().fg(TEXT).bg(BORDER)));
            spans.push(Span::raw(" "));
        }
        frame.render_widget(Paragraph::new(clip_line(Line::from(spans), room)), bar);
        area = Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        );
    }
    if app.runs.is_empty() {
        let lines: Vec<(String, Color)> = if app.runs_filtered() {
            vec![
                ("No run matches.".into(), TEXT),
                (String::new(), TEXT),
                ("Esc shows all runs again.".into(), MUTED),
            ]
        } else {
            vec![
                ("No runs yet.".into(), TEXT),
                (String::new(), TEXT),
                (
                    "Runs show up here when a routine is due, or when you start one with r.".into(),
                    MUTED,
                ),
            ]
        };
        empty_note(frame, area, &lines);
        return;
    }
    let now = Local::now();
    let runs = app.runs.clone();
    struct RunRow {
        mark: Span<'static>,
        name: Line<'static>,
        about: Line<'static>,
        started: String,
        how: String,
        status: Span<'static>,
        took: String,
    }
    let rows: Vec<RunRow> = runs
        .iter()
        .map(|run| {
            let (icon, color) = icons.status(run.status);
            let mut name = vec![Span::styled(run.routine_name.clone(), bold(TEXT))];
            // Runs kept when their routine was deleted.
            if !app.routines.iter().any(|r| r.id == run.routine_id) {
                name.push(plain("  deleted routine", FAINT));
            }
            let open = !run.tab_closed && !run.pane_id.is_empty() && run.status != Status::Closed;
            if open {
                name.push(plain(format!("  {} tab open", icons.tab_open()), ACCENT));
            }
            RunRow {
                mark: plain(icon, color),
                name: Line::from(name),
                about: run_about(run),
                started: short_time(&run.started, &now),
                how: if run.attempt > 0 {
                    format!("{} {}", run.trigger.label(), run.attempt)
                } else {
                    run.trigger.label().to_string()
                },
                status: plain(run.status.label(), color),
                took: match run.status {
                    // A skipped run never started.
                    Status::Skipped => String::new(),
                    s if s.active() => format!("for {}", crate::app::took(run)),
                    _ => format!("took {}", crate::app::took(run)),
                },
            }
        })
        .collect();
    let most = |f: &dyn Fn(&RunRow) -> usize| rows.iter().map(f).max().unwrap_or(0) as u16;
    let started_w = most(&|r| r.started.width().max(r.how.width())).clamp(7, 14);
    let inner = area.width.saturating_sub(2);
    let gap = spacing(inner);
    // The start time has a column of its own while the run keeps enough
    // room; else it goes in front of how long the run took.
    let with_started = {
        let status_w = most(&|r| r.status.width().max(r.took.width())).clamp(6, 18);
        inner >= 2 + 3 * gap + NAME_MIN + started_w + status_w
    };
    let status_line = |r: &RunRow| {
        if with_started || r.took.is_empty() {
            if with_started {
                r.took.clone()
            } else {
                r.started.clone()
            }
        } else {
            format!("{} · {}", r.started, r.took.trim_start_matches("took "))
        }
    };
    let status_w = most(&|r| r.status.width().max(status_line(r).width())).clamp(6, 22);
    let cols: Vec<Col> = if with_started {
        vec![
            Col { head: "", width: 2 },
            Col {
                head: "Run",
                width: inner.saturating_sub(2 + 3 * gap + started_w + status_w),
            },
            Col {
                head: "Started",
                width: started_w,
            },
            Col {
                head: "Status",
                width: status_w,
            },
        ]
    } else {
        vec![
            Col { head: "", width: 2 },
            Col {
                head: "Run",
                width: inner.saturating_sub(2 + 2 * gap + status_w),
            },
            Col {
                head: "Status",
                width: status_w,
            },
        ]
    };
    let cells: Vec<Cells> = rows
        .into_iter()
        .map(|r| {
            let second = faint(status_line(&r));
            let mut v = vec![[Line::from(r.mark), Line::from("")], [r.name, r.about]];
            if with_started {
                v.push([Line::from(plain(r.started, TEXT)), faint(r.how)]);
            }
            v.push([Line::from(r.status), second]);
            v
        })
        .collect();
    let sel = app.sel_run;
    draw_list(
        frame,
        app,
        area,
        Table {
            list: 2,
            gap,
            cols: &cols,
        },
        cells,
        sel,
    );
}

// ---- form ----------------------------------------------------------------

/// The width of the form's labels.
const LABEL_W: usize = 19;

/// A typed field's text. Without the cursor it is cut with "…" (a folder
/// at its start, since its end says the most). With the cursor, the part
/// around it shows, so what is typed is always in sight.
fn text_value(
    value: &str,
    cursor: Option<usize>,
    room: usize,
    from_end: bool,
) -> Vec<Span<'static>> {
    let Some(cursor) = cursor else {
        let shown = if from_end {
            clip_start(value, room)
        } else {
            clip(value, room)
        };
        return vec![plain(shown, TEXT)];
    };
    let chars: Vec<char> = value.chars().collect();
    let cursor = cursor.min(chars.len());
    let width = |from: usize, to: usize| -> usize {
        chars[from..to].iter().map(|c| c.width().unwrap_or(0)).sum()
    };
    // One column is the cursor's.
    let room = room.saturating_sub(1).max(3);
    let (mut from, mut to) = (0, chars.len());
    if width(0, chars.len()) > room {
        // Slide the window so the cursor is in it, with an ellipsis at
        // each end that is cut.
        while from < cursor && width(from, cursor) + 2 > room {
            from += 1;
        }
        let lead = usize::from(from > 0);
        to = cursor;
        while to < chars.len() {
            let tail = usize::from(to + 1 < chars.len());
            if lead + width(from, to + 1) + tail > room {
                break;
            }
            to += 1;
        }
    }
    let text = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
    let mut v = Vec::new();
    if from > 0 {
        v.push(plain("…", FAINT));
    }
    v.push(plain(text(from, cursor), TEXT));
    v.push(plain("▏", ACCENT));
    v.push(plain(text(cursor, to), TEXT));
    if to < chars.len() {
        v.push(plain("…", FAINT));
    }
    v
}

/// A field's value; `room` is the width it has. For a choice it also
/// gives where its ‹ and › are, counted from the value's start.
fn field_value(
    form: &Form,
    f: Field,
    selected: bool,
    room: usize,
) -> (Vec<Span<'static>>, Option<(u16, u16)>) {
    match f {
        Field::More => {
            let (sign, words) = if form.more {
                ("▾ ", "shown below".to_string())
            } else {
                ("▸ ", form.more_summary())
            };
            (
                vec![
                    plain(sign, ACCENT),
                    plain(clip(&words, room.saturating_sub(2)), FAINT),
                ],
                None,
            )
        }
        f if f.is_choice() => {
            // ‹ › show that ← → (or a click on them) step through the
            // choices.
            let arrow = if selected { ACCENT } else { FAINT };
            let shown = clip(&form.shown(f), room.saturating_sub(4));
            let value = if f == Field::Enabled {
                plain(shown.clone(), if form.enabled { GREEN } else { MUTED })
            } else {
                plain(shown.clone(), TEXT)
            };
            let right = 2 + shown.width() as u16;
            let mut v = vec![plain("‹ ", arrow), value, plain(" ›", arrow)];
            let detail = form.detail(f);
            // The short detail only when it fits whole; the explanation
            // below the form says more.
            let left = room.saturating_sub(shown.width() + 6);
            if !detail.is_empty() && detail.width() <= left {
                v.push(plain(format!("  {detail}"), FAINT));
            }
            (v, Some((0, right)))
        }
        f => {
            let value = form.text(f).unwrap_or("").to_string();
            let suffix = if f == Field::Limit { " minutes" } else { "" };
            let room = room.saturating_sub(suffix.width());
            let mut v = if value.is_empty() && !selected {
                let empty = match f {
                    Field::Name => "required",
                    Field::Args => "none",
                    Field::Check => "none, always runs",
                    Field::After => "nothing",
                    _ => "",
                };
                vec![plain(empty, FAINT)]
            } else {
                text_value(
                    &value,
                    selected.then_some(form.cursor),
                    room,
                    f == Field::Folder,
                )
            };
            if !suffix.is_empty() {
                v.push(plain(suffix, FAINT));
            }
            (v, None)
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
    /// The next times the schedule comes to, under the fields of "When".
    Next,
    Head(&'static str),
    Blank,
    Field(usize),
    /// A further line of the prompt's preview (1 is its second line).
    More(usize),
}

/// The schedule in words for the When heading, such as "Weekdays at
/// 09:00", or what is wrong with it.
fn schedule_summary(form: &Form) -> Span<'static> {
    match form.schedule() {
        // In the schedule's own words, as the list shows it.
        Ok(c) => plain(Schedule::parse(&c).expect("checked").describe(), GREEN),
        Err(e) => plain(e, RED),
    }
}

/// The next times the form's schedule comes to, so its owner sees that it
/// is the one they mean; or why there are none.
fn next_times(form: &Form, room: usize) -> Vec<Span<'static>> {
    let none = |why: &str, color: Color| vec![plain(why.to_string(), color)];
    // "When Herdr starts", also when it is typed as a custom schedule.
    let at_start = form
        .schedule()
        .ok()
        .and_then(|text| Schedule::parse(&text).ok())
        .is_some_and(|s| matches!(s, Schedule::AtStart));
    match form.upcoming(3) {
        Ok(times) if !times.is_empty() => {
            // Whole times only: as many of the three as fit.
            let mut v = Vec::new();
            let mut used = 0;
            for (i, t) in times.iter().enumerate() {
                let text = t.format("%a %-d %b %H:%M").to_string();
                let sep = if i > 0 { 3 } else { 0 };
                if i > 0 && used + sep + text.width() > room {
                    break;
                }
                if i > 0 {
                    v.push(plain(" · ", FAINT));
                }
                used += sep + text.width();
                v.push(plain(text, TEXT));
            }
            v
        }
        Ok(_) if at_start && form.is_cron() => none("each time the computer starts", MUTED),
        Ok(_) if at_start => none("each time Herdr starts", MUTED),
        Ok(_) => none("never: that time has passed", YELLOW),
        Err(_) => none("–", FAINT),
    }
}

/// The weekday switches: a sign and the day's name each, as wide as the
/// room allows. Returns where each day is, for the mouse.
fn day_switches(form: &Form, selected: bool, room: usize) -> (Vec<Span<'static>>, Vec<(u16, u16)>) {
    // Long names, then closer together, then two letters.
    let (letters, sign_gap, gap) = if room >= 47 {
        (3, " ", "  ")
    } else if room >= 41 {
        (3, " ", " ")
    } else {
        (2, "", " ")
    };
    let mut spans = Vec::new();
    let mut places = Vec::new();
    let mut x = 0u16;
    for (i, name) in crate::form::WEEKDAYS.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(gap));
            x += gap.width() as u16;
        }
        let on = form.days[i];
        let sign = if on { "●" } else { "○" };
        let text = format!("{sign}{sign_gap}{}", &name[..letters]);
        let mut style = Style::new().fg(if on { GREEN } else { MUTED });
        if selected && form.day_cursor == i {
            style = style
                .fg(if on { GREEN } else { TEXT })
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        let w = text.width() as u16;
        places.push((x, w));
        x += w;
        spans.push(Span::styled(text, style));
    }
    (spans, places)
}

/// The form: what the routine does, when and where it runs, then "More
/// settings". The save button and the explanation of the current field
/// stay in sight below the fields, however long the list above gets.
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
    let width = inner.width as usize;
    let fields = form.fields();
    let current = form.current();
    let save_at = fields.iter().position(|f| *f == Field::Save);

    // Below the fields: what the current field means, set off by an accent
    // bar like a tooltip. Four lines, or more (up to eight) where the
    // fields still keep six rows.
    let mut hint = wrap(form.hint(current), width.saturating_sub(4));
    let show_hint = inner.height >= 12;
    let most = (inner.height as usize).saturating_sub(10).clamp(4, 8);
    if hint.len() > most {
        hint.truncate(most);
        if let Some(last) = hint.last_mut() {
            *last = clip(&format!("{last} …"), width.saturating_sub(4));
        }
    }
    // A row between fields and button (it says what is out of sight), the
    // button, and the explanation with a gap above it.
    let fixed = 2 + if show_hint { 2 + hint.len() } else { 0 };
    let list_h = (inner.height as usize).saturating_sub(fixed).max(1);

    // The rows: a heading at the start of each part. An empty row closes
    // each part when the window is tall enough for all of it.
    let layout = |spaced: bool| {
        let mut rows: Vec<FormRow> = Vec::new();
        let mut last = "-";
        for (i, f) in fields.iter().enumerate() {
            if *f == Field::Save {
                continue;
            }
            let s = form.section(*f);
            if s != last {
                if spaced && !rows.is_empty() {
                    rows.push(FormRow::Blank);
                }
                if !s.is_empty() {
                    rows.push(FormRow::Head(s));
                }
                last = s;
            }
            rows.push(FormRow::Field(i));
            // The part "When" ends with the times it comes to.
            let last_of_when = s == "When"
                && fields
                    .get(i + 1)
                    .is_none_or(|next| form.section(*next) != "When");
            if last_of_when {
                rows.push(FormRow::Next);
            }
        }
        rows
    };
    let mut rows = layout(true);
    if rows.len() > list_h {
        rows = layout(false);
    }
    // The prompt shows its first lines, as many as fit (up to eight), and
    // ends in "…" when it goes on.
    let value_x = inner.x + 1 + LABEL_W as u16;
    let value_w = width.saturating_sub(LABEL_W + 2).max(10);
    let mut preview: Vec<String> = Vec::new();
    for l in form.prompt.trim_end().lines() {
        if l.trim().is_empty() {
            continue;
        }
        preview.extend(wrap(&l.replace('\t', "    "), value_w));
    }
    let prompt_at = fields.iter().position(|f| *f == Field::Prompt);
    // One row stays empty, so the form does not touch what is below it.
    let free = list_h.saturating_sub(rows.len() + 1);
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
        let line = preview.get(j).cloned().unwrap_or_default();
        if cut && j == extra {
            // One "…" says it goes on: its own, or the one of a line that
            // has no room for it.
            if line.width() + 2 > value_w {
                return vec![plain(clip(&format!("{line}…"), value_w), TEXT)];
            }
            return vec![plain(line, TEXT), plain(" …", ACCENT)];
        }
        vec![plain(line, TEXT)]
    };

    // The list scrolls only when the current field would leave the
    // window; the heading of its part stays in view with it.
    let mut top = form.top.min(rows.len().saturating_sub(list_h));
    if let Some(sel) = rows.iter().position(|r| *r == FormRow::Field(form.field)) {
        if sel < top {
            top = sel;
            if sel > 0 && matches!(rows[sel - 1], FormRow::Head(_)) {
                top -= 1;
            }
        } else if sel >= top + list_h {
            top = sel + 1 - list_h;
        }
    }
    if let Some(f) = app.form.as_mut() {
        f.top = top;
    }

    let schedule_words = schedule_summary(&form);
    let upcoming = next_times(&form, value_w);
    for (r, layout_row) in rows.iter().enumerate().skip(top).take(list_h) {
        let y = inner.y + (r - top) as u16;
        let row = Rect::new(inner.x, y, inner.width, 1);
        let i = match layout_row {
            FormRow::Blank => continue,
            FormRow::Next => {
                let mut spans = vec![plain(format!(" {:<w$}", "Next times", w = LABEL_W), FAINT)];
                spans.extend(upcoming.clone());
                frame.render_widget(Paragraph::new(clip_line(Line::from(spans), width)), row);
                continue;
            }
            FormRow::Head(name) => {
                let mut spans = vec![Span::styled(format!(" {name}"), bold(ACCENT))];
                if *name == "When" {
                    spans.push(Span::raw("  "));
                    spans.push(schedule_words.clone());
                }
                let used = Line::from(spans.clone()).width();
                let rule = width.saturating_sub(used + 2);
                if rule > 2 {
                    spans.push(plain(format!(" {}", "─".repeat(rule)), BORDER));
                }
                frame.render_widget(Paragraph::new(clip_line(Line::from(spans), width)), row);
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
        let f = fields[i];
        let selected = i == form.field;
        if selected {
            frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
        }
        app.hits.push((row, Hit::Field(i)));
        let label = format!(" {:<w$}", form.label(f), w = LABEL_W);
        let label_style = if selected {
            bold(TEXT)
        } else {
            Style::new().fg(MUTED)
        };
        let mut spans = vec![Span::styled(label, label_style)];
        if f == Field::Prompt {
            if preview.is_empty() {
                spans.push(plain("empty · ⏎ writes it", YELLOW));
            } else {
                spans.extend(preview_line(0));
            }
        } else if f == Field::Days {
            let (value, places) = day_switches(&form, selected, value_w);
            spans.extend(value);
            // A click on a day switches it.
            for (day, (dx, w)) in places.into_iter().enumerate() {
                app.hits
                    .push((Rect::new(value_x + dx, y, w, 1), Hit::Day(day)));
            }
        } else {
            let (value, arrows) = field_value(&form, f, selected, value_w);
            spans.extend(value);
            // A click on ‹ or › steps like ← and →.
            if let Some((left, right)) = arrows {
                app.hits
                    .push((Rect::new(value_x + left, y, 2, 1), Hit::Cycle(i, -1)));
                app.hits
                    .push((Rect::new(value_x + right, y, 2, 1), Hit::Cycle(i, 1)));
            }
        }
        let mut line = clip_line(Line::from(spans), width);
        if selected {
            line = relight(line);
        }
        frame.render_widget(Paragraph::new(line), row);
    }

    // Right under the fields, so it reads as part of the form.
    let shown = rows.len().saturating_sub(top).min(list_h) as u16;
    let y = inner.y + shown;
    if y + 1 >= inner.bottom() {
        return;
    }
    // What is out of sight above and below.
    let hidden = |range: std::ops::Range<usize>| {
        rows.get(range)
            .map(|r| r.iter().filter(|r| matches!(r, FormRow::Field(_))).count())
            .unwrap_or(0)
    };
    let (above, below) = (hidden(0..top), hidden(top + list_h..rows.len()));
    let more = match (above, below) {
        (0, 0) => String::new(),
        (a, 0) => format!("↑ {a} more above"),
        (0, b) => format!("↓ {b} more below"),
        (a, b) => format!("↑ {a} above · ↓ {b} below"),
    };
    frame.render_widget(
        Paragraph::new(plain(more.clone(), MUTED)).alignment(Alignment::Right),
        Rect::new(inner.x, y, inner.width.saturating_sub(1), 1),
    );
    // The save button, always in sight, and whether it can be saved.
    let on_save = save_at == Some(form.field);
    let save = button("⏎ save", on_save);
    let back = button("Esc back", false);
    let x = inner.x + 1;
    let (save_w, back_w) = (save.width() as u16, back.width() as u16);
    if let Some(i) = save_at {
        app.hits
            .push((Rect::new(x, y + 1, save_w, 1), Hit::Field(i)));
    }
    app.hits.push((
        Rect::new(x + save_w + 2, y + 1, back_w, 1),
        Hit::Key('\u{1b}'),
    ));
    let status = match form.build() {
        Ok(_) => plain("ready to save", GREEN),
        Err(e) => plain(format!("not ready: {e}"), YELLOW),
    };
    let beside = width.saturating_sub((save_w + back_w) as usize + 7);
    let mut buttons = vec![Span::raw(" "), save, Span::raw("  "), back];
    if status.width() <= beside {
        buttons.push(Span::raw("   "));
        buttons.push(status);
    } else {
        // Too long to sit beside the buttons: on the row above them.
        let room = width.saturating_sub(more.width() + 3);
        frame.render_widget(
            Paragraph::new(clip_line(Line::from(vec![Span::raw(" "), status]), room)),
            Rect::new(inner.x, y, inner.width, 1),
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(buttons)),
        Rect::new(inner.x, y + 1, inner.width, 1),
    );
    if !show_hint || y + 3 >= inner.bottom() {
        return;
    }
    let bar = || plain("▎ ", ACCENT);
    let mut lines = vec![Line::from(vec![
        bar(),
        Span::styled(form.label(current), bold(TEXT)),
    ])];
    lines.extend(
        hint.into_iter()
            .map(|l| Line::from(vec![bar(), plain(l, MUTED)])),
    );
    frame.render_widget(
        Paragraph::new(Text::from(lines)),
        Rect::new(
            inner.x + 1,
            y + 3,
            inner.width.saturating_sub(2),
            inner.bottom() - (y + 3),
        ),
    );
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
    let empty = ed.text.lines.len() == 1 && ed.text.lines[0].is_empty();
    if empty {
        let what = if matches!(&ed.target, EditTarget::File(_)) {
            "The file is empty."
        } else {
            "Write what the agent should do, as you would type it to the agent."
        };
        let lines: Vec<Line> = wrap(what, width.saturating_sub(1))
            .into_iter()
            .map(|l| Line::from(plain(l, FAINT)))
            .collect();
        frame.render_widget(
            Paragraph::new(Text::from(lines)),
            Rect::new(
                text.x + 1,
                text.y,
                text.width.saturating_sub(1),
                text.height,
            ),
        );
    }
    let marked = ed.text.selection();
    let normal = Style::new().fg(TEXT);
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
                    let style = if run_marked { mark } else { normal };
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
                let style = if run_marked { mark } else { normal };
                spans.push(Span::styled(run, style));
            }
            if last_of_line && next_marked {
                spans.push(Span::styled(" ", mark));
            }
            Line::from(spans)
        })
        .collect();
    if !empty {
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
                Paragraph::new(plain(label, MUTED)),
                Rect::new(outer.x + outer.width - w - 2, outer.bottom() - 1, w, 1),
            );
        }
    }
}

// ---- footer ----------------------------------------------------------------

/// A key in the footer: what it does, what a click on it sends, and how
/// soon it gives way when the footer is narrow (0 stays longest).
struct FootKey {
    key: String,
    what: String,
    hit: Option<Hit>,
    rank: u8,
}

fn foot(key: &str, what: impl Into<String>, hit: Option<Hit>, rank: u8) -> FootKey {
    FootKey {
        key: key.to_string(),
        what: what.into(),
        hit,
        rank,
    }
}

const ESC: Option<Hit> = Some(Hit::Key('\u{1b}'));
const ENTER: Option<Hit> = Some(Hit::Key('\n'));

fn hit(key: char) -> Option<Hit> {
    Some(Hit::Key(key))
}

/// The keys of what is in front: the open dialog's, else the editor's,
/// the form's or the list's. To offer a key in the footer, add it here,
/// in the place it should show, with its rank.
fn footer_keys(app: &App) -> Vec<FootKey> {
    match &app.mode {
        Mode::Help => vec![foot("↑ ↓", "scroll", None, 1), foot("Esc", "close", ESC, 0)],
        Mode::Pick(p) => vec![
            foot("⏎", "choose", ENTER, 0),
            if p.filters() {
                foot("type", "to narrow the list", None, 2)
            } else {
                foot("1–9", "choose by number", None, 2)
            },
            foot("Esc", "back", ESC, 0),
        ],
        Mode::Input(_) => vec![foot("⏎", "ok", ENTER, 0), foot("Esc", "cancel", ESC, 0)],
        Mode::RunPage(page) => {
            let mut v = Vec::new();
            if let Some(run) = app.page_run(page) {
                let open = !run.tab_closed && run.cron_pid == 0 && !run.pane_id.is_empty();
                if open {
                    v.push(foot("o", "go to its tab", hit('o'), 1));
                }
                v.push(foot("l", "whole output", hit('l'), 2));
                if run.status.active() {
                    v.push(foot("x", "stop", hit('x'), 1));
                } else {
                    v.push(foot("t", "chat about it", hit('t'), 1));
                }
                if app.routines.iter().any(|r| r.id == run.routine_id) {
                    v.push(foot("r", "run again", hit('r'), 3));
                }
                v.push(foot("f", "open file", hit('f'), 4));
                if !run.status.active() {
                    v.push(foot("d", "delete", hit('d'), 3));
                }
            }
            v.push(foot("Esc", "close", ESC, 0));
            v
        }
        Mode::Confirm(c) => {
            let mut v = vec![foot(
                if c.enter_confirms() { "y ⏎" } else { "y" },
                c.yes(),
                hit('y'),
                0,
            )];
            if matches!(c.purpose, crate::app::ConfirmFor::Delete(_, true)) {
                v.push(foot("r", "delete with its runs", hit('r'), 0));
            }
            v.push(foot("n", c.no(), hit('n'), 0));
            v
        }
        Mode::Message { .. } => {
            let mut v = vec![foot("⏎", "ok", ENTER, 0)];
            if app.help_max > 0 {
                v.push(foot("↑ ↓", "scroll", None, 1));
            }
            v
        }
        Mode::Log(log) => {
            let mut v = Vec::new();
            if log.approve.is_some() {
                v.push(if log.scroll >= log.max {
                    foot("y", "approve", hit('y'), 0)
                } else {
                    foot("↓", "read to the end to approve", None, 0)
                });
            }
            v.push(foot("Esc", "close", ESC, 0));
            if log.file.is_some() {
                v.push(foot("f", "open file", hit('f'), 2));
            }
            if log.max > 0 {
                v.push(foot("↑ ↓", "scroll", None, 1));
            }
            v
        }
        Mode::Cron(_) => vec![
            foot("⏎", "choose", ENTER, 1),
            foot("k", "keep as a cron job", hit('k'), 2),
            foot("m", "move to Herdr", hit('m'), 2),
            foot("c", "copy", hit('c'), 2),
            foot("Esc", "back", ESC, 0),
        ],
        Mode::Menu(_) => vec![foot("⏎", "choose", ENTER, 0), foot("Esc", "close", ESC, 0)],
        Mode::AskQuit => Vec::new(),
        Mode::Normal => {
            if let Some(ed) = &app.editor {
                editor_keys(ed)
            } else if let Some(form) = &app.form {
                form_keys(form)
            } else {
                list_keys(app)
            }
        }
    }
}

fn editor_keys(ed: &crate::app::Editor) -> Vec<FootKey> {
    vec![
        foot(
            "Esc",
            // A routine's own prompt is written at once; the form's is
            // kept until the form is saved.
            if ed.target == EditTarget::Form {
                "back to the form"
            } else {
                "save and close"
            },
            ESC,
            0,
        ),
        foot("Ctrl+D", "discard", Some(Hit::Ctrl('d')), 1),
        foot("Ctrl+Z", "undo", Some(Hit::Ctrl('z')), 2),
        foot("Ctrl+Y", "redo", Some(Hit::Ctrl('y')), 3),
        foot("Ctrl+O", "other editor", Some(Hit::Ctrl('o')), 4),
    ]
}

fn form_keys(form: &Form) -> Vec<FootKey> {
    let f = form.current();
    let enter = match f {
        Field::Prompt if form.is_shell() => "write the command",
        Field::Prompt => "write the prompt",
        Field::Save => "save",
        Field::More if form.more => "hide",
        Field::More => "show",
        Field::Days => "switch the day",
        f if f.is_choice() => "choose",
        _ => "next",
    };
    let mut v = vec![foot("⏎", enter, ENTER, 0)];
    if f == Field::Days {
        v.push(foot("← →", "day", None, 2));
        v.push(foot("1-7", "Mon to Sun", None, 4));
    }
    if f.is_choice() {
        v.push(foot("← →", "change", None, 2));
    }
    if f.is_text() {
        v.push(foot("← →", "cursor", None, 4));
    }
    v.push(foot("↑ ↓", "field", None, 3));
    v.push(foot("Ctrl+S", "save", Some(Hit::Ctrl('s')), 0));
    v.push(foot("Esc", "back", ESC, 0));
    if f.is_text() {
        v.push(foot("Ctrl+U", "clear", Some(Hit::Ctrl('u')), 5));
    } else {
        // In a text field `?` is typed.
        v.push(foot("?", "all keys", hit('?'), 1));
    }
    v
}

fn list_keys(app: &App) -> Vec<FootKey> {
    let help = foot("?", "all keys", hit('?'), 0);
    let close = foot("q", "close", hit('q'), 2);
    match app.view {
        View::Cron if app.selected_cron_job().is_some() => vec![
            foot("⏎", "take it over", ENTER, 0),
            foot("n", "new", hit('n'), 1),
            foot("C", "take over all", hit('C'), 3),
            help,
            close,
        ],
        View::Routines | View::Cron => {
            let Some(r) = app.selected_routine() else {
                // An empty list: only what works without a routine.
                return vec![foot("n", "new", hit('n'), 0), help, close];
            };
            let pending = !r.approved && !r.unreadable;
            let run = app.latest_run(&r.id);
            let open = run
                .is_some_and(|run| !run.tab_closed && run.cron_pid == 0 && !run.pane_id.is_empty());
            let needs_you = run.is_some_and(|run| run.status == Status::NeedsYou);
            let mut v = vec![foot("⏎", "edit", ENTER, 0), foot("n", "new", hit('n'), 1)];
            v.push(if pending {
                foot("a", "approve", hit('a'), 0)
            } else {
                foot("r", "run now", hit('r'), 1)
            });
            v.push(foot("space", "on/off", hit(' '), 2));
            // The run that waits for the owner comes before the rest.
            if open {
                v.push(foot(
                    "o",
                    "go to its tab",
                    hit('o'),
                    if needs_you { 0 } else { 3 },
                ));
            }
            if run.is_some_and(|run| run.status.active()) {
                v.push(foot("x", "stop its run", hit('x'), 3));
            }
            if let Some(run) = run {
                v.push(foot("l", "output", hit('l'), 3));
                if !run.status.active() {
                    v.push(foot("t", "chat about its run", hit('t'), 4));
                }
                v.push(foot("h", "its runs", hit('h'), 4));
            }
            v.push(foot(
                "p",
                if r.is_shell() { "command" } else { "prompt" },
                hit('p'),
                5,
            ));
            v.push(foot("c", "duplicate", hit('c'), 6));
            v.push(foot("d", "delete", hit('d'), 6));
            v.push(foot(
                "s",
                if app.sort == Sort::Attention {
                    "sort by name"
                } else {
                    "sort by what needs you"
                },
                hit('s'),
                7,
            ));
            v.push(help);
            v.push(close);
            v
        }
        View::Runs => {
            let mut v = Vec::new();
            let run = app.runs.get(app.sel_run);
            if let Some(run) = run {
                let open = !run.tab_closed && run.cron_pid == 0 && !run.pane_id.is_empty();
                v.push(foot("⏎", "open", ENTER, 0));
                // The run that waits for the owner: its tab comes first.
                if open {
                    let rank = if run.status == Status::NeedsYou { 0 } else { 2 };
                    v.push(foot("o", "go to its tab", hit('o'), rank));
                }
                v.push(foot("l", "output", hit('l'), 2));
                if run.status.active() {
                    v.push(foot("x", "stop", hit('x'), 2));
                } else {
                    v.push(foot("t", "chat about it", hit('t'), 2));
                }
            }
            if app.runs_filtered() {
                v.push(match (app.filter_from, &app.run_filter) {
                    (Some(_), Some(_)) => foot("Esc", "back to the routine", ESC, 1),
                    _ => foot("Esc", "all runs", ESC, 1),
                });
            }
            if !app.all_runs.is_empty() {
                // Once it is on, its ✕ and Esc take it off (and `p` too).
                if !app.runs_problems {
                    v.push(foot("p", "problems only", hit('p'), 3));
                }
                v.push(foot("/", "search", hit('/'), 3));
            }
            if let Some(run) = run {
                if app.run_filter.is_none() {
                    v.push(foot("h", "only its runs", hit('h'), 4));
                }
                v.push(foot("f", "open file", hit('f'), 5));
                if app.routines.iter().any(|r| r.id == run.routine_id) {
                    v.push(foot("r", "run again", hit('r'), 5));
                }
                if !run.status.active() {
                    v.push(foot("d", "delete", hit('d'), 5));
                }
            }
            v.push(help);
            v.push(close);
            v
        }
    }
}

/// How many rows the footer needs: one, or more when a message does not
/// fit on one line (up to three).
fn footer_height(app: &App, width: u16) -> u16 {
    match (&app.flash, &app.mode) {
        (_, Mode::AskQuit) | (None, _) => 1,
        (Some(flash), _) => wrap(&flash.text, width.saturating_sub(2) as usize)
            .len()
            .clamp(1, 3) as u16,
    }
}

fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) {
    frame.render_widget(Block::new().style(Style::new().bg(BAR_BG)), area);
    let width = area.width as usize;
    if app.mode == Mode::AskQuit {
        let question = if width >= 60 {
            " Close the list? Routines keep running. "
        } else {
            " Close the list? "
        };
        let (yes, no) = (" y close ", " n stay ");
        let x = area.x + question.width() as u16;
        let line = Line::from(vec![
            Span::styled(question, bold(TEXT)),
            Span::styled(
                yes,
                Style::new()
                    .fg(theme::on(RED))
                    .bg(RED)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(no, Style::new().fg(TEXT).bg(BORDER)),
        ]);
        app.hits
            .push((Rect::new(x, area.y, yes.width() as u16, 1), Hit::Key('y')));
        app.hits.push((
            Rect::new(x + yes.width() as u16 + 1, area.y, no.width() as u16, 1),
            Hit::Key('n'),
        ));
        frame.render_widget(Paragraph::new(clip_line(line, width)), area);
        return;
    }
    // A message gets the room it needs: beside the keys when it is short,
    // the whole footer (over several rows) when it is long. It is never
    // cut short of the footer's three rows, and then ends in "…".
    let mut right_w = 0;
    if let Some(flash) = &app.flash {
        let mut lines = wrap(&flash.text, width.saturating_sub(2));
        if lines.len() > 1 {
            if lines.len() > area.height as usize {
                lines.truncate(area.height as usize);
                if let Some(last) = lines.last_mut() {
                    *last = clip(&format!("{last} …"), width.saturating_sub(2));
                }
            }
            let text: Vec<Line> = lines
                .into_iter()
                .map(|l| Line::from(plain(format!(" {l}"), flash.color)))
                .collect();
            frame.render_widget(Paragraph::new(Text::from(text)), area);
            return;
        }
        right_w = flash.text.width() + 3;
        frame.render_widget(
            Paragraph::new(plain(format!("{} ", flash.text), flash.color))
                .alignment(Alignment::Right),
            area,
        );
    } else if let (Some(ed), Mode::Normal) = (&app.editor, &app.mode) {
        let text = format!("line {} of {} ", ed.text.row + 1, ed.text.lines.len());
        right_w = text.width() + 2;
        frame.render_widget(
            Paragraph::new(plain(text, MUTED)).alignment(Alignment::Right),
            area,
        );
    }
    // The keys take what is left; the least important go first.
    let room = width.saturating_sub(1 + right_w);
    let mut keys = footer_keys(app);
    let need = |k: &FootKey| k.key.width() + 1 + k.what.width() + 2;
    // (The last key's own gap may hang over; a column stays free.)
    while keys.iter().map(need).sum::<usize>() > room + 1 && !keys.is_empty() {
        let worst = keys.iter().map(|k| k.rank).max().unwrap_or(0);
        let at = keys.iter().rposition(|k| k.rank == worst).unwrap_or(0);
        keys.remove(at);
    }
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x + 1;
    for k in keys {
        let text = format!(" {}  ", k.what);
        let w = (k.key.width() + text.width()) as u16;
        if let Some(hit) = k.hit {
            app.hits.push((Rect::new(x, area.y, w - 2, 1), hit));
        }
        x += w;
        spans.push(Span::styled(k.key, bold(TEXT)));
        spans.push(plain(text, MUTED));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, room as u16 + 1, 1),
    );
}

// ---- dialogs ---------------------------------------------------------------

/// A box of this size in the middle of the stage (the screen between
/// header and footer), made smaller when the stage is.
fn centered(stage: Rect, width: u16, height: u16) -> Rect {
    let w = width
        .min(stage.width.saturating_sub(2))
        .max(10.min(stage.width));
    let h = height.min(stage.height).max(3.min(stage.height));
    Rect::new(
        stage.x + (stage.width - w) / 2,
        stage.y + (stage.height - h) / 2,
        w,
        h,
    )
}

/// Draws a dialog's frame and returns the room inside it. Every dialog
/// goes through here: it notes the dialog's place, so a click beside it
/// closes it (`App::click`).
fn dialog(frame: &mut Frame, app: &mut App, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    // A dialog nearly as wide as the window leaves a strip of a few
    // columns on each side, where single letters of the page behind it
    // would show: those strips are cleared too.
    let window = frame.area();
    let right_edge = area.x + area.width;
    let (left, right) = (
        area.x.saturating_sub(window.x),
        (window.x + window.width).saturating_sub(right_edge),
    );
    if left <= 3 {
        frame.render_widget(Clear, Rect::new(window.x, area.y, left, area.height));
    }
    if right <= 3 {
        frame.render_widget(Clear, Rect::new(right_edge, area.y, right, area.height));
    }
    app.dialog = Some(area);
    let title = clip(title, area.width.saturating_sub(6) as usize);
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

/// Which rows show, in the bottom border of a dialog whose list scrolls.
fn position_note(frame: &mut Frame, area: Rect, top: usize, visible: usize, n: usize) {
    if n <= visible || area.height < 3 {
        return;
    }
    let text = format!(" {}–{} of {n} ", top + 1, (top + visible).min(n));
    let w = text.width() as u16;
    if area.width > w + 4 {
        frame.render_widget(
            Paragraph::new(plain(text, MUTED)),
            Rect::new(area.right() - w - 2, area.bottom() - 1, w, 1),
        );
    }
}

/// The right-click menu, at the mouse and moved inside the stage: each
/// entry with the key that does the same on the right.
fn draw_menu(frame: &mut Frame, app: &mut App, stage: Rect, m: &crate::app::Menu) {
    let label_w = m.items.iter().map(|i| i.label.width()).max().unwrap_or(0);
    let key_w = m.items.iter().map(|i| i.key.width()).max().unwrap_or(0);
    let width = ((label_w + key_w + 7) as u16).min(stage.width);
    let height = (m.items.len() as u16 + 2).min(stage.height);
    let x = m.x.clamp(
        stage.x,
        stage.right().saturating_sub(width + 1).max(stage.x),
    );
    let y = m.y.clamp(stage.y, stage.bottom().saturating_sub(height));
    let area = Rect::new(x, y, width, height);
    frame.render_widget(Clear, area);
    app.dialog = Some(area);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    for (i, item) in m.items.iter().enumerate().take(inner.height as usize) {
        let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        let selected = i == m.sel;
        if selected {
            frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
        }
        let color = if item.danger {
            RED
        } else if selected {
            TEXT
        } else {
            MUTED
        };
        let gap = (inner.width as usize).saturating_sub(item.label.width() + item.key.width() + 2);
        let line = Line::from(vec![
            Span::styled(
                format!(" {}", item.label),
                if selected {
                    bold(color)
                } else {
                    Style::new().fg(color)
                },
            ),
            Span::raw(" ".repeat(gap)),
            plain(format!("{} ", item.key), MUTED),
        ]);
        frame.render_widget(Paragraph::new(line), row);
        app.hits.push((row, Hit::MenuRow(i)));
    }
}

/// A list to choose from. A click on an entry chooses it; its keys are in
/// the footer.
fn draw_picker(frame: &mut Frame, app: &mut App, stage: Rect, mut p: crate::app::Picker) {
    // Details on a line of their own, except in long lists (the agents),
    // where they follow the name. The whole list decides, so typing into
    // the filter does not change the rows' shape.
    let any = p.all.iter().any(|i| !i.detail.is_empty());
    let two = any && p.all.len() <= 8;
    // Long lists get a filter line on top instead of number keys.
    let filter_h = u16::from(p.filters()) * 2;
    // A note about all entries goes under them.
    let hint: Vec<String> = if p.hint.is_empty() {
        Vec::new()
    } else {
        wrap(
            &p.hint,
            centered(stage, 72, 1).width.saturating_sub(4) as usize,
        )
    };
    let hint_h = if hint.is_empty() {
        0
    } else {
        hint.len() as u16 + 1
    };
    // A detail that is longer than the dialog is wide gets a second or
    // third row, the same for every entry, where all of them still fit;
    // in a low window it is cut instead.
    let detail_w = (centered(stage, 72, 1).width.saturating_sub(6) as usize).max(10);
    let detail_rows = p
        .all
        .iter()
        .map(|i| wrap(&i.detail, detail_w).len())
        .max()
        .unwrap_or(1)
        .clamp(1, 3) as u16;
    let mut per = if two { 1 + detail_rows } else { 1 };
    if per > 2 && p.all.len() as u16 * per + 2 + filter_h + hint_h > stage.height {
        per = 2;
    }
    let want_h = p.all.len() as u16 * per + 2 + filter_h + hint_h;
    let area = centered(stage, 72, want_h);
    let title = p.title.clone();
    let mut inner = dialog(frame, app, area, &title);
    // The note only where every entry still fits above it.
    if hint_h > 0 && inner.height >= p.all.len() as u16 * per + filter_h + hint_h {
        let y = inner.bottom() - hint.len() as u16;
        frame.render_widget(
            Paragraph::new(Text::from(
                hint.iter()
                    .map(|l| Line::from(plain(l.clone(), FAINT)))
                    .collect::<Vec<_>>(),
            )),
            Rect::new(inner.x, y, inner.width, hint.len() as u16),
        );
        inner.height -= hint_h;
    }
    if p.filters() {
        let line = Line::from(vec![
            plain("Filter ", MUTED),
            Span::styled(p.filter.clone(), bold(TEXT)),
            plain("▏", ACCENT),
            plain(
                if p.filter.is_empty() {
                    "type to narrow the list"
                } else {
                    ""
                },
                FAINT,
            ),
        ]);
        frame.render_widget(
            Paragraph::new(clip_line(line, inner.width as usize)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        inner = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(2),
        );
    }
    let visible = (inner.height / per).max(1) as usize;
    let n = p.items.len();
    // The list scrolls only when the marked entry would leave the window.
    let mut top = p.top.min(n.saturating_sub(visible));
    if p.sel < top {
        top = p.sel;
    } else if p.sel >= top + visible {
        top = p.sel + 1 - visible;
    }
    p.top = top;
    if p.items.is_empty() {
        frame.render_widget(
            Paragraph::new(plain("Nothing matches", FAINT)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
    }
    let width = inner.width as usize;
    for (i, item) in p.items.iter().enumerate().skip(top).take(visible) {
        let y = inner.y + ((i - top) as u16) * per;
        let r = Rect::new(inner.x, y, inner.width, per.min(inner.bottom() - y));
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
            plain(number, MUTED),
            Span::styled(item.label.clone(), bold(TEXT)),
        ];
        if !two && !item.detail.is_empty() {
            first.push(plain(format!("  {}", item.detail), MUTED));
        }
        let mut lines = vec![clip_line(Line::from(first), width)];
        if two {
            let rows = usize::from(per - 1);
            let mut detail = wrap(&item.detail, detail_w);
            if detail.len() > rows {
                // No room for all of it: the last row says it goes on.
                detail.truncate(rows);
                if let Some(last) = detail.last_mut() {
                    *last = clip(&format!("{last} …"), detail_w);
                }
            }
            for l in detail {
                lines.push(clip_line(Line::from(plain(format!("  {l}"), MUTED)), width));
            }
        }
        frame.render_widget(Paragraph::new(Text::from(lines)).style(style), r);
        app.hits.push((r, Hit::PickRow(i)));
    }
    position_note(frame, area, top, visible, n);
    app.mode = Mode::Pick(p);
}

fn draw_input(frame: &mut Frame, app: &mut App, stage: Rect, i: &crate::app::Input) {
    let width = centered(stage, 72, 1).width;
    let hint = wrap(&i.hint, width.saturating_sub(4) as usize);
    let area = centered(stage, width, hint.len() as u16 + 6);
    let inner = dialog(frame, app, area, &i.title);
    let field = Rect::new(
        inner.x,
        inner.y + 1.min(inner.height.saturating_sub(1)),
        inner.width,
        1,
    );
    frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), field);
    let mut value = vec![Span::raw(" ")];
    value.extend(text_value(
        &i.text,
        Some(i.text.chars().count()),
        inner.width.saturating_sub(1) as usize,
        false,
    ));
    frame.render_widget(Paragraph::new(Line::from(value)), field);
    if inner.height > 3 {
        frame.render_widget(
            Paragraph::new(Text::from(
                hint.into_iter()
                    .map(|l| Line::from(plain(l, MUTED)))
                    .collect::<Vec<_>>(),
            )),
            Rect::new(
                inner.x,
                inner.y + 3,
                inner.width,
                inner.height.saturating_sub(3),
            ),
        );
    }
}

/// A question. `y` has its own button, named after what it does: red when
/// something is lost, in the accent colour when ⏎ confirms it too.
fn draw_confirm(frame: &mut Frame, app: &mut App, stage: Rect, c: &crate::app::Confirm) {
    let width = centered(stage, 76, 1).width;
    let mut wrapped: Vec<String> = c
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
        .collect();
    let area = centered(stage, width, wrapped.len() as u16 + 5);
    let inner = dialog(frame, app, area, &c.question);
    // In a low window the gaps go first, then the end of the text.
    let (top, rest) = if (inner.height as usize) < wrapped.len() + 3 {
        (0, 1)
    } else {
        (1, 3)
    };
    let room = inner.height.saturating_sub(rest) as usize;
    if wrapped.len() > room && room > 0 {
        wrapped.truncate(room);
        if let Some(last) = wrapped.last_mut() {
            *last = clip(&format!("{last} …"), inner.width as usize);
        }
    }
    let text: Vec<Line> = wrapped
        .into_iter()
        .map(|l| Line::from(plain(l, MUTED)))
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(text)),
        Rect::new(inner.x, inner.y + top, inner.width, room as u16),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    let strong = |bg: Color| {
        Style::new()
            .fg(theme::on(bg))
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    };
    let yes_style = if c.danger() {
        strong(RED)
    } else if c.enter_confirms() {
        strong(ACCENT)
    } else {
        strong(GREEN)
    };
    let mut buttons: Vec<(String, char, Style)> = vec![(
        if c.enter_confirms() {
            format!(" y or ⏎ {} ", c.yes())
        } else {
            format!(" y {} ", c.yes())
        },
        'y',
        yes_style,
    )];
    // Deleting a routine with runs: keep them (y) or delete them too (r).
    if matches!(c.purpose, crate::app::ConfirmFor::Delete(_, true)) {
        buttons.push((" r delete with its runs ".into(), 'r', strong(RED)));
    }
    buttons.push((
        format!(" n {} ", c.no()),
        'n',
        Style::new().fg(TEXT).bg(BORDER),
    ));
    let mut spans = Vec::new();
    let mut x = inner.x;
    for (text, key, style) in buttons {
        let w = text.width() as u16;
        if x + w <= inner.right() {
            app.hits.push((Rect::new(x, y, w, 1), Hit::Key(key)));
        }
        x += w + 1;
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(clip_line(Line::from(spans), inner.width as usize)),
        Rect::new(inner.x, y, inner.width, 1),
    );
}

/// A note to read. One that is longer than the window scrolls.
fn draw_message(frame: &mut Frame, app: &mut App, stage: Rect, title: &str, text: &[String]) {
    let width = 78u16.min(stage.width.saturating_sub(2)).max(24);
    let mut lines: Vec<Line> = Vec::new();
    for (i, t) in text.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        let color = if i == 0 { TEXT } else { MUTED };
        for l in wrap(t, width.saturating_sub(6) as usize) {
            lines.push(Line::from(plain(l, color)));
        }
    }
    let area = centered(stage, width, lines.len() as u16 + 5);
    let inner = dialog(frame, app, area, title);
    // A gap above the text, one below it and the button; in a low window
    // only the button.
    let fits = inner.height as usize >= lines.len() + 3;
    let (top, rest) = if fits { (1, 3) } else { (0, 1) };
    let visible = inner.height.saturating_sub(rest);
    let total = lines.len() as u16;
    app.help_max = total.saturating_sub(visible);
    app.help_scroll = app.help_scroll.min(app.help_max);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((app.help_scroll, 0)),
        Rect::new(inner.x, inner.y + top, inner.width, visible),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    let ok = " ⏎ ok ";
    let mut spans = vec![Span::styled(
        ok,
        Style::new()
            .fg(theme::on(ACCENT))
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    )];
    if app.help_max > 0 {
        spans.push(plain(
            if app.help_scroll >= app.help_max {
                "  ↑ ↓ scroll · end".to_string()
            } else {
                format!(
                    "  ↑ ↓ scroll · {} more lines",
                    app.help_max - app.help_scroll
                )
            },
            MUTED,
        ));
    }
    frame.render_widget(
        Paragraph::new(clip_line(Line::from(spans), inner.width as usize)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    app.hits
        .push((Rect::new(inner.x, y, ok.width() as u16, 1), Hit::Key('\n')));
}

/// The jobs that are only in the crontab, and the ways to take them over,
/// as a list to choose from like any other.
fn draw_take(frame: &mut Frame, app: &mut App, stage: Rect, jobs: &[crate::crontab::CronJob]) {
    let width = stage.width.saturating_sub(4).min(100);
    let text_w = width.saturating_sub(4) as usize;
    // The ways, each with what it means on the lines below it.
    let ways: Vec<Vec<Line>> = TAKES
        .iter()
        .map(|(key, _, label, detail)| {
            let mut lines = vec![Line::from(vec![
                plain(format!(" {key}  "), MUTED),
                Span::styled(*label, bold(TEXT)),
            ])];
            for l in wrap(detail, text_w.saturating_sub(5)) {
                lines.push(Line::from(plain(format!("    {l}"), MUTED)));
            }
            lines
        })
        .collect();
    let ways_h: usize = ways.iter().map(Vec::len).sum();
    let warning: Vec<String> = if crate::autostart::is_on() {
        Vec::new()
    } else {
        wrap(
            "Autostart is off: a job moved to Herdr runs only while Herdr runs; kept as a cron job it always runs.",
            text_w,
        )
    };
    // The jobs get what is left, at least one row.
    let mut job_lines: Vec<Line> = Vec::new();
    for job in jobs {
        let when = match Schedule::parse(&job.schedule) {
            // In the crontab, `@reboot` means the computer's start.
            Ok(Schedule::AtStart) => "When the computer starts".to_string(),
            Ok(s) => s.describe(),
            Err(_) => job.schedule.clone(),
        };
        job_lines.push(clip_line(
            Line::from(vec![
                Span::styled(format!("{when}  "), bold(TEXT)),
                plain(job.command.clone(), MUTED),
            ]),
            text_w,
        ));
        if let Some(note) = &job.note {
            job_lines.push(clip_line(
                Line::from(plain(format!("  {note}"), YELLOW)),
                text_w,
            ));
        }
    }
    let fixed = 2
        + 1
        + ways_h
        + if warning.is_empty() {
            0
        } else {
            warning.len() + 1
        };
    let room = (stage.height as usize).saturating_sub(fixed).max(1);
    if job_lines.len() > room {
        let hidden = job_lines.len() - (room - 1);
        job_lines.truncate(room - 1);
        job_lines.push(Line::from(plain(
            format!("… and {hidden} more lines"),
            FAINT,
        )));
    }
    let height = (job_lines.len() + fixed) as u16;
    let area = centered(stage, width, height);
    let title = if jobs.len() == 1 {
        "Take over 1 cron job from your crontab".to_string()
    } else {
        format!("Take over {} cron jobs from your crontab", jobs.len())
    };
    let inner = dialog(frame, app, area, &title);
    let mut y = inner.y;
    let shown = job_lines.len() as u16;
    frame.render_widget(
        Paragraph::new(Text::from(job_lines)),
        Rect::new(inner.x, y, inner.width, shown.min(inner.height)),
    );
    y += shown + 1;
    for (i, lines) in ways.into_iter().enumerate() {
        let h = lines.len() as u16;
        if y + h > inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, h);
        if i == app.take_sel {
            frame.render_widget(Block::new().style(Style::new().bg(SELECTED_BG)), row);
        }
        frame.render_widget(Paragraph::new(Text::from(lines)), row);
        app.hits.push((row, Hit::Take(i)));
        y += h;
    }
    if !warning.is_empty() && y + 1 < inner.bottom() {
        let lines: Vec<Line> = warning
            .into_iter()
            .map(|l| Line::from(plain(l, YELLOW)))
            .collect();
        frame.render_widget(
            Paragraph::new(Text::from(lines)),
            Rect::new(inner.x, y + 1, inner.width, inner.bottom() - (y + 1)),
        );
    }
}

/// A run's output, or a routine to approve. How the run went and why
/// stays above the text; the keys are in the footer.
fn draw_log(frame: &mut Frame, app: &mut App, stage: Rect, mut log: LogView) {
    let area = centered(stage, stage.width.saturating_sub(4).min(140), stage.height);
    let title = log.title.clone();
    let mut inner = dialog(frame, app, area, &title);
    if !log.note.is_empty() && inner.height > 4 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                clip(&log.note, inner.width as usize),
                bold(TEXT),
            )),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        inner = Rect::new(inner.x, inner.y + 2, inner.width, inner.height - 2);
    }
    let visible = inner.height.saturating_sub(1);
    // Long lines wrap, so nothing hides past the right edge.
    let mut rows: Vec<String> = log
        .lines
        .iter()
        .flat_map(|l| hard_wrap(&l.replace('\t', "    "), inner.width as usize))
        .collect();
    // The view scrolls over at most 65,535 rows; of more, it shows the
    // end. A routine that long cannot be read in full here, so it cannot
    // be approved here either: `y` must never approve rows nobody could
    // scroll to.
    if rows.len() > u16::MAX as usize && log.approve.take().is_some() {
        let note = "It is too long to show in full (more than 65,535 rows), so it cannot be approved here. Shorten its prompt or command first.";
        log.lines.extend([String::new(), note.to_string()]);
        rows.push(String::new());
        rows.extend(hard_wrap(note, inner.width as usize));
        log.scroll = u16::MAX;
    }
    let over = rows.len().saturating_sub(u16::MAX as usize);
    let rows: Vec<String> = rows.into_iter().skip(over).collect();
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
    if log.approve.is_some() {
        if log.scroll >= log.max {
            let text = " y approve ";
            foot.push(Span::styled(
                text,
                Style::new()
                    .fg(theme::on(GREEN))
                    .bg(GREEN)
                    .add_modifier(Modifier::BOLD),
            ));
            app.hits
                .push((Rect::new(inner.x, y, text.width() as u16, 1), Hit::Key('y')));
        } else {
            foot.push(Span::styled("↓", bold(TEXT)));
            foot.push(plain(" read to the end to approve", MUTED));
        }
    }
    frame.render_widget(
        Paragraph::new(clip_line(Line::from(foot), inner.width as usize)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    if log.max > 0 {
        let position = format!(
            "lines {}–{} of {}",
            log.scroll + 1,
            (log.scroll + visible).min(total),
            total
        );
        let w = (position.width() as u16).min(inner.width);
        frame.render_widget(
            Paragraph::new(plain(position, MUTED)).alignment(Alignment::Right),
            Rect::new(inner.right() - w, y, w, 1),
        );
    }
    app.mode = Mode::Log(log);
}

/// A run on a page of its own: how it went and why, when, how long, what
/// started it, its result and the end of its output. What can be done
/// with it is in the footer.
fn draw_run_page(frame: &mut Frame, app: &mut App, icons: &Icons, stage: Rect, page: &RunPage) {
    let Some(run) = app.page_run(page).cloned() else {
        return;
    };
    const LABEL: usize = 12;
    let now = Local::now();
    // As wide as the output view; as tall as what it shows.
    let width = centered(stage, stage.width.saturating_sub(4).min(110), 1).width;
    let w = width.saturating_sub(4) as usize;
    let height = stage.height.saturating_sub(2) as usize;
    let title = format!(
        "{} · {}",
        run.routine_name,
        run.started.format("%a %-d %b %H:%M")
    );
    let room = w.saturating_sub(LABEL).max(10);
    let row = |label: &str, value: Vec<Span<'static>>| -> Line<'static> {
        let mut spans = vec![plain(format!("{label:<LABEL$}"), MUTED)];
        spans.extend(value);
        clip_line(Line::from(spans), w)
    };
    let (icon, color) = icons.status(run.status);
    let mut lines: Vec<Line> = vec![row(
        "Status",
        vec![Span::styled(
            format!("{icon} {}", run.status.label()),
            bold(color),
        )],
    )];
    // Why it ended that way, whole (up to three rows).
    let message = crate::app::visible_text(run.message.trim());
    if !message.is_empty() {
        for (i, l) in wrap(&message, room).into_iter().take(3).enumerate() {
            lines.push(row(
                if i == 0 { "Reason" } else { "" },
                vec![plain(l, TEXT)],
            ));
        }
    }
    // A skipped time never started: it has a time, but took none.
    let skipped = run.status == Status::Skipped;
    let mut started = vec![
        plain(run.started.format("%a %-d %b %Y %H:%M").to_string(), TEXT),
        plain(format!(" · {}", crate::app::ago(&run.started, &now)), FAINT),
    ];
    if skipped {
        started.push(plain(format!(" · {}", run.trigger.label()), FAINT));
    }
    lines.push(row(if skipped { "When" } else { "Started" }, started));
    if !skipped {
        if run.status.active() {
            let mut going = vec![plain(
                crate::app::span_words(run.awake_for(now).num_seconds()),
                TEXT,
            )];
            if run.asleep_secs > 0 {
                going.push(plain(
                    format!(
                        " · not counting {} the computer slept",
                        crate::app::span_words(i64::try_from(run.asleep_secs).unwrap_or(0))
                    ),
                    FAINT,
                ));
            }
            lines.push(row("Going for", going));
        } else {
            lines.push(row("Took", vec![plain(crate::app::took(&run), TEXT)]));
        }
        // What started it, for which time, and which try it is.
        let by = match run.trigger {
            crate::store::Trigger::Scheduled => "The schedule",
            crate::store::Trigger::Missed => "Catching up on a missed time",
            crate::store::Trigger::Manual => "You, by hand",
            crate::store::Trigger::Retry => "A retry after a run that failed",
            crate::store::Trigger::Start => "Herdr starting",
        };
        let mut why = vec![plain(by, TEXT)];
        if let Some(slot) = run.slot {
            if (run.started - slot).num_minutes().abs() >= 2 {
                why.push(plain(
                    format!(" · for {}", slot.format("%a %-d %b %H:%M")),
                    FAINT,
                ));
            }
        }
        if run.attempt > 0 {
            why.push(plain(format!(" · try {}", run.attempt + 1), FAINT));
        }
        lines.push(row("Started by", why));
    }
    // The result: as much as leaves room for the output.
    let result: Vec<String> = run
        .result
        .lines()
        .filter(|l| !l.trim().is_empty())
        .flat_map(|l| wrap(&crate::app::visible_text(l), room))
        .collect();
    let most = (height.saturating_sub(lines.len() + 4) / 2).clamp(1, 8);
    let label = if run.agent == crate::routine::SHELL {
        "Last line"
    } else {
        "Result"
    };
    for (i, l) in result.iter().take(most).enumerate() {
        let text = if i + 1 == most && result.len() > most {
            format!("{} …", clip(l, room.saturating_sub(2)))
        } else {
            l.clone()
        };
        lines.push(row(
            if i == 0 { label } else { "" },
            vec![plain(text, TEXT)],
        ));
    }
    // The end of the output fills the rest.
    let left = height.saturating_sub(lines.len());
    if left >= 3 {
        lines.push(Line::from(""));
        let rows: Vec<String> = page
            .tail
            .iter()
            .flat_map(|l| hard_wrap(&l.replace('\t', "    "), w))
            .collect();
        let fit = left - 2;
        let more = page.cut || rows.len() > fit;
        let head = if more { "End of the output" } else { "Output" };
        let note = if more { " l shows all of it" } else { "" };
        let rule = w.saturating_sub(head.width() + note.width() + 2);
        lines.push(Line::from(vec![
            Span::styled(head, bold(ACCENT)),
            plain(format!(" {} ", "─".repeat(rule)), BORDER),
            plain(note, FAINT),
        ]));
        if rows.is_empty() {
            let open = !run.tab_closed && run.cron_pid == 0 && !run.pane_id.is_empty();
            let why = if skipped {
                "Nothing ran at this time."
            } else if run.status.active() && open {
                "Nothing is saved yet: o goes to its tab, where it is at work."
            } else if run.status.active() {
                "Nothing is saved yet."
            } else {
                "No output was saved for this run."
            };
            lines.push(Line::from(plain(why, FAINT)));
        }
        let skip = rows.len().saturating_sub(fit);
        for l in rows.into_iter().skip(skip) {
            lines.push(Line::from(plain(l, TEXT)));
        }
    }
    let area = centered(stage, width, lines.len() as u16 + 2);
    let inner = dialog(frame, app, area, &title);
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Splits a line into rows of at most `width` columns and keeps every
/// character (unlike `wrap`, which joins words): it breaks after the last
/// space that fits, or inside a word that is longer than a row.
pub fn hard_wrap(text: &str, width: usize) -> Vec<String> {
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
            if line.width() + c.width().unwrap_or(0) > width {
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

/// Every key the list handles, by where it works. Keep it in step with
/// `App::key`, `list_key`, `form_key`, `editor_key` and the editor's own
/// keys: a key that is not here does not exist for the owner.
const HELP: [(&str, &[(&str, &str)]); 7] = [
    (
        "List",
        &[
            ("↑ ↓  j k", "move"),
            ("PgUp PgDn  g G", "a page up or down; first, last (also Home, End)"),
            ("← →  Tab  1 2 3", "routines / cron jobs / runs"),
            (
                "Mouse",
                "a click marks a row, a click on the marked row opens it; the wheel scrolls; a right click opens a menu; the keys in the footer and the buttons can be clicked; a click beside a dialog closes it",
            ),
            (
                "S",
                "all routines and cron jobs on or off (or click \"routines\" at the top)",
            ),
            (
                "A",
                "autostart: Herdr starts at login, so routines run after a restart, or not",
            ),
            (
                "W",
                "keep the computer awake: during runs, always, or not (or click \"awake\" at the top)",
            ),
            (
                "C",
                "take over the jobs that are only in your crontab (◇ under Cron jobs)",
            ),
            (
                "s",
                "sort the routines: what needs you first, or by name (the choice is kept)",
            ),
            ("i", "Nerd Font icons / plain symbols"),
            ("?", "these keys"),
            ("Esc", "close a dialog (never the list)"),
            ("q  Ctrl+C", "close the list (asks first)"),
        ],
    ),
    (
        "Routines and cron jobs",
        &[
            ("⏎  e", "edit"),
            ("n", "new: Herdr routine, cron job, or describe it to Claude"),
            ("p", "write the prompt or command"),
            (
                "r",
                "run now (asks first); a cron job runs in the background, as cron would",
            ),
            (
                "a",
                "approve one that is new or was changed outside the list: shows it, y approves at the end",
            ),
            ("Space", "switch on / off"),
            ("c", "duplicate (the copy starts switched off)"),
            ("o", "go to the tab of its last run"),
            ("l", "output of its last run"),
            ("x", "stop its run that is going (asks first)"),
            (
                "t",
                "chat about its last run: a tab with Claude that reads the run and answers, or changes the routine",
            ),
            ("h", "its runs (or click \"n runs ›\"); Esc comes back"),
            (
                "Line on top",
                "what needs you, what is running and what comes next; a click goes there",
            ),
            ("d  Del", "delete (asks: keep its runs, or delete them too)"),
            (
                "⏎ on ◇",
                "take over a job that is only in your crontab: k keep as a cron job, m move to Herdr, c copy",
            ),
        ],
    ),
    (
        "Runs",
        &[
            (
                "⏎",
                "open the run: how it went and why, its result and the end of its output; the keys below work there too",
            ),
            ("o", "go to the run's tab, or its output once the tab is closed"),
            ("l", "the whole output of the run"),
            ("f", "open the output file"),
            ("r", "run its routine again (asks first)"),
            ("x", "stop the run while it is going (asks first)"),
            (
                "t",
                "chat about the run once it ended: a tab with Claude that reads its record and output, answers your questions and changes the routine when you ask (a change waits for a)",
            ),
            ("h", "only its runs; again for all runs"),
            ("p", "only runs that went wrong or need you; again for all"),
            ("/", "search: the routine's name, the message or the result"),
            (
                "Esc",
                "remove the filters: back to the routine, or all runs (✕ removes one)",
            ),
            ("d  Del", "delete the run (asks first)"),
        ],
    ),
    (
        "Form",
        &[
            ("↑ ↓  Tab", "field (PgUp, PgDn: several)"),
            (
                "⏎  Space",
                "choose from the list, write the prompt, switch a weekday, show or hide More settings, save",
            ),
            (
                "← →",
                "next / previous choice; in a text field they move the cursor, on the weekdays from day to day",
            ),
            (
                "1–7",
                "on the weekdays: switch Monday (1) to Sunday (7)",
            ),
            (
                "Home  End",
                "start / end of the text; on a choice, first / last field",
            ),
            ("Backspace  Del", "delete before / at the cursor"),
            ("Ctrl+U", "clear the field"),
            ("Ctrl+S", "save"),
            ("Esc", "back (asks when something changed)"),
        ],
    ),
    (
        "Prompt editor",
        &[
            (
                "Esc  Ctrl+S",
                "done: keeps what you wrote (a routine's prompt is saved at once)",
            ),
            ("Ctrl+D", "discard what you wrote (asks first)"),
            ("Ctrl+Z  Ctrl+Y", "undo / redo"),
            ("arrows", "move; with Ctrl or Alt by word; ⏎ starts a new line"),
            (
                "Home  End",
                "start / end of the line (Ctrl+E: end); with Ctrl of the text",
            ),
            ("PgUp  PgDn", "a page up or down"),
            ("Shift+arrows", "mark text; or drag with the mouse"),
            ("Double click", "mark a word"),
            ("Ctrl+A", "mark everything"),
            ("Ctrl+C  X  V", "copy, cut, paste"),
            ("Ctrl+W", "delete the word before the cursor"),
            ("Ctrl+O", "open in another editor ($VISUAL, $EDITOR)"),
        ],
    ),
    (
        "Output and approval",
        &[
            (
                "↑ ↓  PgUp PgDn",
                "scroll (Space: a page down; g, G: start, end)",
            ),
            ("f", "open the output file"),
            ("y", "approve, once read to the end"),
            ("Esc  ⏎  q", "close"),
        ],
    ),
    (
        "Lists to choose from and questions",
        &[
            ("↑ ↓  ⏎", "mark and choose; a click chooses"),
            ("1–9", "choose by number"),
            ("typing", "narrows a long list (such as the agents)"),
            (
                "y",
                "yes; ⏎ too where nothing is lost. Deleting, discarding, stopping a run and running by hand need y",
            ),
            ("r", "when deleting a routine: delete its runs too"),
            ("any other key", "no"),
        ],
    ),
];

/// The width of the keys in the help.
const HELP_KEYS: usize = 17;

/// The keys as lines that fit `width`: a long explanation wraps under
/// itself, so none is cut at the edge.
fn help_lines(width: usize) -> Vec<Line<'static>> {
    let room = width.saturating_sub(HELP_KEYS).max(10);
    let mut lines: Vec<Line> = Vec::new();
    for (gi, (title, keys)) in HELP.iter().enumerate() {
        if gi > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(*title, bold(ACCENT))));
        for (k, what) in keys.iter() {
            for (n, part) in wrap(what, room).into_iter().enumerate() {
                let key = if n == 0 { *k } else { "" };
                lines.push(Line::from(vec![
                    Span::styled(format!("{key:<HELP_KEYS$}"), bold(TEXT)),
                    plain(part, MUTED),
                ]));
            }
        }
    }
    lines
}

fn draw_help(frame: &mut Frame, app: &mut App, stage: Rect) {
    let how = |width: usize| -> Vec<Line<'static>> {
        let mut out = vec![Line::from(Span::styled("How it works", bold(ACCENT)))];
        for (i, p) in INTRO.iter().enumerate() {
            if i > 0 {
                out.push(Line::from(""));
            }
            for l in wrap(p, width) {
                out.push(Line::from(plain(l, MUTED)));
            }
        }
        out
    };
    let width = 78u16.min(stage.width.saturating_sub(2)).max(24);
    let text_w = width.saturating_sub(4) as usize;
    let mut lines = help_lines(text_w);
    let one_column = how(text_w);
    // In a wide window that is too low for one column, the note on how it
    // works goes beside the keys.
    let two_columns =
        stage.width >= 124 && (stage.height as usize) < lines.len() + 1 + one_column.len() + 2;
    let (area, columns): (Rect, Vec<Vec<Line<'static>>>) = if two_columns {
        lines = help_lines(64);
        let right = how(52);
        let height = lines.len().max(right.len()) as u16 + 2;
        (centered(stage, 124, height), vec![lines, right])
    } else {
        lines.push(Line::from(""));
        lines.extend(one_column);
        let height = lines.len() as u16 + 2;
        (centered(stage, width, height), vec![lines])
    };
    let inner = dialog(frame, app, area, "Keys");
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
            Paragraph::new(plain(hint, MUTED)),
            Rect::new(inner.x, inner.y + visible, inner.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_words_whole() {
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

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn what_is_cut_ends_in_an_ellipsis() {
        assert_eq!(
            clip("Monthly on the 1st at 09:00", 26).chars().last(),
            Some('…')
        );
        assert_eq!(clip("short", 26), "short");
        assert_eq!(clip_start("~/code/site/docs", 10), "…site/docs");
        // A line of several colours keeps them and gets the ellipsis in
        // the colour of the part that was cut.
        let line = Line::from(vec![plain("✗ Failed", RED), plain("  13 runs ›", ACCENT)]);
        let cut = clip_line(line.clone(), 12);
        assert_eq!(text(&cut), "✗ Failed  1…");
        assert_eq!(cut.width(), 12);
        assert_eq!(cut.spans.last().unwrap().style.fg, Some(ACCENT));
        assert_eq!(text(&clip_line(line, 40)), "✗ Failed  13 runs ›");
    }

    #[test]
    fn a_text_field_shows_the_part_around_its_cursor() {
        let value = "~/projects/a-very-long-folder/with/many/parts/inside";
        let shown = |cursor| -> String {
            text_value(value, Some(cursor), 20, false)
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };
        // At the end: the end shows, cut at its start.
        let end = shown(value.chars().count());
        assert!(end.starts_with('…') && end.ends_with("inside▏"), "{end}");
        // At the start: the start shows, cut at its end.
        let start = shown(0);
        assert!(
            start.starts_with("▏~/projects") && start.ends_with('…'),
            "{start}"
        );
        // In the middle: cut at both ends, the cursor in it.
        let middle = shown(25);
        assert!(
            middle.starts_with('…') && middle.ends_with('…') && middle.contains('▏'),
            "{middle}"
        );
        for s in [end, start, middle] {
            assert!(s.width() <= 20, "{s}");
        }
        // Without the cursor a folder keeps its end.
        let folder: String = text_value(value, None, 20, true)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            folder.starts_with('…') && folder.ends_with("inside"),
            "{folder}"
        );
    }

    #[test]
    fn the_list_of_routines_takes_the_shape_that_fits() {
        // Wide: everything side by side, with wide gaps.
        let (shape, gap, name, sched) = routine_shape(118, 34, 40, 25);
        assert_eq!((shape, gap), (Shape::Full, 3));
        assert_eq!(name + sched, 118 - 2 - 9 - 25);
        assert_eq!(sched, 40);
        // 80 columns: still all three, with narrow gaps and a shorter
        // schedule column.
        let (shape, gap, name, sched) = routine_shape(78, 34, 40, 25);
        assert_eq!((shape, gap), (Shape::Full, 1));
        assert!(name >= NAME_MIN && sched >= 22, "{name} {sched}");
        // A split pane: the last run moves into the routine's second line.
        assert_eq!(routine_shape(58, 34, 40, 25).0, Shape::Compact);
        // Very narrow: the routine alone.
        assert_eq!(routine_shape(38, 34, 40, 25).0, Shape::Narrow);
    }

    #[test]
    fn the_header_drops_things_in_order_and_keeps_the_warning() {
        let chip = |keep: bool| Chip {
            long: "● chip on".into(),
            short: "● chip".into(),
            style: Style::new(),
            key: None,
            keep,
        };
        let all = vec![chip(false), chip(false), chip(true)];
        let plans = plans(&all);
        // First everything, then without the title.
        assert_eq!((plans[0].title, plans[0].chips.len()), (2, 3));
        assert_eq!((plans[1].title, plans[1].chips[0].1), (1, false));
        // Then the chips that are no warning get short, and go from the
        // left; the warning stays as it is.
        assert_eq!(plans[2].chips, vec![(0, true), (1, true), (2, false)]);
        assert_eq!(plans[3].chips, vec![(1, true), (2, false)]);
        assert_eq!(plans[4].chips, vec![(2, false)]);
        // Then the tabs get shorter, and only at the very end the warning
        // does. It never goes.
        for p in &plans[4..plans.len() - 1] {
            assert_eq!(p.chips, vec![(2, false)]);
        }
        let last = plans.last().unwrap();
        assert_eq!((last.title, last.tabs), (0, 2));
        assert_eq!(last.chips, vec![(2, true)]);
        assert!(plans.windows(2).all(|w| w[0].tabs <= w[1].tabs));
        // A chip that drops in later (a third one) takes its turn by its
        // place in the list.
        let four = vec![chip(false), chip(false), chip(false), chip(true)];
        let plans = super::plans(&four);
        assert_eq!(plans[3].chips.len(), 3);
        assert_eq!(plans[5].chips, vec![(3, false)]);
        // The help lists a key once per place, and wraps instead of
        // cutting.
        for line in help_lines(40) {
            assert!(line.width() <= 40, "{}", text(&line));
        }
    }
}
