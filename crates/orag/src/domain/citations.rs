//! Extraction and validation of `[n]` citation markers in generated answers.
//!
//! Policy (v0.1): the answer is scanned as raw text, tuned to never miss a
//! real citation. A lost citation makes a grounded answer look ungrounded;
//! a false one only shows up in `invalid` or credits a source the model did
//! use nearby. The only text skipped is closed fenced code blocks, also
//! inside lists and blockquotes. Markers may be `[1]`, `[1, 3]`, `[1-3]`,
//! `[^1]` or `\[1\]`. Accepted limitations: brackets in inline code
//! (`` `xs[1]` ``), markdown links (`[3](https://…)`), source footers
//! (`[1]: iade.md`) and small numeric ranges in prose (`[18-25]`) count as
//! markers; emphasized markers (`[**1**]`) do not. The generation prompt
//! asks for `[1]` or `[2][3]`.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Citations {
    /// 1-based source numbers that exist, unique, in first-seen order.
    pub valid: Vec<usize>,
    /// Numbers cited by the model that do not correspond to a source.
    pub invalid: Vec<usize>,
}

/// Longest number read as a marker; longer digit runs (years) are not markers.
const MAX_MARKER_DIGITS: usize = 3;
/// Every marker number is below this, so a flat table can dedupe them.
const MARKER_LIMIT: usize = 1000;
/// Widest range (`[a-b]`) that is expanded into its numbers.
const MAX_RANGE_SPAN: usize = 64;

/// Finds citation markers (see the module docs; spaces and line breaks are
/// allowed inside) and checks `1 <= n <= source_count`. A `[` inside another
/// bracket starts a new candidate, so `[bkz. [2]]` still yields 2.
pub fn extract_citations(answer: &str, source_count: usize) -> Citations {
    let mut found = Collector {
        source_count,
        seen: [false; MARKER_LIMIT],
        citations: Citations::default(),
    };
    for prose in outside_closed_fences(answer) {
        found.scan(prose);
    }
    found.citations
}

struct Collector {
    source_count: usize,
    seen: [bool; MARKER_LIMIT],
    citations: Citations,
}

impl Collector {
    fn scan(&mut self, prose: &str) {
        let mut rest = prose;
        while let Some(open) = rest.find('[') {
            let after = &rest[open + 1..];
            let Some(close) = after.find(']') else { break };
            let inner = &after[..close];
            if let Some(nested) = inner.rfind('[') {
                rest = &after[nested..];
                continue;
            }
            if is_marker(inner) {
                for part in inner.split(',') {
                    for number in part_numbers(part).into_iter().flatten() {
                        self.add(number);
                    }
                }
            }
            rest = &after[close + 1..];
        }
    }

    fn add(&mut self, number: usize) {
        if std::mem::replace(&mut self.seen[number], true) {
            return;
        }
        if (1..=self.source_count).contains(&number) {
            self.citations.valid.push(number);
        } else {
            self.citations.invalid.push(number);
        }
    }
}

/// Whether every comma-separated part of a bracket's inside is a marker part.
fn is_marker(inner: &str) -> bool {
    inner.split(',').all(|part| part_numbers(part).is_some())
}

/// The numbers of one marker part (`2`, ` ^3 `, `1-3`, `1\`), or `None`.
fn part_numbers(part: &str) -> Option<std::ops::RangeInclusive<usize>> {
    let part = part.trim().trim_start_matches('^').trim_end_matches('\\');
    match part.split_once(['-', '–', '—', '−']) {
        Some((low, high)) => {
            let (low, high) = (number(low.trim())?, number(high.trim())?);
            (low <= high && high - low <= MAX_RANGE_SPAN).then_some(low..=high)
        }
        None => number(part).map(|n| n..=n),
    }
}

/// The parts of `answer` outside fenced code blocks that are closed. A fence
/// is a line of three or more backticks or tildes (after any indentation and
/// blockquote `>` markers), closed by a later line of the same character at
/// least as long. A fence that is never closed is plain text, so a stray
/// fence never hides the rest of the answer. Linear time: a suffix table of
/// the longest closing run below each line tells whether an opener closes
/// before searching, and a search only walks lines it then skips.
fn outside_closed_fences(answer: &str) -> Vec<&str> {
    let lines: Vec<(usize, &str)> = answer
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .collect();
    let closers: Vec<Option<(u8, usize)>> = lines.iter().map(|(_, line)| closer(line)).collect();
    // longest_below[i][m]: longest closing run of marker m on lines >= i.
    let mut longest_below = vec![[0usize; 2]; lines.len() + 1];
    for i in (0..lines.len()).rev() {
        longest_below[i] = longest_below[i + 1];
        if let Some((marker, run)) = closers[i] {
            let slot = marker_slot(marker);
            longest_below[i][slot] = longest_below[i][slot].max(run);
        }
    }
    let mut parts = Vec::new();
    let mut prose_start = 0;
    let mut i = 0;
    while i < lines.len() {
        let (start, line) = lines[i];
        let close = opener(line)
            .filter(|&(marker, run)| longest_below[i + 1][marker_slot(marker)] >= run)
            .and_then(|(marker, run)| {
                (i + 1..lines.len())
                    .find(|&j| closers[j].is_some_and(|(m, r)| m == marker && r >= run))
            });
        match close {
            Some(j) => {
                parts.push(&answer[prose_start..start]);
                prose_start = lines[j].0 + lines[j].1.len();
                i = j + 1;
            }
            None => i += 1,
        }
    }
    parts.push(&answer[prose_start..]);
    parts
}

fn marker_slot(marker: u8) -> usize {
    usize::from(marker == b'~')
}

/// The fence character and run length if `line` opens a fence.
fn opener(line: &str) -> Option<(u8, usize)> {
    let (marker, run, info) = fence_run(line)?;
    (!(marker == b'`' && info.contains('`'))).then_some((marker, run))
}

/// The fence character and run length if `line` is only a fence (a closer).
fn closer(line: &str) -> Option<(u8, usize)> {
    let (marker, run, info) = fence_run(line)?;
    info.trim().is_empty().then_some((marker, run))
}

/// A run of three or more backticks or tildes at the start of `line` after
/// indentation and blockquote markers, with the text after it.
fn fence_run(line: &str) -> Option<(u8, usize, &str)> {
    let body = line
        .trim_end_matches(['\n', '\r'])
        .trim_start_matches(|c: char| c == '>' || c.is_whitespace());
    let marker = *body.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let run = body.bytes().take_while(|&b| b == marker).count();
    (run >= 3).then(|| (marker, run, &body[run..]))
}

/// 1-3 ASCII digits with no leading zero (`0` itself is allowed).
fn number(text: &str) -> Option<usize> {
    let digits = !text.is_empty()
        && text.len() <= MAX_MARKER_DIGITS
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'));
    digits.then(|| text.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cite(answer: &str, sources: usize) -> (Vec<usize>, Vec<usize>) {
        let c = extract_citations(answer, sources);
        (c.valid, c.invalid)
    }

    #[test]
    fn extracts_unique_valid_markers_in_order() {
        assert_eq!(
            cite("Yes [2]. Also [1][2] and [ 3 ].", 3),
            (vec![2, 1, 3], vec![])
        );
    }

    #[test]
    fn out_of_range_markers_are_invalid() {
        assert_eq!(cite("See [0], [4] and [2].", 3), (vec![2], vec![0, 4]));
    }

    #[test]
    fn comma_lists_count_each_number_and_may_wrap() {
        assert_eq!(cite("Both [1, 3] and [2,9].", 3), (vec![1, 3, 2], vec![9]));
        assert_eq!(cite("[1,\n2]", 3), (vec![1, 2], vec![]));
    }

    #[test]
    fn non_markers_are_ignored() {
        let answer = "array[i] [abc] [1, a] [1,] [,2] [1;2] [007] [01] [2024] [3-1] [**1**]";
        assert_eq!(cite(answer, 3), (vec![], vec![]));
    }

    #[test]
    fn a_marker_inside_other_brackets_is_found() {
        assert_eq!(cite("Kural böyle [bkz. [2]].", 3), (vec![2], vec![]));
        assert_eq!(cite("x[ [1] y", 3), (vec![1], vec![]));
    }

    #[test]
    fn markers_count_whatever_surrounds_them() {
        let answer = "İade 14 gündür[1]. Geçerli [2](madde 3). Yıl [3](2019).";
        assert_eq!(cite(answer, 3), (vec![1, 2, 3], vec![]));
    }

    #[test]
    fn closed_fences_are_skipped() {
        let answer = "Kural [1].\n```rust\nlet a = [2];\n```\nSon [3].";
        assert_eq!(cite(answer, 3), (vec![1, 3], vec![]));
        let tilde = "~~~\n[1]\n```\n[2]\n~~~\n[3]";
        assert_eq!(cite(tilde, 3), (vec![3], vec![]));
    }

    #[test]
    fn an_unclosed_fence_hides_nothing() {
        assert_eq!(
            cite("Süre [1]\n````\nkod\n```\n[2]", 3),
            (vec![1, 2], vec![])
        );
        let inline = "```npm install``` çalıştırın [1].";
        assert_eq!(cite(inline, 3), (vec![1], vec![]));
    }

    #[test]
    fn accepted_limitations_are_counted() {
        // Inline code, links and footers are not skipped (see module docs).
        let answer = "Use `xs[1]`, see [2](https://x).\n\n[3]: iade.md";
        assert_eq!(cite(answer, 3), (vec![1, 2, 3], vec![]));
    }

    #[test]
    fn ranges_footnotes_and_escapes_are_read() {
        assert_eq!(cite("Süre [1–2]. Ek [2-4].", 3), (vec![1, 2, 3], vec![4]));
        assert_eq!(cite("14 gün[^1] ve \\[2\\].", 3), (vec![1, 2], vec![]));
    }

    #[test]
    fn fences_inside_lists_and_quotes_are_skipped() {
        let list =
            "1. Kurulum:\n    - Adım:\n      ```bash\n      echo ${arr[2]}\n      ```\nSonuç [1].";
        assert_eq!(cite(list, 3), (vec![1], vec![]));
        let quote = "> ```\n> x[2]\n> ```\n[3]";
        assert_eq!(cite(quote, 3), (vec![3], vec![]));
    }

    #[test]
    fn unclosed_fences_cost_linear_time() {
        // Strictly shorter unclosed openers defeat any per-run memo.
        let answer: String = (3..1_400)
            .rev()
            .map(|run| "`".repeat(run) + " x\n")
            .collect();
        let started = std::time::Instant::now();
        assert_eq!(cite(&(answer + "[1]"), 3), (vec![1], vec![]));
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    #[test]
    fn repeated_markers_are_counted_once() {
        let answer: String = (0..50_000).map(|n| format!(" [{}]", n % 1000)).collect();
        assert_eq!(cite(&answer, 10).0.len(), 10);
    }
}
