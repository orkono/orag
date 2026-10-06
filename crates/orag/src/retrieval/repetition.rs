//! Stops an answer that has fallen into a loop: the same line again and
//! again, which a small model can produce from repetitive source text
//! (anonymized court decisions) until the output budget runs out.

/// This many identical lines in a row end the answer.
pub const MAX_LINE_REPEATS: usize = 3;
/// Shorter lines (`---`, `* 20`) may repeat legitimately.
const MIN_GUARDED_LINE_CHARS: usize = 12;

/// Watches completed lines. Only consecutive copies count (blank lines in
/// between are ignored), so a line that legitimately recurs, such as the same
/// finding under several defendants or a table separator, never trips it.
#[derive(Debug, Default)]
pub struct LineRepeatGuard {
    line: String,
    last: String,
    run: usize,
}

impl LineRepeatGuard {
    /// Feeds streamed text; true once `MAX_LINE_REPEATS` identical lines
    /// have been completed in a row.
    pub fn push(&mut self, piece: &str) -> bool {
        let mut looped = false;
        for ch in piece.chars() {
            if ch == '\n' {
                looped |= self.finish_line();
            } else {
                self.line.push(ch);
            }
        }
        looped
    }

    fn finish_line(&mut self) -> bool {
        let key = line_key(&self.line);
        self.line.clear();
        if key.is_empty() {
            return false;
        }
        if key == self.last {
            self.run += 1;
        } else {
            self.last = key;
            self.run = 1;
        }
        self.last.chars().count() >= MIN_GUARDED_LINE_CHARS && self.run >= MAX_LINE_REPEATS
    }
}

/// The line without its list marker (`1.`, `2)`, then `*`, `-`, `•`; each
/// followed by a space) and with whitespace collapsed: a loop often numbers
/// each copy. A number without a space after its dot is content, so dotted
/// dates and thousands (`01.02.2016`, `1.000 TL`) stay.
pub fn line_key(line: &str) -> String {
    let mut rest = line.trim_start();
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0
        && let Some(tail) = rest[digits..]
            .strip_prefix(['.', ')'])
            .filter(|tail| tail.starts_with(char::is_whitespace))
    {
        rest = tail.trim_start();
    }
    while let Some(tail) = rest
        .strip_prefix(['*', '-', '•'])
        .filter(|tail| tail.starts_with(char::is_whitespace))
    {
        rest = tail.trim_start();
    }
    rest.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "* Sanık hakkında kasten yaralama [3]\n";

    #[test]
    fn the_third_copy_in_a_row_trips_the_guard() {
        let mut guard = LineRepeatGuard::default();
        assert!(!guard.push(LINE));
        assert!(!guard.push(LINE));
        assert!(guard.push(LINE));
    }

    #[test]
    fn blank_lines_between_copies_do_not_hide_a_loop() {
        let mut guard = LineRepeatGuard::default();
        assert!(!guard.push(&format!("{LINE}\n{LINE}\n")));
        assert!(guard.push(&format!("\n{LINE}")));
    }

    #[test]
    fn a_line_that_recurs_between_other_lines_is_not_a_loop() {
        let mut guard = LineRepeatGuard::default();
        for defendant in ["A", "B", "C", "D"] {
            assert!(!guard.push(&format!("Sanık {defendant}:\n{LINE}")));
        }
        let mut guard = LineRepeatGuard::default();
        for table in 0..4 {
            let text = format!("| kod | fiyat | adet |\n|---|---|---|\n| {table} | 10 | 2 |\n");
            assert!(!guard.push(&text));
        }
    }

    #[test]
    fn lines_split_across_pieces_and_spacing_compare_equal() {
        let mut guard = LineRepeatGuard::default();
        assert!(!guard.push("*   Sanık hakkında "));
        assert!(!guard.push("kasten yaralama [3]\n* Sanık  hakkında kasten"));
        assert!(guard.push(" yaralama [3]\n* Sanık hakkında kasten yaralama [3]\n"));
    }

    #[test]
    fn list_numbers_and_markers_do_not_hide_a_loop() {
        let mut guard = LineRepeatGuard::default();
        let body = "Sanık hakkında kasten yaralama suçundan hüküm [1]";
        assert!(!guard.push(&format!("1.  {body}\n")));
        assert!(!guard.push(&format!("2) {body}\n")));
        assert!(guard.push(&format!("- {body}\n")));
        let mut guard = LineRepeatGuard::default();
        for n in 1..=2 {
            assert!(!guard.push(&format!("{n}. • {body}\n")));
        }
        assert!(guard.push(&format!("36. {body}\n")));
    }

    #[test]
    fn different_items_with_numbers_are_not_a_loop() {
        let mut guard = LineRepeatGuard::default();
        for n in 1..=20 {
            assert!(!guard.push(&format!("{n}. Ürün kodu {n} için fiyat bilgisi\n")));
        }
    }

    #[test]
    fn short_or_unfinished_lines_never_trip_it() {
        let mut guard = LineRepeatGuard::default();
        for _ in 0..10 {
            assert!(!guard.push("| --- |\n"));
        }
        assert!(!guard.push(&LINE.trim_end().repeat(5)));
    }

    #[test]
    fn line_keys_drop_markers_and_spacing() {
        assert_eq!(line_key("  12.  *  a   b "), "a b");
        assert_eq!(line_key("3) - a"), "a");
        assert_eq!(line_key("02/02/2016 tarihinde"), "02/02/2016 tarihinde");
        // Dotted dates and thousands are content, not list numbers.
        assert_eq!(line_key("01.02.2016 tarihli"), "01.02.2016 tarihli");
        assert_eq!(line_key("1.000 TL ceza"), "1.000 TL ceza");
        assert_eq!(line_key("2. 1.000 TL ceza"), "1.000 TL ceza");
    }

    #[test]
    fn dated_or_priced_lines_are_not_a_loop() {
        let mut guard = LineRepeatGuard::default();
        for day in ["01", "08", "15", "22"] {
            assert!(!guard.push(&format!("{day}.02.2016 tarihli duruşmaya katılmadı [1]\n")));
        }
        let mut guard = LineRepeatGuard::default();
        for n in 1..=4 {
            assert!(!guard.push(&format!("{n}.000 TL adli para cezası verildi [2]\n")));
        }
    }
}
