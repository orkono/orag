//! Minimal normalized document representation ("AST-lite") for v0.1.
//! Richer nodes (pages, figures, formulas, spans) arrive with PDF in v0.2.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParsedDocument {
    pub title: Option<String>,
    pub blocks: Vec<Block>,
    /// Extraction warnings surfaced on the document (e.g. `ocr_required`).
    pub warnings: Vec<String>,
    /// The 1-based physical PDF page of each block, parallel to `blocks`;
    /// empty for formats without pages. Provenance only: it never changes
    /// chunk boundaries or text. Missing in the output of an older parser
    /// child, hence the default.
    #[serde(default)]
    pub pages: Vec<Option<u32>>,
}

impl ParsedDocument {
    /// The page of block `index`, if the document has a page for every block.
    pub fn page_of(&self, index: usize) -> Option<u32> {
        if self.pages.len() == self.blocks.len() {
            self.pages[index]
        } else {
            None
        }
    }
}
