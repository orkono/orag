//! Labeled evaluation queries (JSONL). Labels are (document, exact substring)
//! so they survive chunker changes (D-018).

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;

use crate::error::{OragError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relevant {
    pub document: String,
    pub contains: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalQuery {
    pub id: String,
    pub lang: String,
    pub query: String,
    #[serde(default)]
    pub relevant: Vec<Relevant>,
    pub answerable: bool,
}

pub fn load_dataset(path: &Path) -> Result<Vec<EvalQuery>> {
    let text = std::fs::read_to_string(path)?;
    let mut queries = Vec::new();
    let mut ids = HashSet::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let at = |msg: String| {
            OragError::InvalidInput(format!("{} line {}: {msg}", path.display(), index + 1))
        };
        let query: EvalQuery = serde_json::from_str(line).map_err(|e| at(e.to_string()))?;
        if query.answerable == query.relevant.is_empty() {
            return Err(at(
                "answerable queries need relevant labels; unanswerable ones must have none".into(),
            ));
        }
        if !ids.insert(query.id.clone()) {
            return Err(at(format!("duplicate id {}", query.id)));
        }
        queries.push(query);
    }
    Ok(queries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_dataset_loads_and_is_consistent() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/datasets/seed.jsonl");
        let queries = load_dataset(&path).unwrap();
        assert_eq!(queries.len(), 16);
        assert_eq!(queries.iter().filter(|q| !q.answerable).count(), 2);
    }

    #[test]
    fn anayasa_dataset_loads_and_is_consistent() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/datasets/anayasa-tr.jsonl");
        let queries = load_dataset(&path).unwrap();
        assert_eq!(queries.len(), 22);
        assert_eq!(queries.iter().filter(|q| !q.answerable).count(), 2);
    }

    #[test]
    fn inconsistent_rows_are_rejected_with_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.jsonl");
        std::fs::write(
            &path,
            "{\"id\":\"a\",\"lang\":\"en\",\"query\":\"q\",\"relevant\":[],\"answerable\":true}\n",
        )
        .unwrap();
        let err = load_dataset(&path).unwrap_err().to_string();
        assert!(err.contains("line 1"), "{err}");
    }
}
