//! Offline model packs (D-012): manifest + weights + license, SHA-256 verified.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{OragError, Result};

pub const MANIFEST_FILE: &str = "orag-model.toml";

/// Smallest `context_tokens` a generation manifest may declare.
pub const MIN_CONTEXT_TOKENS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Embedding,
    Generation,
}

impl ModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelRole::Embedding => "embedding",
            ModelRole::Generation => "generation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pooling {
    Mean,
    Cls,
    Last,
}

impl Pooling {
    pub fn as_str(self) -> &'static str {
        match self {
            Pooling::Mean => "mean",
            Pooling::Cls => "cls",
            Pooling::Last => "last",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingSpec {
    pub dimensions: usize,
    pub pooling: Pooling,
    pub max_tokens: usize,
    #[serde(default)]
    pub query_prefix: String,
    #[serde(default)]
    pub document_prefix: String,
    #[serde(default)]
    pub require_trailing_eos: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptFormat {
    /// The GGUF's embedded template through llama.cpp's native formatter
    /// (plain transcript if the model has none).
    #[default]
    Native,
    /// ChatML rendered by ORAG.
    Chatml,
    /// ChatML with an empty `<think></think>` prefill: Qwen3/3.5 non-thinking
    /// mode (what the Jinja template does with `enable_thinking=false`).
    ChatmlNothink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSpec {
    pub context_tokens: usize,
    pub max_output_tokens: usize,
    #[serde(default)]
    pub prompt_format: PromptFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub id: String,
    pub role: ModelRole,
    pub file: String,
    pub sha256: String,
    pub license: String,
    pub license_file: String,
    pub source: String,
    pub revision: String,
    pub embedding: Option<EmbeddingSpec>,
    pub generation: Option<GenerationSpec>,
}

impl ModelManifest {
    pub fn parse(text: &str) -> Result<ModelManifest> {
        let manifest: ModelManifest = toml::from_str(text)
            .map_err(|e| OragError::InvalidInput(format!("{MANIFEST_FILE}: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        let invalid = |msg: String| Err(OragError::InvalidInput(format!("{MANIFEST_FILE}: {msg}")));
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 64
            && !self.id.starts_with('.')
            && self.id.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
            });
        if !id_ok {
            return invalid(format!(
                "id `{}` must be 1-64 chars of [a-z0-9._-]",
                self.id
            ));
        }
        for name in [&self.file, &self.license_file] {
            if !is_plain_file_name(name) || name == MANIFEST_FILE {
                return invalid(format!(
                    "`{name}` must be a plain file name inside the pack, other than {MANIFEST_FILE}"
                ));
            }
        }
        if self.file == self.license_file {
            return invalid("file and license_file must be different files".into());
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return invalid("sha256 must be 64 lowercase hex characters".into());
        }
        match (self.role, &self.embedding, &self.generation) {
            (ModelRole::Embedding, Some(spec), None) => {
                if !(1..=8192).contains(&spec.dimensions)
                    || !(16..=32768).contains(&spec.max_tokens)
                {
                    return invalid(
                        "embedding dimensions must be 1-8192 and max_tokens 16-32768".into(),
                    );
                }
            }
            (ModelRole::Generation, None, Some(spec)) => {
                if !(MIN_CONTEXT_TOKENS..=131072).contains(&spec.context_tokens)
                    || spec.max_output_tokens < 16
                    || spec.max_output_tokens > spec.context_tokens / 2
                {
                    return invalid(
                        "context_tokens must be 512-131072 and max_output_tokens 16..=context/2"
                            .into(),
                    );
                }
            }
            _ => {
                return invalid(
                    "role must match exactly one of [embedding] or [generation]".into(),
                );
            }
        }
        Ok(())
    }
}

fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\']) && name != ".."
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledModel {
    pub manifest: ModelManifest,
    pub dir: PathBuf,
}

impl InstalledModel {
    pub fn model_path(&self) -> PathBuf {
        self.dir.join(&self.manifest.file)
    }
}

pub fn read_manifest(dir: &Path) -> Result<ModelManifest> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| OragError::InvalidInput(format!("cannot read {}: {e}", path.display())))?;
    ModelManifest::parse(&text)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Installs a pack as `models_dir/<id>/`: copy into an exclusively created
/// staging directory, hash the weights as they are written, write the validated
/// manifest, then rename into place. The bytes checked are the bytes installed.
pub fn import_pack(pack_dir: &Path, models_dir: &Path) -> Result<ModelManifest> {
    let manifest = read_manifest(pack_dir)?;
    let dest = models_dir.join(&manifest.id);
    if dest.exists() {
        return Err(OragError::Conflict(format!(
            "model `{}` is already installed",
            manifest.id
        )));
    }
    std::fs::create_dir_all(models_dir)?;
    remove_stale_staging(models_dir);
    let staging = create_staging_dir(models_dir, &manifest.id)?;
    let result = stage_and_verify(pack_dir, &staging, &manifest).and_then(|()| {
        std::fs::rename(&staging, &dest).map_err(|e| {
            if dest.exists() {
                OragError::Conflict(format!("model `{}` is already installed", manifest.id))
            } else {
                e.into()
            }
        })
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result.map(|()| manifest)
}

/// A staging directory this old belongs to an import that crashed or was killed.
pub(crate) const STALE_STAGING_AGE: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

/// Removes staging directories left by interrupted imports. Younger ones may
/// belong to an import still running, so they are kept. Best effort: a
/// directory that cannot be inspected or removed only costs disk space.
fn remove_stale_staging(models_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(models_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_staging = name
            .to_str()
            .is_some_and(|n| n.starts_with('.') && n.contains(".importing-"));
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        if is_staging && age.is_some_and(|age| age > STALE_STAGING_AGE) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `create_dir` (not `_all`) fails if the name exists, so concurrent imports never share a directory.
fn create_staging_dir(models_dir: &Path, id: &str) -> Result<PathBuf> {
    for attempt in 0..100u32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let candidate = models_dir.join(format!(
            ".{id}.importing-{}-{nanos}-{attempt}",
            std::process::id()
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(OragError::Internal(
        "could not create a unique staging directory".into(),
    ))
}

fn stage_and_verify(pack_dir: &Path, staging: &Path, manifest: &ModelManifest) -> Result<()> {
    let license = pack_dir.join(&manifest.license_file);
    if !license.is_file() {
        return Err(OragError::InvalidInput(format!(
            "license file `{}` is missing",
            manifest.license_file
        )));
    }
    let weights = pack_dir.join(&manifest.file);
    if !weights.is_file() {
        return Err(OragError::InvalidInput(format!(
            "weights file `{}` is missing from the pack",
            manifest.file
        )));
    }
    let actual = copy_hashing(&weights, &staging.join(&manifest.file))?;
    std::fs::copy(&license, staging.join(&manifest.license_file))?;
    if actual != manifest.sha256 {
        return Err(OragError::InvalidInput(format!(
            "checksum mismatch for {}: expected {}, got {actual}",
            manifest.file, manifest.sha256
        )));
    }
    let serialized = toml::to_string(manifest)
        .map_err(|e| OragError::Internal(format!("manifest serialization: {e}")))?;
    std::fs::write(staging.join(MANIFEST_FILE), serialized)?;
    Ok(())
}

/// Copies `from` to `to` (which must not exist) and returns the SHA-256 of
/// the bytes written, so the weights are read once and the hash covers
/// exactly what was installed.
fn copy_hashing(from: &Path, to: &Path) -> Result<String> {
    use std::io::Write;
    let mut input = std::fs::File::open(from)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
    }
    output.sync_all()?;
    Ok(hex::encode(hasher.finalize()))
}

/// A model directory whose manifest cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrokenModel {
    pub dir: PathBuf,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModelListing {
    pub installed: Vec<InstalledModel>,
    /// Reported, not fatal: one bad directory must not hide the good ones.
    pub broken: Vec<BrokenModel>,
}

pub fn list_models(models_dir: &Path) -> Result<ModelListing> {
    let entries = match std::fs::read_dir(models_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModelListing::default());
        }
        Err(err) => return Err(err.into()),
    };
    let mut listing = ModelListing::default();
    for entry in entries {
        let dir = entry?.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') || !dir.join(MANIFEST_FILE).is_file() {
            continue;
        }
        match load_installed(&dir, &name) {
            Ok(model) => listing.installed.push(model),
            Err(err) => listing.broken.push(BrokenModel {
                dir,
                error: err.to_string(),
            }),
        }
    }
    listing
        .installed
        .sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    listing.broken.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(listing)
}

/// Reads an installed model; its manifest id must equal its directory name,
/// so a renamed directory or a case-insensitive lookup never yields another model.
fn load_installed(dir: &Path, name: &str) -> Result<InstalledModel> {
    let manifest = read_manifest(dir)?;
    if manifest.id != name {
        return Err(OragError::InvalidInput(format!(
            "directory `{name}` holds model `{}`; reinstall it with `orag models import`",
            manifest.id
        )));
    }
    Ok(InstalledModel {
        manifest,
        dir: dir.to_path_buf(),
    })
}

/// Looks up an installed model by id, refusing anything that is not a plain id.
fn locate(models_dir: &Path, id: &str) -> Result<InstalledModel> {
    let dir = models_dir.join(id);
    if !is_plain_file_name(id) || !dir.join(MANIFEST_FILE).is_file() {
        return Err(OragError::InvalidInput(format!(
            "model `{id}` is not installed; run `orag models import <pack-dir>`"
        )));
    }
    load_installed(&dir, id)
}

pub fn find_model(models_dir: &Path, id: &str, role: ModelRole) -> Result<InstalledModel> {
    let model = locate(models_dir, id)?;
    if model.manifest.role != role {
        let wanted = match role {
            ModelRole::Embedding => "an embedding",
            ModelRole::Generation => "a generation",
        };
        return Err(OragError::InvalidInput(format!(
            "model `{id}` is not {wanted} model"
        )));
    }
    Ok(model)
}

pub fn verify_model(models_dir: &Path, id: &str) -> Result<()> {
    let model = locate(models_dir, id)?;
    let actual = sha256_file(&model.model_path())?;
    if actual != model.manifest.sha256 {
        return Err(OragError::InvalidInput(format!(
            "model `{id}` is corrupted: expected {}, got {actual}",
            model.manifest.sha256
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn write_pack(dir: &Path, id: &str, role: &str, payload: &[u8]) -> PathBuf {
        let pack = dir.join(format!("pack-{id}"));
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("model.gguf"), payload).unwrap();
        std::fs::write(pack.join("LICENSE"), "Apache-2.0 text").unwrap();
        let sha = hex::encode(Sha256::digest(payload));
        let section = if role == "embedding" {
            "[embedding]\ndimensions = 16\npooling = \"mean\"\nmax_tokens = 128\n"
        } else {
            "[generation]\ncontext_tokens = 2048\nmax_output_tokens = 256\n"
        };
        std::fs::write(
            pack.join(MANIFEST_FILE),
            format!(
                "id = \"{id}\"\nrole = \"{role}\"\nfile = \"model.gguf\"\nsha256 = \"{sha}\"\n\
                 license = \"Apache-2.0\"\nlicense_file = \"LICENSE\"\nsource = \"test\"\nrevision = \"r1\"\n\n{section}"
            ),
        )
        .unwrap();
        pack
    }

    #[test]
    fn import_installs_verified_pack_and_lists_it() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        let pack = write_pack(dir.path(), "emb-a", "embedding", b"weights");
        let manifest = import_pack(&pack, &models).unwrap();
        assert_eq!(manifest.id, "emb-a");
        let installed = list_models(&models).unwrap().installed;
        assert_eq!(installed.len(), 1);
        assert!(installed[0].model_path().exists());
        assert!(installed[0].dir.join("LICENSE").exists());
        verify_model(&models, "emb-a").unwrap();
    }

    #[test]
    fn checksum_mismatch_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        let pack = write_pack(dir.path(), "emb-a", "embedding", b"weights");
        std::fs::write(pack.join("model.gguf"), b"tampered").unwrap();
        let err = import_pack(&pack, &models).unwrap_err().to_string();
        assert!(err.contains("checksum mismatch"), "{err}");
        assert!(list_models(&models).unwrap().installed.is_empty());
        let leftovers: Vec<_> = std::fs::read_dir(&models).unwrap().collect();
        assert!(
            leftovers.is_empty(),
            "staging directory left behind: {leftovers:?}"
        );
    }

    #[test]
    fn importing_twice_is_a_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        let pack = write_pack(dir.path(), "emb-a", "embedding", b"w");
        import_pack(&pack, &models).unwrap();
        assert!(matches!(
            import_pack(&pack, &models),
            Err(OragError::Conflict(_))
        ));
    }

    #[test]
    fn verify_detects_tampering_after_install() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        import_pack(
            &write_pack(dir.path(), "gen-a", "generation", b"w"),
            &models,
        )
        .unwrap();
        std::fs::write(models.join("gen-a").join("model.gguf"), b"changed").unwrap();
        assert!(verify_model(&models, "gen-a").is_err());
    }

    #[test]
    fn find_model_checks_role_and_presence() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        import_pack(
            &write_pack(dir.path(), "gen-a", "generation", b"w"),
            &models,
        )
        .unwrap();
        assert!(find_model(&models, "gen-a", ModelRole::Generation).is_ok());
        let wrong = find_model(&models, "gen-a", ModelRole::Embedding)
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("not an embedding model"), "{wrong}");
        let missing = find_model(&models, "nope", ModelRole::Generation)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("orag models import"), "{missing}");
    }

    #[test]
    fn manifest_validation_rejects_unsafe_or_inconsistent_values() {
        let base = |extra: &str| {
            format!(
                "id = \"x\"\nrole = \"embedding\"\nfile = \"{extra}\"\nsha256 = \"{}\"\nlicense = \"MIT\"\n\
                 license_file = \"LICENSE\"\nsource = \"s\"\nrevision = \"r\"\n\n[embedding]\ndimensions = 4\n\
                 pooling = \"last\"\nmax_tokens = 64\n",
                "a".repeat(64)
            )
        };
        assert!(ModelManifest::parse(&base("model.gguf")).is_ok());
        assert!(ModelManifest::parse(&base("../model.gguf")).is_err());
        assert!(ModelManifest::parse(&base("sub/model.gguf")).is_err());
        assert!(ModelManifest::parse(&base(".hidden")).is_err());
        let wrong_section =
            base("model.gguf").replace("role = \"embedding\"", "role = \"generation\"");
        assert!(ModelManifest::parse(&wrong_section).is_err());
        let bad_id = base("model.gguf").replace("id = \"x\"", "id = \"Bad/Id\"");
        assert!(ModelManifest::parse(&bad_id).is_err());
    }

    #[test]
    fn reserved_or_shared_file_names_are_rejected() {
        let manifest = |file: &str, license: &str| {
            format!(
                "id = \"x\"\nrole = \"embedding\"\nfile = \"{file}\"\nsha256 = \"{}\"\nlicense = \"MIT\"\n\
                 license_file = \"{license}\"\nsource = \"s\"\nrevision = \"r\"\n\n[embedding]\ndimensions = 4\n\
                 pooling = \"last\"\nmax_tokens = 64\n",
                "a".repeat(64)
            )
        };
        assert!(ModelManifest::parse(&manifest("m.gguf", "LICENSE")).is_ok());
        assert!(ModelManifest::parse(&manifest("m.gguf", MANIFEST_FILE)).is_err());
        assert!(ModelManifest::parse(&manifest(MANIFEST_FILE, "LICENSE")).is_err());
        assert!(ModelManifest::parse(&manifest("m.gguf", "m.gguf")).is_err());
    }

    #[test]
    fn verify_only_accepts_installed_model_ids() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        import_pack(&write_pack(dir.path(), "emb-a", "embedding", b"w"), &models).unwrap();
        // A pack outside the models directory must not be reachable by id.
        write_pack(dir.path(), "outside", "embedding", b"w");
        let escape = verify_model(&models, "../pack-outside")
            .unwrap_err()
            .to_string();
        assert!(escape.contains("orag models import"), "{escape}");
        let missing = verify_model(&models, "nope").unwrap_err().to_string();
        assert!(missing.contains("orag models import"), "{missing}");
    }

    #[test]
    fn a_directory_must_hold_the_model_its_name_says() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        import_pack(&write_pack(dir.path(), "emb-a", "embedding", b"w"), &models).unwrap();
        std::fs::rename(models.join("emb-a"), models.join("emb-b")).unwrap();
        assert!(find_model(&models, "emb-b", ModelRole::Embedding).is_err());
        let listing = list_models(&models).unwrap();
        assert!(listing.installed.is_empty());
        assert_eq!(listing.broken.len(), 1);
    }

    #[test]
    fn one_broken_manifest_does_not_hide_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        import_pack(&write_pack(dir.path(), "emb-a", "embedding", b"w"), &models).unwrap();
        std::fs::create_dir_all(models.join("junk")).unwrap();
        std::fs::write(models.join("junk").join(MANIFEST_FILE), "not toml [").unwrap();
        let listing = list_models(&models).unwrap();
        assert_eq!(listing.installed.len(), 1);
        assert_eq!(listing.broken.len(), 1);
        assert!(listing.broken[0].dir.ends_with("junk"));
    }

    #[test]
    fn a_missing_weights_file_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let pack = write_pack(dir.path(), "emb-a", "embedding", b"w");
        std::fs::remove_file(pack.join("model.gguf")).unwrap();
        let err = import_pack(&pack, &dir.path().join("models"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("model.gguf"), "{err}");
    }

    /// A directory handle that can change its times: Windows opens a
    /// directory only with FILE_FLAG_BACKUP_SEMANTICS and needs write access.
    fn open_dir_for_times(dir: &Path) -> std::fs::File {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(dir)
                .unwrap()
        }
        #[cfg(not(windows))]
        std::fs::File::open(dir).unwrap()
    }

    #[test]
    fn stale_staging_directories_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("models");
        let stale = models.join(".emb-a.importing-1-2-0");
        std::fs::create_dir_all(&stale).unwrap();
        let old =
            std::time::SystemTime::now() - STALE_STAGING_AGE - std::time::Duration::from_secs(60);
        open_dir_for_times(&stale).set_modified(old).unwrap();
        let fresh = models.join(".emb-b.importing-1-2-0");
        std::fs::create_dir_all(&fresh).unwrap();
        import_pack(&write_pack(dir.path(), "emb-c", "embedding", b"w"), &models).unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists(), "an import that may still run is left alone");
    }

    #[test]
    fn roles_print_in_lowercase() {
        assert_eq!(ModelRole::Embedding.as_str(), "embedding");
        assert_eq!(ModelRole::Generation.as_str(), "generation");
    }
}
