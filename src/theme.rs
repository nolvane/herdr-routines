//! Colours and icons, the same violet look as the other Herdr plugins of
//! this author.

use crate::store::Status;
use ratatui::style::Color;

pub const TEXT: Color = Color::Rgb(0xE4, 0xE4, 0xEF);
pub const MUTED: Color = Color::Rgb(0xA6, 0xAB, 0xCF);
pub const FAINT: Color = Color::Rgb(0x7A, 0x7F, 0xA3);
pub const BORDER: Color = Color::Rgb(0x3A, 0x3E, 0x5C);
pub const SELECTED_BG: Color = Color::Rgb(0x30, 0x33, 0x55);
/// The row under the mouse.
pub const HOVER_BG: Color = Color::Rgb(0x26, 0x28, 0x42);
pub const ACCENT: Color = Color::Rgb(0x9D, 0x8C, 0xF7);
pub const GREEN: Color = Color::Rgb(0x3F, 0xD2, 0x8A);
pub const RED: Color = Color::Rgb(0xF0, 0x56, 0x6D);
pub const YELLOW: Color = Color::Rgb(0xE8, 0xB9, 0x4E);
pub const BLUE: Color = Color::Rgb(0x6C, 0xA8, 0xF0);
pub const BAR_BG: Color = Color::Rgb(0x25, 0x28, 0x3E);
/// Marked text in the prompt editor.
pub const MARK_BG: Color = Color::Rgb(0x4A, 0x40, 0x8C);

/// What the mark in front of a routine says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    On,
    Off,
    Problem,
    Waiting,
}

pub struct Icons {
    pub nerd: bool,
}

impl Icons {
    fn pick(&self, nerd: &'static str, plain: &'static str) -> &'static str {
        if self.nerd {
            nerd
        } else {
            plain
        }
    }

    /// A run's status. Every status has its own symbol, also without the
    /// Nerd Font, so none is told apart by colour alone.
    pub fn status(&self, s: Status) -> (&'static str, Color) {
        match s {
            Status::Starting => (self.pick("\u{f43a}", "…"), BLUE),
            Status::Running => (self.pick("\u{f46a}", "↻"), BLUE),
            Status::NeedsYou => (self.pick("\u{f420}", "?"), YELLOW),
            Status::Finished => (self.pick("\u{f4a4}", "✓"), GREEN),
            Status::TimedOut => (self.pick("\u{f4e3}", "⧗"), RED),
            Status::Failed => (self.pick("\u{f530}", "✗"), RED),
            Status::Skipped => (self.pick("\u{f517}", "⊘"), MUTED),
            Status::Closed => (self.pick("\u{f468}", "■"), MUTED),
            Status::NothingToDo => (self.pick("\u{f4a7}", "–"), MUTED),
            Status::NoResult => (self.pick("\u{f421}", "!"), YELLOW),
        }
    }

    /// A routine's mark: on is a full dot, off a hollow one (shape, not
    /// only colour), a problem a red warning sign, and one that waits for
    /// the owner's approval its own yellow sign.
    pub fn routine(&self, mark: Mark) -> (&'static str, Color) {
        match mark {
            Mark::On => (self.pick("\u{f444}", "●"), GREEN),
            Mark::Off => ("○", FAINT),
            Mark::Problem => (self.pick("\u{f421}", "!"), RED),
            Mark::Waiting => (self.pick("\u{f420}", "◐"), YELLOW),
        }
    }

    pub fn tab_open(&self) -> &'static str {
        self.pick("\u{f524}", "▸")
    }

    /// In front of the title.
    pub fn title(&self) -> &'static str {
        self.pick("\u{f455}", "◷")
    }

    /// A chip in the header that is on.
    pub fn running(&self) -> &'static str {
        self.pick("\u{f444}", "●")
    }

    /// A chip in the header that is off: hollow, in both icon sets.
    pub fn stopped(&self) -> &'static str {
        "○"
    }
}

/// Text colour that reads on a coloured background.
pub fn on(bg: Color) -> Color {
    match bg {
        Color::Rgb(r, g, b) => {
            let l = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            if l > 150.0 {
                Color::Rgb(0x16, 0x17, 0x22)
            } else {
                Color::Rgb(0xF5, 0xF5, 0xFA)
            }
        }
        _ => TEXT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_state_has_a_symbol_of_its_own() {
        for nerd in [true, false] {
            let icons = Icons { nerd };
            let statuses = [
                Status::Starting,
                Status::Running,
                Status::NeedsYou,
                Status::Finished,
                Status::TimedOut,
                Status::Failed,
                Status::Skipped,
                Status::Closed,
                Status::NothingToDo,
            ];
            let signs: HashSet<&str> = statuses.iter().map(|s| icons.status(*s).0).collect();
            assert_eq!(signs.len(), statuses.len(), "nerd {nerd}: {signs:?}");
            // On and off, a problem and waiting for approval differ by
            // shape, not only by colour.
            let marks = [Mark::On, Mark::Off, Mark::Problem, Mark::Waiting];
            let signs: HashSet<&str> = marks.iter().map(|m| icons.routine(*m).0).collect();
            assert_eq!(signs.len(), marks.len(), "nerd {nerd}: {signs:?}");
            assert_ne!(icons.running(), icons.stopped());
        }
    }
}
