//! Minimal normalized document representation ("AST-lite") for v0.1.
//! Richer nodes (pages, figures, formulas, spans) arrive with PDF in v0.2.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    ListItem(String),
    Code(String),
    /// Rows rendered as `cell | cell`, one row per line, header first.
    Table(String),
}

impl Block {
    pub fn text(&self) -> &str {
        match self {
            Block::Heading { text, .. } => text,
            Block::Paragraph(text)
            | Block::ListItem(text)
            | Block::Code(text)
            | Block::Table(text) => text,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedDocument {
    pub title: Option<String>,
    pub blocks: Vec<Block>,
    /// Extraction warnings surfaced on the document (e.g. `ocr_required`).
    pub warnings: Vec<String>,
}
