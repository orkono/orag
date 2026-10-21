//! The bench's questions and documents, built into the desktop app only
//! (`orag` stays unchanged) and written out before a bench runs: the 10
//! pinned v0.2 dev questions, the v0.2 retrieval labels, and the v0.2
//! corpus (12 files).

use std::path::{Path, PathBuf};

macro_rules! file {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../eval/corpus/v02/",
                $name
            )) as &[u8],
        )
    };
}

pub const CORPUS: &[(&str, &[u8])] = &[
    file!("en-barista-manual.md"),
    file!("en-pump-manual.docx"),
    file!("en-security-policy.md"),
    file!("en-warranty-terms.md"),
    file!("tr-anayasa.txt"),
    file!("tr-fiyat-listesi.txt"),
    file!("tr-insan-kaynaklari.md"),
    file!("tr-izin-yonetmeligi.md"),
    file!("tr-kargo-politikasi.md"),
    file!("tr-orx7-kurulum.md"),
    file!("tr-sulama-katalogu.pdf"),
    file!("tr-yargitay-1cd-2016-347.pdf"),
];

pub const QUESTIONS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../eval/datasets/v06-bench.jsonl"
));
pub const RETRIEVAL: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../eval/datasets/v02-retrieval.jsonl"
));

/// Where [`extract`] put the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub corpus: PathBuf,
    pub questions: PathBuf,
    pub retrieval: PathBuf,
}

/// Writes the files under `dir` (which must not hold them yet).
pub fn extract(dir: &Path) -> std::io::Result<Extracted> {
    let corpus = dir.join("corpus");
    std::fs::create_dir_all(&corpus)?;
    for (name, bytes) in CORPUS {
        write_new(&corpus.join(name), bytes)?;
    }
    let questions = dir.join("questions.jsonl");
    write_new(&questions, QUESTIONS)?;
    let retrieval = dir.join("retrieval.jsonl");
    write_new(&retrieval, RETRIEVAL)?;
    Ok(Extracted {
        corpus,
        questions,
        retrieval,
    })
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)
}

/// Lowercase hex SHA-256 of the questions, recorded in the results file.
pub fn questions_sha256() -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(QUESTIONS)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }

    #[test]
    fn the_embedded_corpus_is_the_v02_corpus() {
        let mut names: Vec<String> = std::fs::read_dir(repo("eval/corpus/v02"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let embedded: Vec<&str> = CORPUS.iter().map(|(n, _)| *n).collect();
        assert_eq!(embedded, names);
    }

    #[test]
    fn extraction_writes_identical_bytes_once() {
        let dir = tempfile::tempdir().unwrap();
        let out = extract(dir.path()).unwrap();
        for (name, _) in CORPUS {
            assert_eq!(
                std::fs::read(out.corpus.join(name)).unwrap(),
                std::fs::read(repo("eval/corpus/v02").join(name)).unwrap(),
                "{name}"
            );
        }
        assert_eq!(
            std::fs::read(&out.questions).unwrap(),
            std::fs::read(repo("eval/datasets/v06-bench.jsonl")).unwrap()
        );
        assert_eq!(
            std::fs::read(&out.retrieval).unwrap(),
            std::fs::read(repo("eval/datasets/v02-retrieval.jsonl")).unwrap()
        );
        assert!(extract(dir.path()).is_err(), "never over existing files");
        assert_eq!(questions_sha256().len(), 64);
    }
}
