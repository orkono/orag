//! Streaming text utilities: UTF-8 reassembly of token bytes and removal of
//! reasoning (`<think>…</think>`) spans.

/// Collects token bytes and releases only complete UTF-8 text.
#[derive(Debug, Default)]
pub struct Utf8Accumulator {
    pending: Vec<u8>,
}

impl Utf8Accumulator {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    out.push_str(text);
                    self.pending.clear();
                    return out;
                }
                Err(err) => {
                    let valid = err.valid_up_to();
                    out.push_str(&String::from_utf8_lossy(&self.pending[..valid]));
                    match err.error_len() {
                        None => {
                            self.pending.drain(..valid); // incomplete tail: wait for more bytes
                            return out;
                        }
                        Some(len) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..valid + len);
                        }
                    }
                }
            }
        }
    }

    pub fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

/// Removes `<think>…</think>` spans (tags may be split across pieces) and the
/// whitespace that follows a closing tag.
#[derive(Debug, Default)]
pub struct ThinkFilter {
    buf: String,
    inside: bool,
    skip_whitespace: bool,
}

impl ThinkFilter {
    /// For prompts whose template already opened `<think>`: the output starts
    /// inside the reasoning block and only its `</think>` appears.
    pub fn already_inside() -> ThinkFilter {
        ThinkFilter {
            inside: true,
            ..ThinkFilter::default()
        }
    }

    pub fn push(&mut self, piece: &str) -> String {
        self.buf.push_str(piece);
        let mut out = String::new();
        loop {
            let tag = if self.inside { CLOSE } else { OPEN };
            if let Some(pos) = self.buf.find(tag) {
                if !self.inside {
                    let before = self.buf[..pos].to_string();
                    self.emit(&mut out, &before);
                }
                self.buf.drain(..pos + tag.len());
                self.inside = !self.inside;
                self.skip_whitespace = !self.inside;
                continue;
            }
            let keep = partial_suffix_len(&self.buf, tag);
            let cut = self.buf.len() - keep;
            if !self.inside {
                let ready = self.buf[..cut].to_string();
                self.emit(&mut out, &ready);
            }
            self.buf.drain(..cut);
            return out;
        }
    }

    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.buf);
        if self.inside {
            return String::new();
        }
        let mut out = String::new();
        self.emit(&mut out, &rest);
        out
    }

    fn emit(&mut self, out: &mut String, text: &str) {
        let text = if self.skip_whitespace {
            text.trim_start()
        } else {
            text
        };
        if !text.is_empty() {
            self.skip_whitespace = false;
            out.push_str(text);
        }
    }
}

/// Length of the longest suffix of `text` that is a proper prefix of `tag`.
fn partial_suffix_len(text: &str, tag: &str) -> usize {
    (1..tag.len())
        .rev()
        .find(|&n| {
            n <= text.len()
                && text.is_char_boundary(text.len() - n)
                && tag.starts_with(&text[text.len() - n..])
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_waits_for_complete_multibyte_sequences() {
        let mut acc = Utf8Accumulator::default();
        let bytes = "ğü".as_bytes(); // c4 9f c3 bc
        assert_eq!(acc.push(&bytes[..1]), "");
        assert_eq!(acc.push(&bytes[1..3]), "ğ");
        assert_eq!(acc.push(&bytes[3..]), "ü");
        assert_eq!(acc.finish(), "");
    }

    #[test]
    fn utf8_replaces_invalid_bytes_and_continues() {
        let mut acc = Utf8Accumulator::default();
        assert_eq!(acc.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
    }

    #[test]
    fn utf8_finish_flushes_dangling_bytes_lossily() {
        let mut acc = Utf8Accumulator::default();
        assert_eq!(acc.push(&[0xC4]), "");
        assert_eq!(acc.finish(), "\u{FFFD}");
    }

    fn run(pieces: &[&str]) -> String {
        let mut filter = ThinkFilter::default();
        let mut out: String = pieces.iter().map(|p| filter.push(p)).collect();
        out.push_str(&filter.finish());
        out
    }

    #[test]
    fn think_filter_passes_plain_text() {
        assert_eq!(run(&["Merhaba ", "dünya"]), "Merhaba dünya");
    }

    #[test]
    fn think_filter_removes_span_and_following_blank_lines() {
        assert_eq!(
            run(&["<think>\nreasoning\n</think>\n\nCevap [1]."]),
            "Cevap [1]."
        );
    }

    #[test]
    fn think_filter_handles_tags_split_across_pieces() {
        assert_eq!(run(&["<thi", "nk>gizli</th", "ink>\n\nCevap"]), "Cevap");
    }

    #[test]
    fn think_filter_releases_held_prefix_that_was_not_a_tag() {
        assert_eq!(run(&["a <", "b> c"]), "a <b> c");
    }

    #[test]
    fn think_filter_drops_unterminated_span() {
        assert_eq!(run(&["Önce ", "<think>never closed"]), "Önce ");
    }

    #[test]
    fn a_prefilled_think_tag_starts_the_filter_inside() {
        let mut filter = ThinkFilter::already_inside();
        let mut out = filter.push("step one, step two</think>\n\nAnswer");
        out.push_str(&filter.finish());
        assert_eq!(out, "Answer");
    }
}
