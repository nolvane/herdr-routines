//! Telling the owner about a run: a Herdr notification, or for routines
//! that ask for it a popup over whatever is in front in Herdr, which stays
//! until it is answered.
//!
//! The popup is a plugin pane (`alert` in `herdr-plugin.toml`) opened with
//! Herdr's `popup` placement. It gets the routine and run ids and its texts
//! through `HERDR_ROUTINES_ALERT`; "Go to its tab" loads the run record and
//! goes through `runner::show`, which checks that the tab is still the
//! run's own.

use crate::herdr;
use crate::paths::Paths;
use crate::routine::Routine;
use crate::store::Run;

const ENV: &str = "HERDR_ROUTINES_ALERT";
const WIDTH: u16 = 64;

/// Tells the owner about a run: a popup when the routine asks for one
/// (`problem`: it failed, ran too long or needs an answer), otherwise, or
/// when the popup cannot open, a Herdr notification. `notify` false: no
/// notification either way (a finished run without `notify_finished`), only
/// the popup a routine asks for.
pub fn tell(paths: &Paths, run: &Run, problem: bool, title: &str, body: &str, notify: bool) {
    let alert = Routine::load(paths, &run.routine_id)
        .map(|r| r.file.alert)
        .unwrap_or_default();
    if alert.popup(problem) && open(run, title, body) {
        return;
    }
    if notify {
        herdr::notify(title, body, if problem { "request" } else { "done" });
    }
}

/// The popup's size for `body`: border, title, a gap, the text, a gap, the
/// buttons, border.
fn size(body: &str) -> (u16, u16) {
    let lines = crate::ui::wrap(body, (WIDTH - 4) as usize).len() as u16;
    (WIDTH, lines + 6)
}

fn one_line(s: &str) -> String {
    crate::app::visible_text(&s.replace(['\t', '\n', '\r'], " "))
}

fn open(run: &Run, title: &str, body: &str) -> bool {
    let value = format!(
        "{}\t{}\t{}\t{}",
        run.routine_id,
        run.id,
        one_line(title),
        one_line(body)
    );
    let (width, height) = size(body);
    herdr::open_popup("alert", width, height, &[(ENV, &value)])
}

/// `herdr-routines alert-pane`: the popup itself.
pub fn pane(paths: &Paths) -> i32 {
    let value = std::env::var(ENV).unwrap_or_default();
    let parts: Vec<&str> = value.splitn(4, '\t').collect();
    let [routine_id, run_id, title, body] = parts.as_slice() else {
        return 2;
    };
    if !crate::routine::valid_id(routine_id) {
        return 2;
    }
    let run = crate::store::runs_of(paths, routine_id)
        .into_iter()
        .find(|r| r.id == *run_id);
    let mut terminal = crate::enter_terminal();
    let go = dialog(&mut terminal, title, body, run.is_some()).unwrap_or(false);
    crate::leave_terminal();
    if go {
        if let Some(run) = run {
            if crate::runner::show(paths, &run).is_err() {
                herdr::notify(
                    &format!("{} has no open tab", run.routine_name),
                    "Its output is in herdr-routines (l on the run).",
                    "none",
                );
            }
        }
    }
    0
}

/// Draws the popup until it is answered: true for "Go to its tab".
fn dialog(
    terminal: &mut ratatui::DefaultTerminal,
    title: &str,
    body: &str,
    can_go: bool,
) -> std::io::Result<bool> {
    use ratatui::crossterm::event::{
        self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
    };
    use ratatui::layout::Rect;

    let mut ok_area = Rect::default();
    let mut go_area = Rect::default();
    loop {
        terminal.draw(|f| (ok_area, go_area) = draw(f, title, body, can_go))?;
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(' ') => {
                    return Ok(false)
                }
                KeyCode::Char('g') if can_go => return Ok(true),
                _ => {}
            },
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                let inside = |r: Rect| {
                    m.column >= r.x
                        && m.column < r.x + r.width
                        && m.row >= r.y
                        && m.row < r.y + r.height
                };
                if inside(ok_area) {
                    return Ok(false);
                }
                if can_go && inside(go_area) {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }
}

/// The popup's inside (Herdr draws its border); returns where the OK and
/// "Go to its tab" buttons are.
fn draw(
    f: &mut ratatui::Frame,
    title: &str,
    body: &str,
    can_go: bool,
) -> (ratatui::layout::Rect, ratatui::layout::Rect) {
    use crate::theme::{ACCENT, BORDER, MUTED, TEXT};
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::{Line, Span, Text};
    use ratatui::widgets::{Clear, Paragraph};

    let area = f.area();
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    );
    let mut text = vec![
        Line::from(Span::styled(
            title.to_string(),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for l in crate::ui::wrap(body, inner.width as usize) {
        text.push(Line::from(Span::styled(l, Style::new().fg(TEXT))));
    }
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(Text::from(text)),
        Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(2),
        ),
    );
    let y = inner.y + inner.height.saturating_sub(1);
    let ok = " ⏎ OK ";
    let go = " g Go to its tab ";
    let ok_area = Rect::new(inner.x, y, ok.chars().count() as u16, 1);
    let mut go_area = Rect::default();
    let mut spans = vec![Span::styled(
        ok,
        Style::new()
            .fg(Color::Rgb(0x1A, 0x1B, 0x2E))
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD),
    )];
    if can_go {
        go_area = Rect::new(
            ok_area.x + ok_area.width + 1,
            y,
            go.chars().count() as u16,
            1,
        );
        spans.push(Span::raw(" "));
        spans.push(Span::styled(go, Style::new().fg(MUTED).bg(BORDER)));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    (ok_area, go_area)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_popup_shows_its_text_and_buttons() {
        use ratatui::backend::TestBackend;
        let body = "The command ended with exit code 1. Its tab stays open.";
        // Herdr's border takes two rows and two columns.
        let (w, h) = size(body);
        let mut t = ratatui::Terminal::new(TestBackend::new(w - 2, h - 2)).unwrap();
        let mut areas = Default::default();
        t.draw(|f| areas = draw(f, "Nightly backup failed", body, true))
            .unwrap();
        let buf = t.backend().buffer();
        let lines: Vec<String> = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect();
        assert!(lines[0].contains("Nightly backup failed"));
        assert!(lines.iter().any(|l| l.contains("exit code 1")));
        let last = lines.last().unwrap();
        assert!(last.contains("⏎ OK") && last.contains("g Go to its tab"));
        assert_eq!(areas.0.y, h - 3);
    }

    #[test]
    fn popup_fits_its_text() {
        let (w, h) = size("short");
        assert_eq!((w, h), (WIDTH, 7));
        let long = "word ".repeat(40);
        assert!(size(&long).1 > 7);
        assert_eq!(one_line("a\tb\nc"), "a b c");
    }
}
