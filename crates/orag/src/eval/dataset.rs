//! Labeled evaluation queries (JSONL). Labels are (document, exact substring)
//! so they survive chunker changes (D-018).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use crate::error::{OragError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relevant {
    pub document: String,
    pub contains: String,
}

/// Question kinds of the v0.2 set (Task 42); older sets have none.
pub const CATEGORIES: [&str; 8] = [
    "factual",
    "cross_language",
    "multi_document",
    "identifier",
    "table",
    "accentless",
    "inflection",
    "unanswerable",
];
/// `dev` tunes, `holdout` confirms; split by document family.
pub const SPLITS: [&str; 2] = ["dev", "holdout"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalQuery {
    pub id: String,
    pub lang: String,
    pub query: String,
    #[serde(default)]
    pub relevant: Vec<Relevant>,
    pub answerable: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub split: Option<String>,
    /// Documents the question belongs to; paraphrases of one fact share it.
    #[serde(default)]
    pub family: Option<String>,
    /// Found only when every label is found, not any one of them.
    #[serde(default)]
    pub require_all: bool,
}

impl EvalQuery {
    pub fn category_name(&self) -> &str {
        self.category.as_deref().unwrap_or("uncategorized")
    }

    pub fn split_name(&self) -> &str {
        self.split.as_deref().unwrap_or("unsplit")
    }
}

/// Checks the optional v0.2 fields shared by queries and answer probes.
pub(crate) fn check_grouping(
    category: Option<&str>,
    split: Option<&str>,
    answerable: bool,
) -> std::result::Result<(), String> {
    if let Some(category) = category {
        if !CATEGORIES.contains(&category) {
            return Err(format!(
                "unknown category {category:?} (one of {})",
                CATEGORIES.join(", ")
            ));
        }
        if (category == "unanswerable") == answerable {
            return Err(
                "category unanswerable is for unanswerable questions, and only for them".into(),
            );
        }
    }
    match split {
        Some(split) if !SPLITS.contains(&split) => Err(format!(
            "unknown split {split:?} (one of {})",
            SPLITS.join(", ")
        )),
        _ => Ok(()),
    }
}

/// Keeps dev and holdout apart: a family, and a piece of evidence, belongs to
/// one split only, or tuning on dev would leak into holdout.
#[derive(Default)]
pub(crate) struct SplitGuard {
    seen: HashMap<String, String>,
}

impl SplitGuard {
    fn claim(&mut self, key: String, split: &str) -> std::result::Result<(), String> {
        match self.seen.get(&key) {
            Some(other) if other != split => Err(format!("{key} is in both {other} and {split}")),
            Some(_) => Ok(()),
            None => {
                self.seen.insert(key, split.to_string());
                Ok(())
            }
        }
    }

    pub(crate) fn family(
        &mut self,
        family: Option<&str>,
        split: Option<&str>,
    ) -> std::result::Result<(), String> {
        match (family, split) {
            (Some(family), Some(split)) => self.claim(format!("family {family:?}"), split),
            _ => Ok(()),
        }
    }

    fn place(&mut self, query: &EvalQuery) -> std::result::Result<(), String> {
        self.family(query.family.as_deref(), query.split.as_deref())?;
        let Some(split) = query.split.as_deref() else {
            return Ok(());
        };
        for label in &query.relevant {
            self.claim(
                format!("evidence {:?} of {}", label.contains, label.document),
                split,
            )?;
        }
        Ok(())
    }
}

pub fn load_dataset(path: &Path) -> Result<Vec<EvalQuery>> {
    let text = std::fs::read_to_string(path)?;
    let mut queries = Vec::new();
    let mut ids = HashSet::new();
    let mut splits = SplitGuard::default();
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
        check_grouping(
            query.category.as_deref(),
            query.split.as_deref(),
            query.answerable,
        )
        .map_err(at)?;
        if query.require_all && query.relevant.len() < 2 {
            return Err(at("require_all needs at least two relevant labels".into()));
        }
        if query.category.is_some()
            && query.require_all != (query.category_name() == "multi_document")
        {
            return Err(at(
                "require_all is for multi_document questions, and every one of them needs it"
                    .into(),
            ));
        }
        splits.place(&query).map_err(at)?;
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

    fn jsonl(lines: &[&str]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        (dir, path)
    }

    #[test]
    fn v02_fields_load_and_older_rows_default() {
        let (_dir, path) = jsonl(&[
            r#"{"id":"a","lang":"tr","category":"table","split":"dev","family":"sulama","query":"q","relevant":[{"document":"d.md","contains":"x"}],"answerable":true}"#,
            r#"{"id":"b","lang":"tr","query":"q","relevant":[{"document":"d.md","contains":"x"}],"answerable":true}"#,
        ]);
        let queries = load_dataset(&path).unwrap();
        assert_eq!(queries[0].family.as_deref(), Some("sulama"));
        assert_eq!(
            (queries[0].category_name(), queries[0].split_name()),
            ("table", "dev")
        );
        assert_eq!(
            (queries[1].category_name(), queries[1].split_name()),
            ("uncategorized", "unsplit")
        );
        assert!(!queries[1].require_all);
    }

    #[test]
    fn unknown_category_or_split_is_rejected_with_its_line() {
        let row = |category: &str, split: &str| {
            format!(
                r#"{{"id":"a","lang":"tr","category":"{category}","split":"{split}","query":"q","relevant":[{{"document":"d.md","contains":"x"}}],"answerable":true}}"#
            )
        };
        let (_dir, path) = jsonl(&[&row("misc", "dev")]);
        let err = load_dataset(&path).unwrap_err().to_string();
        assert!(err.contains("line 1") && err.contains("misc"), "{err}");
        let (_dir, path) = jsonl(&[&row("table", "train")]);
        let err = load_dataset(&path).unwrap_err().to_string();
        assert!(err.contains("line 1") && err.contains("train"), "{err}");
    }

    #[test]
    fn the_unanswerable_category_must_match_the_flag() {
        let (_dir, path) = jsonl(&[
            r#"{"id":"a","lang":"tr","category":"unanswerable","query":"q","relevant":[{"document":"d.md","contains":"x"}],"answerable":true}"#,
        ]);
        assert!(
            load_dataset(&path)
                .unwrap_err()
                .to_string()
                .contains("unanswerable")
        );
        let (_dir, path) = jsonl(&[
            r#"{"id":"a","lang":"tr","category":"factual","query":"q","relevant":[],"answerable":false}"#,
        ]);
        assert!(
            load_dataset(&path)
                .unwrap_err()
                .to_string()
                .contains("unanswerable")
        );
    }

    #[test]
    fn require_all_needs_two_labels() {
        let (_dir, path) = jsonl(&[
            r#"{"id":"a","lang":"tr","query":"q","relevant":[{"document":"d.md","contains":"x"}],"answerable":true,"require_all":true}"#,
        ]);
        let err = load_dataset(&path).unwrap_err().to_string();
        assert!(err.contains("require_all"), "{err}");
    }

    #[test]
    fn v02_dataset_has_the_planned_mix_and_families_stay_in_one_split() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/datasets/v02-retrieval.jsonl");
        let queries = load_dataset(&path).unwrap();
        assert_eq!(queries.len(), 200);
        let planned = [
            ("factual", 40),
            ("cross_language", 25),
            ("multi_document", 20),
            ("identifier", 25),
            ("table", 25),
            ("accentless", 20),
            ("inflection", 20),
            ("unanswerable", 25),
        ];
        let mut splits: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for (category, count) in planned {
            let rows: Vec<&EvalQuery> = queries
                .iter()
                .filter(|q| q.category_name() == category)
                .collect();
            assert_eq!(rows.len(), count, "{category}");
            let dev = rows.iter().filter(|q| q.split_name() == "dev").count();
            assert!(
                dev * 10 >= count * 4 && (count - dev) * 10 >= count * 4,
                "{category}: {dev} of {count} in dev"
            );
        }
        for query in &queries {
            assert!(query.split.is_some(), "{} has no split", query.id);
            let family = query
                .family
                .as_deref()
                .expect("every v0.2 row has a family");
            let split = splits.entry(family).or_insert(query.split_name());
            assert_eq!(
                *split,
                query.split_name(),
                "family {family} is in both splits"
            );
            assert_eq!(
                query.require_all,
                query.category_name() == "multi_document",
                "{}",
                query.id
            );
        }
    }

    #[test]
    fn a_family_or_evidence_in_both_splits_is_rejected() {
        let row = |id: &str, split: &str, family: &str, contains: &str| {
            format!(
                r#"{{"id":"{id}","lang":"tr","category":"factual","split":"{split}","family":"{family}","query":"q","relevant":[{{"document":"d.md","contains":"{contains}"}}],"answerable":true}}"#
            )
        };
        let (_dir, path) = jsonl(&[&row("a", "dev", "f", "x"), &row("b", "holdout", "f", "y")]);
        let err = load_dataset(&path).unwrap_err().to_string();
        assert!(err.contains("line 2") && err.contains("family"), "{err}");
        let (_dir, path) = jsonl(&[&row("a", "dev", "f", "x"), &row("b", "holdout", "g", "x")]);
        let err = load_dataset(&path).unwrap_err().to_string();
        assert!(err.contains("line 2") && err.contains("evidence"), "{err}");
        let (_dir, path) = jsonl(&[&row("a", "dev", "f", "x"), &row("b", "dev", "g", "x")]);
        assert_eq!(load_dataset(&path).unwrap().len(), 2);
    }

    #[test]
    fn require_all_belongs_to_multi_document_questions() {
        let (_dir, path) = jsonl(&[
            r#"{"id":"a","lang":"tr","category":"multi_document","query":"q","relevant":[{"document":"d.md","contains":"x"},{"document":"e.md","contains":"y"}],"answerable":true}"#,
        ]);
        assert!(
            load_dataset(&path)
                .unwrap_err()
                .to_string()
                .contains("require_all")
        );
    }

    #[test]
    fn v02_probes_use_the_split_of_their_retrieval_family() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../eval/datasets");
        let queries = load_dataset(&root.join("v02-retrieval.jsonl")).unwrap();
        let probes = crate::eval::answers::load_probes(&root.join("v02-answers.jsonl")).unwrap();
        let mut guard = SplitGuard::default();
        for query in &queries {
            guard
                .family(query.family.as_deref(), query.split.as_deref())
                .unwrap();
        }
        for probe in &probes {
            guard
                .family(probe.family.as_deref(), probe.split.as_deref())
                .unwrap_or_else(|err| panic!("{}: {err}", probe.id));
        }
    }
}
