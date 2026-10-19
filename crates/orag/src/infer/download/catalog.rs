//! The models the setup page can download (step 76): the presets of
//! `scripts/fetch-model-pack.sh` that ship a plain LICENSE file, pinned to the
//! same revisions and SHA-256 hashes. A test diffs this table with the script.

use crate::error::Result;
use crate::infer::models::{ModelManifest, ModelRole};

/// The license name every catalog pack declares.
const LICENSE_NAME: &str = "Apache-2.0";
/// The license file name inside an installed pack.
pub const LICENSE_FILE: &str = "LICENSE";

/// One downloadable file: where it comes from and what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub url: String,
    /// File name inside the installed pack.
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

/// One downloadable model pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub role: ModelRole,
    /// Hugging Face repository, `owner/name`.
    pub repo: String,
    /// Pinned commit of `repo`.
    pub revision: String,
    pub file: String,
    pub sha256: String,
    /// Exact byte size of `file` at `revision`.
    pub size: u64,
    pub license_url: String,
    pub license_sha256: String,
    pub license_size: u64,
    /// The manifest's role table (`[embedding]` or `[generation]`), as TOML.
    pub role_section: String,
}

impl CatalogEntry {
    /// The weights, then the license: the order they are downloaded in.
    pub fn files(&self) -> [RemoteFile; 2] {
        [
            RemoteFile {
                url: format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    self.repo, self.revision, self.file
                ),
                name: self.file.clone(),
                sha256: self.sha256.clone(),
                size: self.size,
            },
            RemoteFile {
                url: self.license_url.clone(),
                name: LICENSE_FILE.to_string(),
                sha256: self.license_sha256.clone(),
                size: self.license_size,
            },
        ]
    }

    /// Bytes of every file of the pack.
    pub fn total_size(&self) -> u64 {
        self.size + self.license_size
    }

    /// The manifest `scripts/fetch-model-pack.sh` writes for this preset,
    /// parsed and validated like any imported pack.
    pub fn manifest(&self) -> Result<ModelManifest> {
        ModelManifest::parse(&self.manifest_toml())
    }

    fn manifest_toml(&self) -> String {
        format!(
            "id = \"{}\"\nrole = \"{}\"\nfile = \"{}\"\nsha256 = \"{}\"\nlicense = \"{LICENSE_NAME}\"\n\
             license_file = \"{LICENSE_FILE}\"\nsource = \"https://huggingface.co/{}\"\nrevision = \"{}\"\n\n{}\n",
            self.id,
            self.role.as_str(),
            self.file,
            self.sha256,
            self.repo,
            self.revision,
            self.role_section
        )
    }
}

/// The models a configured id can be downloaded as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn new(entries: Vec<CatalogEntry>) -> Catalog {
        Catalog { entries }
    }

    /// The catalog built into the binary.
    pub fn builtin() -> Catalog {
        Catalog::new(builtin_entries())
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn find(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}

const QWEN3_EMBEDDING_SECTION: &str = "[embedding]\ndimensions = 1024\npooling = \"last\"\nmax_tokens = 1024\n\
query_prefix = \"Instruct: Given a search query, retrieve relevant passages that answer the query\\nQuery: \"\n\
document_prefix = \"\"\nrequire_trailing_eos = true";

#[allow(clippy::too_many_arguments)]
fn entry(
    id: &str,
    role: ModelRole,
    repo: &str,
    revision: &str,
    file: &str,
    sha256: &str,
    size: u64,
    license: (&str, &str, u64),
    role_section: &str,
) -> CatalogEntry {
    let (license_url, license_sha256, license_size) = license;
    CatalogEntry {
        id: id.into(),
        role,
        repo: repo.into(),
        revision: revision.into(),
        file: file.into(),
        sha256: sha256.into(),
        size,
        license_url: license_url.into(),
        license_sha256: license_sha256.into(),
        license_size,
        role_section: role_section.into(),
    }
}

/// Sizes are the `x-linked-size` Hugging Face reports at each revision.
fn builtin_entries() -> Vec<CatalogEntry> {
    vec![
        entry(
            "qwen3-embedding-0.6b-q8_0",
            ModelRole::Embedding,
            "Qwen/Qwen3-Embedding-0.6B-GGUF",
            "370f27d7550e0def9b39c1f16d3fbaa13aa67728",
            "Qwen3-Embedding-0.6B-Q8_0.gguf",
            "06507c7b42688469c4e7298b0a1e16deff06caf291cf0a5b278c308249c3e439",
            639_150_592,
            (
                "https://www.apache.org/licenses/LICENSE-2.0.txt",
                "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
                11_358,
            ),
            QWEN3_EMBEDDING_SECTION,
        ),
        entry(
            "qwen3.5-4b-q4_k_m",
            ModelRole::Generation,
            "unsloth/Qwen3.5-4B-GGUF",
            "e87f176479d0855a907a41277aca2f8ee7a09523",
            "Qwen3.5-4B-Q4_K_M.gguf",
            "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
            2_740_937_888,
            (
                "https://huggingface.co/Qwen/Qwen3.5-4B/resolve/851bf6e806efd8d0a36b00ddf55e13ccb7b8cd0a/LICENSE",
                "bbedc3fda3305820b977265f01b8619d87570a6739de3a5582c3464840f1e57a",
                11_544,
            ),
            "[generation]\ncontext_tokens = 8192\nmax_output_tokens = 1024\nprompt_format = \"chatml-nothink\"",
        ),
        entry(
            "qwen3-4b-instruct-2507-q4_k_m",
            ModelRole::Generation,
            "unsloth/Qwen3-4B-Instruct-2507-GGUF",
            "a06e946bb6b655725eafa393f4a9745d460374c9",
            "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
            2_497_281_120,
            (
                "https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507/resolve/cdbee75f17c01a7cc42f958dc650907174af0554/LICENSE",
                "832dd9e00a68dd83b3c3fb9f5588dad7dcf337a0db50f7d9483f310cd292e92e",
                11_343,
            ),
            "[generation]\ncontext_tokens = 8192\nmax_output_tokens = 1024\nprompt_format = \"chatml\"",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// `$'...'` (ANSI-C quoting) or `"..."` as bash reads it.
    fn unquote(value: &str) -> String {
        let Some(inner) = value.strip_prefix("$'").and_then(|v| v.strip_suffix('\'')) else {
            return value.trim_matches('"').to_string();
        };
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match (c, c == '\\') {
                (_, true) => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                },
                (c, false) => out.push(c),
            }
        }
        out
    }

    /// Each preset of the script's `case` with its `key=value` assignments.
    fn script_presets() -> HashMap<String, HashMap<String, String>> {
        let script = include_str!("../../../../../scripts/fetch-model-pack.sh");
        let body =
            &script[script.find("case \"$preset\" in").unwrap()..script.find("\nesac").unwrap()];
        let mut presets = HashMap::new();
        let mut current: Option<(String, HashMap<String, String>)> = None;
        for line in body.lines() {
            if let Some(name) = line.strip_prefix("  ").and_then(|l| l.strip_suffix(')'))
                && !name.starts_with(' ')
            {
                current = Some((name.to_string(), HashMap::new()));
            } else if line.trim() == ";;" {
                presets.extend(current.take());
            } else if let (Some((_, fields)), Some((key, value))) =
                (current.as_mut(), line.trim().split_once('='))
            {
                fields.insert(key.to_string(), unquote(value));
            }
        }
        presets
    }

    #[test]
    fn the_catalog_pins_exactly_what_the_pack_script_pins() {
        let presets = script_presets();
        let catalog = Catalog::builtin();
        assert_eq!(catalog.entries().len(), 3);
        for entry in catalog.entries() {
            let script = presets
                .get(&entry.id)
                .unwrap_or_else(|| panic!("{} is not a preset of the script", entry.id));
            let field = |key: &str| script.get(key).map(String::as_str).unwrap_or_default();
            assert_eq!(field("repo"), entry.repo, "{}", entry.id);
            assert_eq!(field("revision"), entry.revision, "{}", entry.id);
            assert_eq!(field("file"), entry.file, "{}", entry.id);
            assert_eq!(field("sha"), entry.sha256, "{}", entry.id);
            assert_eq!(field("license_url"), entry.license_url, "{}", entry.id);
            assert_eq!(field("license_sha"), entry.license_sha256, "{}", entry.id);
            assert_eq!(field("role"), entry.role.as_str(), "{}", entry.id);
            assert_eq!(field("role_section"), entry.role_section, "{}", entry.id);
            assert!(
                !script.contains_key("mit_holder"),
                "{} needs a generated MIT text",
                entry.id
            );
        }
        for generated in ["bge-m3-q8_0", "multilingual-e5-large-instruct-q8_0"] {
            assert!(
                presets.contains_key(generated),
                "the parser missed {generated}"
            );
            assert!(
                catalog.find(generated).is_none(),
                "{generated} has no LICENSE file"
            );
        }
    }

    #[test]
    fn every_entry_yields_a_valid_manifest() {
        for entry in Catalog::builtin().entries() {
            let manifest = entry.manifest().unwrap();
            assert_eq!(
                (manifest.id.as_str(), manifest.role, manifest.file.as_str()),
                (entry.id.as_str(), entry.role, entry.file.as_str())
            );
            assert_eq!(manifest.license_file, LICENSE_FILE);
            assert_eq!(
                manifest.source,
                format!("https://huggingface.co/{}", entry.repo)
            );
            assert!(
                entry.size > 100_000_000 && entry.license_size > 10_000,
                "{}",
                entry.id
            );
        }
        let embedding = Catalog::builtin()
            .find("qwen3-embedding-0.6b-q8_0")
            .unwrap()
            .manifest()
            .unwrap();
        let spec = embedding.embedding.unwrap();
        assert!(
            spec.query_prefix.ends_with("the query\nQuery: "),
            "{:?}",
            spec.query_prefix
        );
        assert!(spec.require_trailing_eos);
    }
}
