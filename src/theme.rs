//! Colours and icons, the same violet look as the other Herdr plugins of
//! this author.

use crate::store::Status;
use ratatui::style::Color;

pub const TEXT: Color = Color::Rgb(0xE4, 0xE4, 0xEF);
pub const MUTED: Color = Color::Rgb(0xA6, 0xAB, 0xCF);
pub const FAINT: Color = Color::Rgb(0x7A, 0x7F, 0xA3);
pub const BORDER: Color = Color::Rgb(0x3A, 0x3E, 0x5C);
pub const SELECTED_BG: Color = Color::Rgb(0x30, 0x33, 0x55);
pub const ACCENT: Color = Color::Rgb(0x9D, 0x8C, 0xF7);
pub const GREEN: Color = Color::Rgb(0x3F, 0xD2, 0x8A);
pub const RED: Color = Color::Rgb(0xF0, 0x56, 0x6D);
pub const YELLOW: Color = Color::Rgb(0xE8, 0xB9, 0x4E);
pub const BLUE: Color = Color::Rgb(0x6C, 0xA8, 0xF0);
pub const BAR_BG: Color = Color::Rgb(0x25, 0x28, 0x3E);

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

    pub fn status(&self, s: Status) -> (&'static str, Color) {
        match s {
            Status::Starting => (self.pick("\u{f43a}", "…"), YELLOW),
            Status::Running => (self.pick("\u{f46a}", "…"), BLUE),
            Status::NeedsYou => (self.pick("\u{f420}", "?"), YELLOW),
            Status::Finished => (self.pick("\u{f4a4}", "✓"), GREEN),
            Status::TimedOut => (self.pick("\u{f4e3}", "⧗"), RED),
            Status::Failed => (self.pick("\u{f530}", "✗"), RED),
            Status::Skipped => (self.pick("\u{f517}", "⊘"), MUTED),
            Status::Closed => (self.pick("\u{f468}", "⊘"), MUTED),
        }
    }

    /// On (green), off (red), or broken (yellow warning).
    pub fn routine(&self, enabled: bool, broken: bool) -> (&'static str, Color) {
        if broken {
            (self.pick("\u{f421}", "!"), YELLOW)
        } else if enabled {
            (self.pick("\u{f444}", "●"), GREEN)
        } else {
            (self.pick("\u{f444}", "●"), RED)
        }
    }

    pub fn tab_open(&self) -> &'static str {
        self.pick("\u{f524}", "▸")
    }

    /// In front of the title.
    pub fn title(&self) -> &'static str {
        self.pick("\u{f455}", "◷")
    }

    pub fn warning(&self) -> &'static str {
        self.pick("\u{f421}", "!")
    }

    /// The scheduler runs.
    pub fn running(&self) -> &'static str {
        self.pick("\u{f444}", "●")
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
