//! Heading lines in plain text and PDF page text, which carry no markup.
//!
//! Only lines that read as titles count: short, not ending like a sentence, and
//! either written in capitals, framed as `-- name --`, or numbered as an
//! outline with a Roman numeral or a letter (`IV.`, `A.`). Levels nest the way
//! Turkish statutes and catalogues do: part > chapter > capitals > Roman >
//! letter. Decimal numbering (`1.`, `2.3`) is left as body text: in manuals it
//! numbers steps far more often than titles, and it gained nothing in the
//! step 52 measurement.

/// Longer lines are prose wrapped by the source, not titles.
const MAX_CHARS: usize = 80;
const MAX_WORDS: usize = 12;
/// Capitals with fewer letters are acronyms or codes (`KDV`, `PN 10`).
const MIN_LETTERS: usize = 4;

const PART_WORDS: [&str; 2] = ["KISIM", "PART"];
const CHAPTER_WORDS: [&str; 3] = ["BÖLÜM", "CHAPTER", "SECTION"];

/// Heading level of `line`, or `None` for body text.
pub(crate) fn heading_level(line: &str) -> Option<u8> {
    let line = line.trim();
    let words = line.split_whitespace().count();
    if words == 0
        || words > MAX_WORDS
        || line.chars().count() > MAX_CHARS
        || has_columns(line)
        || line.ends_with(['.', ',', ';', ':', '!', '?'])
    {
        return None;
    }
    dashed(line)
        .or_else(|| capitals(line))
        .or_else(|| enumerated(line))
}

/// Runs of spaces, a tab or a `|` cell separator: a table row, not a title.
fn has_columns(line: &str) -> bool {
    line.contains('\t') || line.contains("   ") || line.contains(" | ")
}

/// `-- name --`, the section marker of plain-text lists.
fn dashed(line: &str) -> Option<u8> {
    let inner = line.strip_prefix("--")?.strip_suffix("--")?;
    (letters(inner) >= MIN_LETTERS).then_some(3)
}

/// A line without a single lower-case letter, ranked by a part or chapter word.
fn capitals(line: &str) -> Option<u8> {
    if letters(line) < MIN_LETTERS || line.chars().any(char::is_lowercase) {
        return None;
    }
    let has = |keywords: &[&str]| {
        line.split(|c: char| !c.is_alphanumeric())
            .any(|word| keywords.contains(&word))
    };
    Some(if has(&PART_WORDS) {
        1
    } else if has(&CHAPTER_WORDS) {
        2
    } else {
        3
    })
}

/// A Roman numeral or a letter followed by a capitalised title.
fn enumerated(line: &str) -> Option<u8> {
    let (marker, title) = line.split_once(char::is_whitespace)?;
    if !title.trim_start().starts_with(char::is_uppercase) || letters(title) < 2 {
        return None;
    }
    let roman = marker.strip_suffix('.').filter(|body| {
        !body.is_empty()
            && body.len() <= 6
            && body.chars().all(|c| "IVXLC".contains(c))
            && (body.len() > 1 || "IVX".contains(body))
    });
    if roman.is_some() {
        return Some(4);
    }
    let mut chars = marker.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some(letter), Some('.'), None) if letter.is_uppercase() => Some(5),
        _ => None,
    }
}

fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_case_lines_are_headings_ranked_by_their_keyword() {
        assert_eq!(heading_level("BİRİNCİ KISIM  "), Some(1));
        assert_eq!(heading_level("PART ONE"), Some(1));
        assert_eq!(heading_level("İKİNCİ BÖLÜM"), Some(2));
        assert_eq!(heading_level("BÖLÜM 2 - BAĞLANTI PARÇALARI"), Some(2));
        assert_eq!(heading_level("CHAPTER 3 - INSTALLATION"), Some(2));
        assert_eq!(heading_level("HAKKIMIZDA"), Some(3));
        assert_eq!(heading_level("TÜRK MİLLETİ ADINA"), Some(3));
        assert_eq!(
            heading_level("DS-1640 YASSI DAMLA SULAMA ŞERİDİ 16 MM, 8 MİL"),
            Some(3)
        );
    }

    #[test]
    fn dashed_section_lines_are_headings() {
        assert_eq!(heading_level("-- PE 100 ANA HAT BORULARI --"), Some(3));
        assert_eq!(heading_level("-- Bağlantı parçaları --"), Some(3));
    }

    #[test]
    fn enumerated_titles_nest_roman_then_letter() {
        assert_eq!(heading_level("IV. Değiştirilemeyecek hükümler "), Some(4));
        assert_eq!(heading_level("I. Devletin şekli"), Some(4));
        assert_eq!(heading_level("A. Özel hayatın gizliliği"), Some(5));
        assert_eq!(heading_level("C. Haberleşme hürriyeti"), Some(5));
        assert_eq!(heading_level("Ç. Seyahat hürriyeti"), Some(5));
    }

    #[test]
    fn body_lines_are_not_headings() {
        for line in [
            "",
            "   ",
            "MADDE 1- Türkiye Devleti bir Cumhuriyettir.",
            "Türk Vatanı ve Milletinin ebedi varlığını ve Yüce Türk Devletinin",
            // Mixed case, even when it starts in capitals.
            "TÜRK MİLLETİ TARAFINDAN, demokrasiye âşık Türk evlatlarının vatan",
            // Column layout: a table header or row, not a heading.
            "KOD        ÜRÜN ADI                     BİRİM     FİYAT (TL)",
            "DS-1600    Yuvarlak damla sulama borusu 16 mm    m     4,20",
            "---------  --------------------------------  -------",
            "DS-1840 | 40 | PN 10 | 27,60",
            // Decimal numbering numbers steps in manuals; it stays body text.
            "1. Milletin temsili",
            "2.3 Kapsam ve tanımlar",
            // A sentence, even a short numbered one.
            "1. Pompayı çalıştırmadan önce vanayı açın.",
            "A. Bu madde, kanunla sınırlanabilir;",
            // A footnote, a page number, an acronym.
            "1 Bu Anayasa; Kurucu Meclis tarafından kabul edilmiştir",
            "2 ",
            "KDV",
            "10/6/26, 9:06 AM about:blank",
            // A garbled title that ends in a sentence mark.
            "DENİZ SULAMA SİSTEMLERİ A. DAMLA SULAMA ÜRÜN KATALOĞU Ş.",
            // Lower-case after the enumerator: a wrapped sentence.
            "2. maddesindeki Cumhuriyetin nitelikleri",
        ] {
            assert_eq!(heading_level(line), None, "{line:?}");
        }
    }

    #[test]
    fn long_lines_are_not_headings() {
        let caps = "ÇOK UZUN BİR SATIR ".repeat(6);
        assert_eq!(heading_level(&caps), None);
        let numbered = format!("IV. {}", "Kelime ".repeat(14));
        assert_eq!(heading_level(&numbered), None);
    }
}
