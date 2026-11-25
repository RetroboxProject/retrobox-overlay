//! Text model of the terminal box: a bounded scrollback with wrapping and selection.
//!
//! It knows nothing about fonts or the screen: it turns the raw output of a
//! process into lines (dropping colours and control codes, honouring carriage
//! returns and backspaces), wraps them to a column count and keeps the
//! selection. The last line is the one being written, where the cursor is,
//! and always exists (empty or not). Memory is bounded by `MAX_LINES`: the
//! oldest lines are dropped first.

use std::collections::VecDeque;

/// Lines kept above the one being written.
const MAX_LINES: usize = 4000;
/// Tab stops, in columns.
const TAB: usize = 8;

/// A visual row: a slice `[start, end)` of the absolute line `line`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// A place in the text: absolute line number and character column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// Where the escape-sequence filter is, between two chunks of output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Escape {
    /// Plain text.
    None,
    /// After ESC.
    Started,
    /// Inside a CSI sequence (`ESC [`), up to its final byte.
    Csi,
    /// Inside an OSC string (`ESC ]`), up to BEL or ST.
    Osc,
    /// ESC inside an OSC string: ST (`ESC \`) when a backslash follows.
    OscEnd,
}

/// Scrollback of the installer's output.
#[derive(Debug)]
pub struct Terminal {
    lines: VecDeque<Vec<char>>,
    partial: Vec<char>,
    /// Where the next character goes in `partial` (carriage return: 0).
    cursor: usize,
    escape: Escape,
    /// Lines dropped from the start: the absolute number of the oldest kept.
    dropped: usize,
    pub anchor: Option<Position>,
    pub selection_end: Option<Position>,
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

impl Terminal {
    pub const fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            partial: Vec::new(),
            cursor: 0,
            escape: Escape::None,
            dropped: 0,
            anchor: None,
            selection_end: None,
        }
    }

    // ------------------------------------------------------------------ input

    /// Append raw process output.
    pub fn feed(&mut self, text: &str) {
        for ch in text.chars() {
            if self.filter_escape(ch) {
                continue;
            }
            match ch {
                '\n' => self.commit(),
                '\r' => self.cursor = 0,
                '\x08' => self.backspace(),
                '\t' => {
                    let stop = (self.cursor / TAB + 1) * TAB;
                    while self.cursor < stop {
                        self.put(' ');
                    }
                }
                // the other control characters draw nothing
                '\0'..='\x07' | '\x0b' | '\x0c' | '\x0e'..='\x1f' | '\x7f' => {}
                ch => self.put(ch),
            }
        }
    }

    /// Whether `ch` belongs to an escape sequence (and is dropped).
    fn filter_escape(&mut self, ch: char) -> bool {
        self.escape = match (self.escape, ch) {
            (Escape::None, '\x1b') => Escape::Started,
            (Escape::None, _) => return false,
            (Escape::Started, '[') => Escape::Csi,
            (Escape::Started, ']') => Escape::Osc,
            // a two-byte escape (or a stray one), a sequence's final byte, BEL
            // or ST
            (Escape::Started | Escape::OscEnd, _)
            | (Escape::Csi, '\u{40}'..='\u{7e}')
            | (Escape::Osc, '\x07') => Escape::None,
            (Escape::Csi, _) => Escape::Csi,
            (Escape::Osc, '\x1b') => Escape::OscEnd,
            (Escape::Osc, _) => Escape::Osc,
        };
        true
    }

    /// Write a character at the cursor: past the end it extends the line,
    /// before it (after a carriage return) it overwrites, as progress bars
    /// rewrite their line.
    fn put(&mut self, ch: char) {
        if let Some(slot) = self.partial.get_mut(self.cursor) {
            *slot = ch;
        } else {
            self.partial.push(ch);
        }
        self.cursor += 1;
    }

    fn backspace(&mut self) {
        if self.cursor >= self.partial.len() {
            // erasing what was just typed ("\b \b")
            self.partial.pop();
            self.cursor = self.partial.len();
        } else {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    fn commit(&mut self) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(std::mem::take(&mut self.partial));
        self.cursor = 0;
    }

    // ------------------------------------------------------------------ lines

    /// Absolute number of the oldest line still kept.
    #[cfg(test)]
    pub const fn first_line(&self) -> usize {
        self.dropped
    }

    /// The cursor's column in the line being written.
    pub const fn cursor_column(&self) -> usize {
        self.cursor
    }

    /// Absolute number of the line being written (where the cursor is).
    pub fn cursor_line(&self) -> usize {
        self.dropped + self.lines.len()
    }

    /// The characters of an absolute line (empty if it is gone).
    pub fn line(&self, number: usize) -> &[char] {
        let Some(index) = number.checked_sub(self.dropped) else {
            return &[];
        };
        match index.cmp(&self.lines.len()) {
            std::cmp::Ordering::Less => &self.lines[index],
            std::cmp::Ordering::Equal => &self.partial,
            std::cmp::Ordering::Greater => &[],
        }
    }

    /// The whole scrollback.
    pub fn text(&self) -> String {
        let mut lines: Vec<String> = self
            .lines
            .iter()
            .map(|line| line.iter().collect())
            .collect();
        if !self.partial.is_empty() {
            lines.push(self.partial.iter().collect());
        }
        lines.join("\n")
    }

    // --------------------------------------------------------------- wrapping

    fn rows_of(length: usize, columns: usize) -> usize {
        length.div_ceil(columns.max(1)).max(1)
    }

    /// Number of visual rows once wrapped to `columns`.
    pub fn row_count(&self, columns: usize) -> usize {
        self.lines
            .iter()
            .map(|line| Self::rows_of(line.len(), columns))
            .sum::<usize>()
            + Self::rows_of(self.partial.len(), columns)
    }

    /// The visual rows `[first, first + count)`, walking back from the end
    /// (`total` is `row_count`): the usual case, looking at the bottom of a
    /// long log, stays cheap.
    pub fn rows(&self, columns: usize, first: usize, count: usize, total: usize) -> Vec<Row> {
        let columns = columns.max(1);
        let wanted_end = (first + count).min(total);
        let mut result = Vec::new();
        let mut row = total;
        let mut number = self.cursor_line();
        loop {
            if row <= first {
                break;
            }
            let length = self.line(number).len();
            let spans = Self::rows_of(length, columns);
            row = row.saturating_sub(spans);
            for span in (0..spans).rev() {
                let visual = row + span;
                if (first..wanted_end).contains(&visual) {
                    let start = span * columns;
                    result.push(Row {
                        line: number,
                        start,
                        end: (start + columns).min(length),
                    });
                }
            }
            if number == self.dropped {
                break;
            }
            number -= 1;
        }
        result.reverse();
        result
    }

    // -------------------------------------------------------------- selection

    /// Ordered selection bounds, or None when nothing is selected.
    pub fn selection(&self) -> Option<(Position, Position)> {
        let (anchor, end) = (self.anchor?, self.selection_end?);
        match anchor.cmp(&end) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Less => Some((anchor, end)),
            std::cmp::Ordering::Greater => Some((end, anchor)),
        }
    }

    /// Select every line kept.
    pub fn select_all(&mut self) {
        let last = self.cursor_line();
        self.anchor = Some(Position {
            line: self.dropped,
            column: 0,
        });
        self.selection_end = Some(Position {
            line: last,
            column: self.line(last).len(),
        });
    }

    pub const fn clear_selection(&mut self) {
        self.anchor = None;
        self.selection_end = None;
    }

    /// The selected text, or the whole scrollback when nothing is selected.
    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection() else {
            return self.text();
        };
        let first = start.line.max(self.dropped);
        (first..=end.line)
            .map(|number| {
                let text = self.line(number);
                let begin = if number == start.line {
                    start.column
                } else {
                    0
                };
                let stop = if number == end.line {
                    end.column
                } else {
                    text.len()
                };
                let (begin, stop) = (begin.min(text.len()), stop.min(text.len()));
                text[begin..stop.max(begin)].iter().collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(terminal: &Terminal) -> Vec<String> {
        (terminal.first_line()..=terminal.cursor_line())
            .map(|number| terminal.line(number).iter().collect())
            .collect()
    }

    #[test]
    fn splits_lines_and_keeps_the_one_being_written() {
        let mut terminal = Terminal::new();
        terminal.feed("one\ntwo\r\nthr");
        assert_eq!(lines(&terminal), ["one", "two", "thr"]);
        assert_eq!(terminal.cursor_line(), 2);
    }

    #[test]
    fn drops_colours_even_when_a_sequence_is_cut_between_chunks() {
        let mut terminal = Terminal::new();
        terminal.feed("\x1b[1;3");
        terminal.feed("1mred\x1b[0m \x1b]0;title\x07ok\x1b]8;;");
        terminal.feed("\x1b\\!");
        assert_eq!(terminal.text(), "red ok!");
    }

    #[test]
    fn a_carriage_return_rewrites_the_line_and_keeps_its_tail() {
        let mut terminal = Terminal::new();
        terminal.feed("10%  downloading\r50%");
        assert_eq!(terminal.text(), "50%  downloading");
        terminal.feed("\n");
        terminal.feed("done\r\n");
        assert_eq!(lines(&terminal), ["50%  downloading", "done", ""]);
    }

    #[test]
    fn backspace_erases_what_was_typed() {
        let mut terminal = Terminal::new();
        terminal.feed("abc\x08 \x08d");
        assert_eq!(terminal.text(), "abd");
    }

    #[test]
    fn tabs_go_to_the_next_stop() {
        let mut terminal = Terminal::new();
        terminal.feed("ab\tc");
        assert_eq!(terminal.text(), "ab      c");
    }

    #[test]
    fn other_control_characters_are_dropped() {
        let mut terminal = Terminal::new();
        terminal.feed("a\x00\x07b\x7fc");
        assert_eq!(terminal.text(), "abc");
    }

    #[test]
    fn wraps_rows_and_walks_back_from_the_end() {
        let mut terminal = Terminal::new();
        terminal.feed("abcdefghij\nxy\n");
        // "abcd" "efgh" "ij" / "xy" / ""
        assert_eq!(terminal.row_count(4), 5);
        let total = terminal.row_count(4);
        let rows = terminal.rows(4, 1, 3, total);
        assert_eq!(
            rows,
            [
                Row {
                    line: 0,
                    start: 4,
                    end: 8
                },
                Row {
                    line: 0,
                    start: 8,
                    end: 10
                },
                Row {
                    line: 1,
                    start: 0,
                    end: 2
                },
            ]
        );
        assert_eq!(terminal.rows(4, 0, 100, total).len(), 5);
    }

    #[test]
    fn keeps_a_bounded_scrollback() {
        let mut terminal = Terminal::new();
        for number in 0..MAX_LINES + 10 {
            terminal.feed(&format!("{number}\n"));
        }
        assert_eq!(terminal.first_line(), 10);
        assert_eq!(terminal.line(9), &[] as &[char]);
        assert_eq!(terminal.line(10).iter().collect::<String>(), "10");
    }

    #[test]
    fn selects_text_across_lines() {
        let mut terminal = Terminal::new();
        terminal.feed("hello\nworld\n");
        assert_eq!(terminal.selected_text(), "hello\nworld");
        terminal.anchor = Some(Position { line: 1, column: 3 });
        terminal.selection_end = Some(Position { line: 0, column: 2 });
        assert_eq!(terminal.selected_text(), "llo\nwor");
        terminal.clear_selection();
        assert_eq!(terminal.selection(), None);
    }
}
