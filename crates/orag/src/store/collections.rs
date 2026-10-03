//! Collections: named, isolated document sets (D-014).

use rusqlite::{OptionalExtension, Row, TransactionBehavior};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

use crate::domain::CollectionId;
use crate::error::{OragError, Result};
use crate::store::Store;

pub const DEFAULT_COLLECTION: &str = "default";
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Collection {
    pub id: CollectionId,
    pub name: String,
    pub document_count: i64,
    pub created_at: String,
}

const SELECT_COLLECTION: &str = "SELECT c.id, c.name, \
     (SELECT COUNT(*) FROM documents d WHERE d.collection_id = c.id), c.created_at \
     FROM collections c";

fn collection_from_row(row: &Row<'_>) -> rusqlite::Result<Collection> {
    Ok(Collection {
        id: row.get(0)?,
        name: row.get(1)?,
        document_count: row.get(2)?,
        created_at: row.get(3)?,
    })
}

/// Case-insensitive comparison form of a name: NFC, Turkish `İ`/`I`/`ı` all
/// as `i`, then lowercase. Names are compared, never stored, in this form, so
/// it does not depend on the (versioned) lexical normalizer.
fn name_fold(name: &str) -> String {
    name.nfc()
        .map(|c| {
            if matches!(c, 'İ' | 'I' | 'ı') {
                'i'
            } else {
                c
            }
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// 1–64 characters: letters, digits, space, `_`, `.`, `-`; no leading/trailing space.
pub fn validate_collection_name(name: &str) -> Result<()> {
    let length = name.chars().count();
    let allowed = |c: char| {
        (c.is_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-'))
            && !crate::domain::normalize::is_default_ignorable(c)
    };
    if length == 0 || length > MAX_NAME_CHARS || name.trim() != name || !name.chars().all(allowed) {
        return Err(OragError::InvalidInput(format!(
            "collection name must be 1-{MAX_NAME_CHARS} letters, digits, spaces, '_', '.' or '-' without surrounding spaces"
        )));
    }
    Ok(())
}

impl Store {
    /// Creates a collection. The name is stored in NFC; a name equal to an
    /// existing one ignoring case (`Arşiv`, `ARŞİV`, see `name_fold`) is a
    /// conflict.
    pub fn create_collection(&self, name: &str) -> Result<Collection> {
        let name: String = name.nfc().collect();
        validate_collection_name(&name)?;
        let fold = name_fold(&name);
        let conflict = || OragError::Conflict(format!("collection `{name}` already exists"));
        self.write(|conn| {
            // All writes go through this one connection, and the check and the
            // insert share an IMMEDIATE transaction, so no other insert can
            // slip between them.
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let names: Vec<String> = tx
                .prepare("SELECT name FROM collections")?
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            if names.iter().any(|existing| name_fold(existing) == fold) {
                return Err(conflict());
            }
            tx.execute("INSERT INTO collections (name) VALUES (?1)", [&name])?;
            let id = tx.last_insert_rowid();
            let created = tx.query_row(
                &format!("{SELECT_COLLECTION} WHERE c.id = ?1"),
                [id],
                collection_from_row,
            )?;
            tx.commit()?;
            Ok(created)
        })
    }

    pub fn list_collections(&self) -> Result<Vec<Collection>> {
        let conn = self.read()?;
        let mut stmt = conn.prepare(&format!("{SELECT_COLLECTION} ORDER BY c.id"))?;
        let rows = stmt.query_map([], collection_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_collection(&self, id: CollectionId) -> Result<Collection> {
        let conn = self.read()?;
        conn.query_row(
            &format!("{SELECT_COLLECTION} WHERE c.id = ?1"),
            [id],
            collection_from_row,
        )
        .optional()?
        .ok_or(OragError::NotFound {
            kind: "collection",
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::error::OragError;
    use crate::store::testing::temp_store;

    #[test]
    fn create_list_get() {
        let (_dir, store) = temp_store();
        let created = store.create_collection("Hukuk Arşivi").unwrap();
        assert_eq!(created.document_count, 0);
        let names: Vec<String> = store
            .list_collections()
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["default", "Hukuk Arşivi"]);
        assert_eq!(
            store.get_collection(created.id).unwrap().name,
            "Hukuk Arşivi"
        );
    }

    #[test]
    fn duplicate_name_is_conflict() {
        let (_dir, store) = temp_store();
        store.create_collection("x").unwrap();
        assert!(matches!(
            store.create_collection("x"),
            Err(OragError::Conflict(_))
        ));
    }

    #[test]
    fn invalid_names_are_rejected() {
        let (_dir, store) = temp_store();
        for bad in [
            "",
            " lead",
            "trail ",
            "a/b",
            "x".repeat(65).as_str(),
            "new\nline",
        ] {
            assert!(
                matches!(
                    store.create_collection(bad),
                    Err(OragError::InvalidInput(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn missing_collection_is_not_found() {
        let (_dir, store) = temp_store();
        assert!(matches!(
            store.get_collection(42),
            Err(OragError::NotFound {
                kind: "collection",
                id: 42
            })
        ));
    }

    #[test]
    fn names_are_nfc_normalized_and_unique_ignoring_case() {
        let (_dir, store) = temp_store();
        let decomposed = "Ars\u{0327}iv";
        assert_eq!(store.create_collection(decomposed).unwrap().name, "Arşiv");
        assert!(matches!(
            store.create_collection("Arşiv"),
            Err(OragError::Conflict(_))
        ));
        assert!(matches!(
            store.create_collection("ARŞİV"),
            Err(OragError::Conflict(_))
        ));
        assert!(matches!(
            store.create_collection("DEFAULT"),
            Err(OragError::Conflict(_))
        ));
    }

    #[test]
    fn invisible_letters_are_refused_in_names() {
        let (_dir, store) = temp_store();
        for bad in ["\u{3164}", "abc\u{3164}", "a\u{115F}b", "x\u{200B}y"] {
            assert!(
                matches!(
                    store.create_collection(bad),
                    Err(OragError::InvalidInput(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_name_fold_is_fixed() {
        // Stored names are compared with this fold; it must never change.
        assert_eq!(super::name_fold("ARŞİV"), "arşiv");
        assert_eq!(super::name_fold("Iğdır"), "iğdir");
        assert_eq!(super::name_fold("Ars\u{0327}iv"), "arşiv");
    }
}
