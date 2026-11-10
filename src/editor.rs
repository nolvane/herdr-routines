//! The prompt editor: a small text area typed into like a note app. Long
//! lines wrap at word boundaries; the arrows, Home and End move on the lines
//! as they show on screen.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

/// The width a tab takes on screen.
const TAB: usize = 4;
/// Undo steps kept.
const UNDO: usize = 200;

/// The width of a character on screen.
pub fn char_width(c: char) -> usize {
    if c == '\t' {
        TAB
    } else {
        c.width().unwrap_or(0)
    }
}

/// Where a logical line breaks on screen: `(start, end)` character ranges,
/// at most `width` wide, broken after a space where there is one.
pub fn segments(line: &[char], width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    if line.is_empty() {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < line.len() {
        let mut w = 0;
        let mut end = start;
        while end < line.len() {
            let cw = char_width(line[end]);
            if w + cw > width && end > start {
                break;
            }
            w += cw;
            end += 1;
        }
        if end < line.len() {
            if let Some(brk) = (start + 1..=end).rev().find(|&i| line[i - 1] == ' ') {
                end = brk;
            }
        }
        out.push((start, end));
        start = end;
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Last {
    Typing,
    Deleting,
    Other,
}

type Snapshot = (Vec<Vec<char>>, usize, usize);

#[derive(Debug, Clone)]
pub struct TextArea {
    pub lines: Vec<Vec<char>>,
    pub row: usize,
    pub col: usize,
    /// The first screen row shown.
    pub top: usize,
    /// The size of the text on screen, set when drawn.
    pub width: usize,
    pub height: usize,
    /// The screen column kept while moving up and down.
    want_x: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Last,
}

impl TextArea {
    pub fn new(text: &str) -> TextArea {
        let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        TextArea {
            lines,
            row: 0,
            col: 0,
            top: 0,
            // Set when drawn; 0 makes the first drawing scroll to the cursor.
            width: 0,
            height: 20,
            want_x: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last: Last::Other,
        }
    }

    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// New text from outside (the person's own editor), as one undo step.
    pub fn replace(&mut self, text: &str) {
        self.save_undo(Last::Other);
        self.lines = text.split('\n').map(|l| l.chars().collect()).collect();
        self.go_to_end();
        self.top = 0;
        self.follow();
    }

    /// Puts the cursor after the last character.
    pub fn go_to_end(&mut self) {
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].len();
    }

    // ---- layout ---------------------------------------------------------

    /// Every screen row: `(line, start, end)`.
    pub fn rows(&self) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for (i, l) in self.lines.iter().enumerate() {
            for (s, e) in segments(l, self.width) {
                out.push((i, s, e));
            }
        }
        out
    }

    /// The screen row and column of the cursor.
    pub fn cursor(&self) -> (usize, usize) {
        let rows = self.rows();
        let mut found = 0;
        for (i, (line, s, e)) in rows.iter().enumerate() {
            if *line != self.row {
                continue;
            }
            let last = rows.get(i + 1).is_none_or(|(l, _, _)| *l != self.row);
            if self.col >= *s && (self.col < *e || last) {
                let x = self.lines[*line][*s..self.col]
                    .iter()
                    .map(|c| char_width(*c))
                    .sum();
                return (i, x);
            }
            found = i;
        }
        (found, 0)
    }

    /// Scrolls so the cursor is on screen.
    pub fn follow(&mut self) {
        let (y, _) = self.cursor();
        let h = self.height.max(1);
        if y < self.top {
            self.top = y;
        } else if y >= self.top + h {
            self.top = y + 1 - h;
        }
        let total = self.rows().len();
        self.top = self.top.min(total.saturating_sub(1));
    }

    pub fn scroll(&mut self, by: i32) {
        let total = self.rows().len();
        let max = total.saturating_sub(self.height.max(1));
        self.top = (self.top as i32 + by).clamp(0, max as i32) as usize;
    }

    /// Moves the cursor to a spot on screen (a click), relative to the text.
    pub fn place(&mut self, y: usize, x: usize) {
        let rows = self.rows();
        let Some(&(line, s, e)) = rows.get(self.top + y).or(rows.last()) else {
            return;
        };
        self.row = line;
        self.col = self.col_at(line, s, e, x, rows.get(self.top + y + 1));
        self.want_x = None;
        self.last = Last::Other;
    }

    /// The character index at screen column `x` of the row `(s, e)`.
    fn col_at(
        &self,
        line: usize,
        s: usize,
        e: usize,
        x: usize,
        next: Option<&(usize, usize, usize)>,
    ) -> usize {
        let chars = &self.lines[line];
        let mut w = 0;
        let mut col = s;
        while col < e {
            let cw = char_width(chars[col]);
            if w + cw > x {
                break;
            }
            w += cw;
            col += 1;
        }
        // The end of a wrapped row belongs to the next row: stay before it.
        let wrapped = next.is_some_and(|(l, _, _)| *l == line);
        if col == e && wrapped && e > s {
            col - 1
        } else {
            col
        }
    }

    fn vertical(&mut self, by: i32) {
        let rows = self.rows();
        let (y, x) = self.cursor();
        let want = *self.want_x.get_or_insert(x);
        let target = y as i32 + by;
        if target < 0 {
            self.row = 0;
            self.col = 0;
        } else if target as usize >= rows.len() {
            self.go_to_end();
        } else {
            let t = target as usize;
            let (line, s, e) = rows[t];
            self.row = line;
            self.col = self.col_at(line, s, e, want, rows.get(t + 1));
        }
        self.last = Last::Other;
    }

    fn home(&mut self) {
        let rows = self.rows();
        let (y, _) = self.cursor();
        self.col = rows[y].1;
        self.want_x = None;
    }

    fn end(&mut self) {
        let rows = self.rows();
        let (y, _) = self.cursor();
        let (line, s, e) = rows[y];
        self.col = self.col_at(line, s, e, usize::MAX, rows.get(y + 1));
        self.want_x = None;
    }

    // ---- editing ----------------------------------------------------------

    fn save_undo(&mut self, kind: Last) {
        if kind == Last::Other || kind != self.last {
            self.undo.push((self.lines.clone(), self.row, self.col));
            if self.undo.len() > UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.last = kind;
        self.want_x = None;
    }

    pub fn insert(&mut self, c: char) {
        if c == '\n' {
            return self.newline();
        }
        // A new word starts a new undo step.
        if c == ' ' {
            self.last = Last::Other;
        }
        self.save_undo(Last::Typing);
        self.lines[self.row].insert(self.col, c);
        self.col += 1;
    }

    /// Pasted text, as one undo step.
    pub fn paste(&mut self, text: &str) {
        self.save_undo(Last::Other);
        for c in text.chars() {
            match c {
                '\r' => {}
                '\n' => {
                    let rest = self.lines[self.row].split_off(self.col);
                    self.lines.insert(self.row + 1, rest);
                    self.row += 1;
                    self.col = 0;
                }
                c if c.is_control() && c != '\t' => {}
                c => {
                    self.lines[self.row].insert(self.col, c);
                    self.col += 1;
                }
            }
        }
        self.last = Last::Other;
    }

    pub fn newline(&mut self) {
        self.save_undo(Last::Other);
        let rest = self.lines[self.row].split_off(self.col);
        self.lines.insert(self.row + 1, rest);
        self.row += 1;
        self.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.col == 0 && self.row == 0 {
            return;
        }
        self.save_undo(Last::Deleting);
        if self.col > 0 {
            self.col -= 1;
            self.lines[self.row].remove(self.col);
        } else {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].extend(line);
        }
    }

    pub fn delete(&mut self) {
        let len = self.lines[self.row].len();
        if self.col == len && self.row + 1 == self.lines.len() {
            return;
        }
        self.save_undo(Last::Deleting);
        if self.col < len {
            self.lines[self.row].remove(self.col);
        } else {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].extend(next);
        }
    }

    /// Deletes the word before the cursor.
    fn delete_word(&mut self) {
        let start = self.word_left_col();
        if start == self.col {
            return self.backspace();
        }
        self.save_undo(Last::Other);
        self.lines[self.row].drain(start..self.col);
        self.col = start;
    }

    pub fn undo(&mut self) {
        if let Some((lines, row, col)) = self.undo.pop() {
            self.redo.push((
                std::mem::replace(&mut self.lines, lines),
                self.row,
                self.col,
            ));
            self.row = row;
            self.col = col;
        }
        self.last = Last::Other;
    }

    pub fn redo(&mut self) {
        if let Some((lines, row, col)) = self.redo.pop() {
            self.undo.push((
                std::mem::replace(&mut self.lines, lines),
                self.row,
                self.col,
            ));
            self.row = row;
            self.col = col;
        }
        self.last = Last::Other;
    }

    fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].len();
        }
    }

    fn right(&mut self) {
        if self.col < self.lines[self.row].len() {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn word_left_col(&self) -> usize {
        let l = &self.lines[self.row];
        let mut c = self.col;
        while c > 0 && l[c - 1].is_whitespace() {
            c -= 1;
        }
        while c > 0 && !l[c - 1].is_whitespace() {
            c -= 1;
        }
        c
    }

    fn word_left(&mut self) {
        if self.col == 0 {
            return self.left();
        }
        self.col = self.word_left_col();
    }

    fn word_right(&mut self) {
        let l = &self.lines[self.row];
        if self.col == l.len() {
            return self.right();
        }
        let mut c = self.col;
        while c < l.len() && !l[c].is_whitespace() {
            c += 1;
        }
        while c < l.len() && l[c].is_whitespace() {
            c += 1;
        }
        self.col = c;
    }

    /// The editing keys. Returns false for keys it does not use, which the
    /// list handles (save, close).
    pub fn key(&mut self, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let page = self.height.max(2) as i32 - 1;
        match k.code {
            KeyCode::Char('z') if ctrl => self.undo(),
            KeyCode::Char('y') if ctrl => self.redo(),
            KeyCode::Char('a') if ctrl => self.home(),
            KeyCode::Char('e') if ctrl => self.end(),
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Char('b') if alt => self.word_left(),
            KeyCode::Char('f') if alt => self.word_right(),
            KeyCode::Char(_) if ctrl || alt => return false,
            KeyCode::Char(c) => self.insert(c),
            KeyCode::Enter => self.newline(),
            KeyCode::Tab => self.insert('\t'),
            KeyCode::Backspace if ctrl || alt => self.delete_word(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Left if ctrl || alt => self.word_left(),
            KeyCode::Right if ctrl || alt => self.word_right(),
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Up => return self.moved(-1),
            KeyCode::Down => return self.moved(1),
            KeyCode::PageUp => return self.moved(-page),
            KeyCode::PageDown => return self.moved(page),
            KeyCode::Home if ctrl => {
                self.row = 0;
                self.col = 0;
            }
            KeyCode::End if ctrl => self.go_to_end(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            _ => return false,
        }
        // Only up and down keep the column (they returned above).
        self.want_x = None;
        let editing = !ctrl
            && !alt
            && matches!(
                k.code,
                KeyCode::Char(_) | KeyCode::Tab | KeyCode::Backspace | KeyCode::Delete
            );
        if !editing {
            // Typing after moving starts a new undo step.
            self.last = Last::Other;
        }
        self.follow();
        true
    }

    fn moved(&mut self, by: i32) -> bool {
        self.vertical(by);
        self.follow();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: &mut TextArea, code: KeyCode) {
        t.key(&KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn typed(t: &mut TextArea, s: &str) {
        for c in s.chars() {
            key(t, KeyCode::Char(c));
        }
    }

    #[test]
    fn wraps_at_spaces() {
        let line: Vec<char> = "one two three".chars().collect();
        assert_eq!(segments(&line, 8), vec![(0, 8), (8, 13)]);
        let long: Vec<char> = "abcdefghij".chars().collect();
        assert_eq!(segments(&long, 4), vec![(0, 4), (4, 8), (8, 10)]);
        assert_eq!(segments(&[], 4), vec![(0, 0)]);
    }

    #[test]
    fn typing_and_text_round_trip() {
        let mut t = TextArea::new("Hello\n\nworld\n");
        assert_eq!(t.text(), "Hello\n\nworld\n");
        t.go_to_end();
        typed(&mut t, "Bye");
        key(&mut t, KeyCode::Enter);
        typed(&mut t, "x");
        assert_eq!(t.text(), "Hello\n\nworld\nBye\nx");
        key(&mut t, KeyCode::Backspace);
        key(&mut t, KeyCode::Backspace);
        assert_eq!(t.text(), "Hello\n\nworld\nBye");
    }

    #[test]
    fn undo_goes_back_by_words() {
        let mut t = TextArea::new("");
        typed(&mut t, "one two");
        t.undo();
        assert_eq!(t.text(), "one");
        t.undo();
        assert_eq!(t.text(), "");
        t.redo();
        assert_eq!(t.text(), "one");
    }

    #[test]
    fn moves_on_screen_rows() {
        let mut t = TextArea::new("one two three");
        t.width = 8;
        t.go_to_end();
        assert_eq!(t.cursor(), (1, 5));
        key(&mut t, KeyCode::Up);
        assert_eq!((t.row, t.col), (0, 5));
        key(&mut t, KeyCode::End);
        // Before the space that ends the wrapped row.
        assert_eq!(t.col, 7);
        key(&mut t, KeyCode::Home);
        assert_eq!(t.col, 0);
        key(&mut t, KeyCode::Down);
        assert_eq!(t.cursor().0, 1);
    }

    #[test]
    fn clicks_place_the_cursor() {
        let mut t = TextArea::new("one two three\nlast");
        t.width = 8;
        t.place(1, 2);
        assert_eq!((t.row, t.col), (0, 10));
        t.place(9, 99);
        assert_eq!((t.row, t.col), (1, 4));
    }

    #[test]
    fn paste_keeps_lines() {
        let mut t = TextArea::new("ab");
        t.col = 1;
        t.paste("1\r\n2");
        assert_eq!(t.text(), "a1\n2b");
        t.undo();
        assert_eq!(t.text(), "ab");
    }

    #[test]
    fn joins_lines_with_backspace_and_delete() {
        let mut t = TextArea::new("ab\ncd");
        t.row = 1;
        t.col = 0;
        key(&mut t, KeyCode::Backspace);
        assert_eq!(t.text(), "abcd");
        assert_eq!((t.row, t.col), (0, 2));
        t.newline();
        t.row = 0;
        t.col = 2;
        key(&mut t, KeyCode::Delete);
        assert_eq!(t.text(), "abcd");
    }
}
