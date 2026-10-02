//! Structure-aware chunking: headings bound chunks, small blocks merge up to a
//! token target, oversized blocks split into overlapping windows.

use unicode_segmentation::UnicodeSegmentation;

use crate::domain::document::{Block, ParsedDocument};
use crate::error::{OragError, Result};

/// Bump when chunk boundaries change; part of the embedding-space fingerprint.
pub const CHUNKER_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkerConfig {
    pub target_tokens: usize,
    pub max_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self {
            target_tokens: 384,
            max_tokens: 512,
            overlap_tokens: 48,
        }
    }
}

impl ChunkerConfig {
    /// `0 < target <= max`, and an overlap below half the budget. A violation
    /// is a server configuration error, not a problem with the document.
    fn validate(&self) -> Result<()> {
        let ok = self.target_tokens > 0
            && self.target_tokens <= self.max_tokens
            && self.overlap_tokens < self.max_tokens.div_ceil(2);
        if ok {
            Ok(())
        } else {
            Err(OragError::Internal(format!(
                "invalid chunker config {self:?}: need 0 < target <= max and overlap < max / 2"
            )))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDraft {
    pub ordinal: u32,
    /// Full heading path, used for citations.
    pub heading_path: Vec<String>,
    /// Heading path as embedded: `A > B`, trimmed to a quarter of the target.
    pub breadcrumb: String,
    /// The chunk's blocks joined by blank lines; a window of an oversized block
    /// is an exact slice of that block's text (nothing inserted or removed).
    /// A section heading with no body of its own is its own chunk.
    pub text: String,
    pub token_count: usize,
}

/// Share of the target a heading breadcrumb may use, so a long heading path
/// still leaves room to merge small blocks under it.
const BREADCRUMB_BUDGET_DIVISOR: usize = 4;
/// Whitespace-free runs (base64, URLs, long words) are cut into units of at
/// most this many graphemes. A unit that alone exceeds the budget is halved.
const MAX_UNIT_GRAPHEMES: usize = 16;

/// Exactly the text the embedder receives for a chunk.
pub fn compose_embedding_text(breadcrumb: &str, body: &str) -> String {
    if breadcrumb.is_empty() {
        body.to_string()
    } else {
        format!("{breadcrumb}\n\n{body}")
    }
}

impl ChunkDraft {
    /// Text given to the embedder; `token_count` measures exactly this string.
    /// The stored/cited text is `self.text`.
    pub fn embedding_text(&self) -> String {
        compose_embedding_text(&self.breadcrumb, &self.text)
    }
}

/// `count_tokens` must count tokens of the full string it is given. Every
/// chunk is counted exactly on its composed embedding text, so none exceeds
/// `max_tokens`. Errors on an inconsistent config (`Internal`), or when the
/// budget cannot hold a single character (`Model`: max_tokens too small).
pub fn chunk_document(
    doc: &ParsedDocument,
    cfg: &ChunkerConfig,
    count_tokens: &dyn Fn(&str) -> usize,
) -> Result<Vec<ChunkDraft>> {
    cfg.validate()?;
    let mut builder = ChunkBuilder {
        cfg: *cfg,
        count_tokens,
        headings: Vec::new(),
        breadcrumb: String::new(),
        breadcrumb_cost: 0,
        bodiless_heading: None,
        current: String::new(),
        current_estimate: 0,
        join_cost: join_cost(count_tokens),
        call_cost: count_tokens(""),
        chunks: Vec::new(),
    };
    for block in &doc.blocks {
        // A blank block, or a blank heading, carries nothing to embed or cite.
        if block.text().trim().is_empty() {
            continue;
        }
        match block {
            Block::Heading { level, text } => builder.enter_heading(*level, text)?,
            other => builder.add_text(other.text())?,
        }
    }
    builder.finish()?;
    Ok(builder.chunks)
}

struct ChunkBuilder<'a> {
    cfg: ChunkerConfig,
    count_tokens: &'a dyn Fn(&str) -> usize,
    headings: Vec<(u8, String)>,
    breadcrumb: String,
    breadcrumb_cost: usize,
    /// Level of the last heading while no text has followed it.
    bodiless_heading: Option<u8>,
    /// Blocks of the open chunk joined by blank lines, and an estimate of its
    /// composed cost (exact for a single block or window).
    current: String,
    current_estimate: usize,
    /// Tokens a blank-line join adds between two blocks, measured once.
    join_cost: usize,
    /// Tokens the tokenizer adds to every string (BOS/EOS), measured once.
    call_cost: usize,
    chunks: Vec<ChunkDraft>,
}

impl ChunkBuilder<'_> {
    fn heading_path(&self) -> Vec<String> {
        self.headings.iter().map(|(_, text)| text.clone()).collect()
    }

    /// Tokens of `body` as the embedder will see it under the current breadcrumb.
    fn cost(&self, body: &str) -> usize {
        (self.count_tokens)(&compose_embedding_text(&self.breadcrumb, body))
    }

    fn set_breadcrumb(&mut self, path: &[String]) {
        self.breadcrumb = self.trimmed_breadcrumb(path);
        self.breadcrumb_cost = (self.count_tokens)(&self.breadcrumb);
    }

    fn enter_heading(&mut self, level: u8, text: &str) -> Result<()> {
        self.flush()?;
        // A heading followed directly by a sibling or a higher heading has no
        // body and no subsections; it becomes a chunk so its words are indexed.
        // One followed by a deeper heading lives on in that heading's breadcrumb.
        if self
            .bodiless_heading
            .is_some_and(|previous| level <= previous)
        {
            self.emit_bodiless_heading()?;
        }
        self.headings.retain(|(existing, _)| *existing < level);
        self.headings.push((level, text.to_string()));
        self.set_breadcrumb(&self.heading_path());
        self.bodiless_heading = Some(level);
        Ok(())
    }

    /// The last heading's text as a chunk under its parents' breadcrumb.
    fn emit_bodiless_heading(&mut self) -> Result<()> {
        let path = self.heading_path();
        let Some((last, parents)) = path.split_last() else {
            return Ok(());
        };
        self.set_breadcrumb(parents);
        self.add_text(last)?;
        self.flush()?;
        self.set_breadcrumb(&path);
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.flush()?;
        if self.bodiless_heading.is_some() {
            self.emit_bodiless_heading()?;
        }
        Ok(())
    }

    /// `A > B > C` within a quarter of the target. Whole headings are dropped
    /// from the top first, so the most specific ones stay; if even the deepest
    /// heading alone is too long, it is cut at a word boundary (or, for one
    /// long word, at a grapheme boundary, so `İ` keeps its dot).
    fn trimmed_breadcrumb(&self, path: &[String]) -> String {
        // A target too small for any breadcrumb (< 4) gets none, so the whole
        // budget is left for the body.
        let budget = self.cfg.target_tokens / BREADCRUMB_BUDGET_DIVISOR;
        if budget == 0 || path.is_empty() {
            return String::new();
        }
        let fits = |text: &str| (self.count_tokens)(text) <= budget;
        for skip in 0..path.len() {
            let candidate = path[skip..].join(" > ");
            if fits(&candidate) {
                return candidate;
            }
        }
        let deepest = path.last().map(String::as_str).unwrap_or("");
        let words: Vec<&str> = deepest.split_whitespace().collect();
        let by_words = last_fitting(0, words.len(), |n| fits(&words[..n].join(" ")));
        if by_words > 0 {
            return words[..by_words].join(" ");
        }
        let graphemes: Vec<&str> = deepest.graphemes(true).collect();
        let prefix = |n: usize| graphemes[..n].concat();
        prefix(last_fitting(0, graphemes.len(), |n| fits(&prefix(n))))
    }

    fn add_text(&mut self, text: &str) -> Result<()> {
        self.bodiless_heading = None;
        let cost = self.cost(text);
        if cost > self.cfg.max_tokens {
            self.flush()?;
            let mut pieces = self.split_oversized(text)?;
            // The last window stays open, so following small blocks merge into it.
            let tail = pieces.pop();
            for (piece, cost) in pieces {
                self.push_chunk(piece, cost)?;
            }
            if let Some((tail, cost)) = tail {
                self.current = tail;
                self.current_estimate = cost;
            }
            return Ok(());
        }
        // Merge on an estimate (open cost + the block's own body cost + the
        // join), so a chunk of many small blocks is not re-tokenized per block;
        // `flush` counts the joined text exactly.
        // `cost` already includes a join when there is a breadcrumb
        // (breadcrumb + join + text); without one it is the text alone, plus
        // the tokens every call adds (BOS/EOS), which the open chunk has already.
        let added = if self.breadcrumb.is_empty() {
            cost.saturating_sub(self.call_cost) + self.join_cost
        } else {
            cost.saturating_sub(self.breadcrumb_cost)
        };
        if !self.current.is_empty() && self.current_estimate + added <= self.cfg.target_tokens {
            self.current.push_str("\n\n");
            self.current.push_str(text);
            self.current_estimate += added;
            return Ok(());
        }
        self.flush()?;
        self.current = text.to_string();
        self.current_estimate = cost;
        Ok(())
    }

    /// Closes the open chunk, counting it exactly. If the estimate was too low
    /// (a tokenizer that adds tokens across joins), it is split like any
    /// oversized text, so no chunk exceeds `max_tokens`.
    fn flush(&mut self) -> Result<()> {
        if self.current.is_empty() {
            return Ok(());
        }
        let text = std::mem::take(&mut self.current);
        self.current_estimate = 0;
        let cost = self.cost(&text);
        if cost <= self.cfg.max_tokens {
            return self.push_chunk(text, cost);
        }
        for (piece, cost) in self.split_oversized(&text)? {
            self.push_chunk(piece, cost)?;
        }
        Ok(())
    }

    fn push_chunk(&mut self, text: String, token_count: usize) -> Result<()> {
        let ordinal = u32::try_from(self.chunks.len())
            .map_err(|_| OragError::InvalidInput("document has too many chunks".into()))?;
        self.chunks.push(ChunkDraft {
            ordinal,
            heading_path: self.heading_path(),
            breadcrumb: self.breadcrumb.clone(),
            text,
            token_count,
        });
        Ok(())
    }

    /// Cuts `text` into windows that are exact slices of the original (no
    /// invented spaces), each with its exact cost, whose composed text fits
    /// `max_tokens`. Words longer than 16 graphemes are cut into units; a unit
    /// that alone exceeds the budget is halved in place until it fits.
    fn split_oversized(&self, text: &str) -> Result<Vec<(String, usize)>> {
        let mut units = unit_spans(text);
        let mut pieces = Vec::new();
        let mut start = 0;
        let mut prev_end = 0;
        while start < units.len() {
            let cost = |from: usize, to: usize, units: &[(usize, usize)]| {
                self.cost(&text[units[from].0..units[to - 1].1])
            };
            if cost(start, start + 1, &units) > self.cfg.max_tokens {
                self.halve_unit(text, &mut units, start)?;
                if start < prev_end {
                    prev_end += 1;
                }
                continue;
            }
            let fits = |end: usize| cost(start, end, &units) <= self.cfg.max_tokens;
            let fitted = last_fitting(start + 1, units.len(), fits);
            if fitted <= prev_end {
                // The overlap left no room to move forward: restart after it.
                start = prev_end;
                continue;
            }
            // A tokenizer need not be monotonic: a shorter window can cost more,
            // so the word-boundary end is used only if it was checked to fit.
            let mut end = word_end(&units, start, fitted, prev_end);
            let mut window_cost = cost(start, end, &units);
            if window_cost > self.cfg.max_tokens {
                end = fitted;
                window_cost = cost(start, end, &units);
            }
            let piece = &text[units[start].0..units[end - 1].1];
            pieces.push((piece.to_string(), window_cost));
            if end == units.len() {
                break;
            }
            prev_end = end;
            start = self.overlap_start(text, &units, start, end, (self.count_tokens)(piece));
        }
        Ok(pieces)
    }

    /// Replaces `units[i]` by two parts cut at its middle grapheme boundary;
    /// a single grapheme too costly for the budget is cut between its
    /// characters as a last resort.
    fn halve_unit(&self, text: &str, units: &mut Vec<(usize, usize)>, i: usize) -> Result<()> {
        let (from, to) = units[i];
        let unit = &text[from..to];
        let mut cuts: Vec<usize> = unit
            .grapheme_indices(true)
            .skip(1)
            .map(|(at, _)| at)
            .collect();
        if cuts.is_empty() {
            cuts = unit.char_indices().skip(1).map(|(at, _)| at).collect();
        }
        let Some(&mid) = cuts.get(cuts.len() / 2) else {
            return Err(OragError::Model(format!(
                "a {}-token chunk budget cannot hold one character; \
                 the embedding model's max_tokens is too small",
                self.cfg.max_tokens
            )));
        };
        units.splice(i..=i, [(from, from + mid), (from + mid, to)]);
        Ok(())
    }

    /// Start of the next window: back from `end` while the repeated text (as
    /// one string, separators included) costs at most `overlap_tokens` and at
    /// most half the window's body, moved forward to a word start, after `start`.
    fn overlap_start(
        &self,
        text: &str,
        units: &[(usize, usize)],
        start: usize,
        end: usize,
        body_cost: usize,
    ) -> usize {
        let limit = self.cfg.overlap_tokens.min(body_cost / 2);
        let tail_end = units[end - 1].1;
        let mut back = end;
        while back > start + 1 {
            let overlap = (self.count_tokens)(&text[units[back - 1].0..tail_end]);
            if overlap > limit {
                break;
            }
            back -= 1;
        }
        match (back..end).find(|&i| starts_word(units, i)) {
            Some(word_start) => word_start,
            // A window with no word boundary at all (CJK, Thai) still overlaps,
            // from a unit boundary; otherwise a cut word is not repeated.
            None if !(start + 1..end).any(|i| starts_word(units, i)) => back,
            None => end,
        }
    }
}

/// Tokens a blank-line join adds between two words, as this tokenizer counts
/// it. Tokens added to every call (BOS/EOS) are counted once, not twice.
fn join_cost(count_tokens: &dyn Fn(&str) -> usize) -> usize {
    let word = count_tokens("a");
    (count_tokens("a\n\na") + count_tokens("")).saturating_sub(2 * word)
}

/// `units[i]` starts a word (or `i` is the start or end).
fn starts_word(units: &[(usize, usize)], i: usize) -> bool {
    i == 0 || i == units.len() || units[i - 1].1 != units[i].0
}

/// `end`, moved back to the last word boundary so an ordinary long word
/// (`değerlendirilmesinin`) is not cut between windows, but only if that keeps
/// at least half the window and still moves past the previous window's end;
/// otherwise (a short run before a long blob) the window keeps its full size.
fn word_end(units: &[(usize, usize)], start: usize, end: usize, prev_end: usize) -> usize {
    let min_end = (start + (end - start).div_ceil(2)).max(prev_end + 1);
    (min_end..=end)
        .rev()
        .find(|&e| starts_word(units, e))
        .unwrap_or(end)
}

/// Largest `n` in `[lo, hi]` with `fits(n)`, for a predicate that holds up to
/// some point and fails after it; `lo` is returned if no larger `n` fits.
/// Exponential probing from `lo` keeps each search near the answer, so
/// splitting a long text costs ~ length × log(window), not quadratic.
fn last_fitting(lo: usize, hi: usize, fits: impl Fn(usize) -> bool) -> usize {
    let mut good = lo;
    let mut step = 1;
    loop {
        let probe = (good + step).min(hi);
        if probe == good {
            return good;
        }
        if fits(probe) {
            good = probe;
            step *= 2;
            continue;
        }
        let (mut lo, mut hi) = (good, probe - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(mid) { lo = mid } else { hi = mid - 1 }
        }
        return lo;
    }
}

/// Byte spans of whitespace-separated words; words longer than
/// `MAX_UNIT_GRAPHEMES` are cut at grapheme boundaries into adjacent spans, so
/// no cut separates a letter from its accent or splits a flag or emoji.
fn unit_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut word_start: Option<usize> = None;
    for (index, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                push_word_units(text, start, index, &mut spans);
            }
        } else if word_start.is_none() {
            word_start = Some(index);
        }
    }
    spans
}

fn push_word_units(text: &str, start: usize, end: usize, spans: &mut Vec<(usize, usize)>) {
    let mut unit_start = start;
    for (count, (offset, _)) in text[start..end].grapheme_indices(true).enumerate() {
        if count > 0 && count % MAX_UNIT_GRAPHEMES == 0 {
            spans.push((unit_start, start + offset));
            unit_start = start + offset;
        }
    }
    spans.push((unit_start, end));
}

#[cfg(test)]
mod tests;
