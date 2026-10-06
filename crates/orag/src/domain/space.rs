//! Embedding-space identity (D-009). Vectors from different spaces never mix.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::chunker::CHUNKER_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceDescriptor {
    pub model_id: String,
    pub model_sha256: String,
    pub pooling: String,
    pub query_prefix: String,
    pub document_prefix: String,
    pub dimensions: usize,
    pub normalized: bool,
    /// Maximum input tokens per embedded text; changes chunk boundaries.
    pub max_tokens: usize,
    /// Whether one trailing EOS is enforced; changes the encoded input.
    pub require_trailing_eos: bool,
}

/// Version of the encoding below; change it only together with a reindex path.
/// Encoding 2 (0.2.0-alpha.3) dropped the normalizer version: lexical text never
/// reaches the embedder, so it has its own version and a startup FTS rebuild
/// (`store::lexical`) instead of a reindex.
const FINGERPRINT_ENCODING: u32 = 2;
/// Normalizer versions that encoding 1 hashed; spaces written then still hold
/// valid vectors.
const LEGACY_NORMALIZER_VERSIONS: [u32; 2] = [1, 2];

impl SpaceDescriptor {
    /// SHA-256 over a hand-written, fixed-order encoding of the descriptor
    /// plus the chunker version. Each field is
    /// `name=<byte length>:<value>\n`, so no value can imitate another field,
    /// and no dependency feature (e.g. serde_json `preserve_order`) can change it.
    /// `model_sha256` comes from a validated pack manifest (64 lowercase hex).
    pub fn fingerprint(&self) -> String {
        self.encode(FINGERPRINT_ENCODING, None)
    }

    /// Whether a stored fingerprint names this space: the current one, or an
    /// encoding-1 fingerprint of the same descriptor and chunker (D-009).
    pub fn accepts(&self, stored: &str) -> bool {
        stored == self.fingerprint()
            || LEGACY_NORMALIZER_VERSIONS
                .iter()
                .any(|&normalizer| stored == self.encode(1, Some(normalizer)))
    }

    /// An encoding-1 fingerprint, as releases before 0.2.0-alpha.3 stored it.
    #[cfg(test)]
    pub(crate) fn legacy_fingerprint(&self, normalizer_version: u32) -> String {
        self.encode(1, Some(normalizer_version))
    }

    fn encode(&self, encoding: u32, normalizer_version: Option<u32>) -> String {
        // Exhaustive destructuring: a new field does not compile until it is encoded here.
        let SpaceDescriptor {
            model_id,
            model_sha256,
            pooling,
            query_prefix,
            document_prefix,
            dimensions,
            normalized,
            max_tokens,
            require_trailing_eos,
        } = self;
        let mut hasher = Sha256::new();
        let mut field = |name: &str, value: &str| {
            hasher.update(format!("{name}={}:", value.len()).as_bytes());
            hasher.update(value.as_bytes());
            hasher.update(b"\n");
        };
        field("encoding", &encoding.to_string());
        field("model_id", model_id);
        field("model_sha256", model_sha256);
        field("pooling", pooling);
        field("query_prefix", query_prefix);
        field("document_prefix", document_prefix);
        field("dimensions", &dimensions.to_string());
        field("normalized", &normalized.to_string());
        field("max_tokens", &max_tokens.to_string());
        field("require_trailing_eos", &require_trailing_eos.to_string());
        field("chunker_version", &CHUNKER_VERSION.to_string());
        if let Some(version) = normalizer_version {
            field("normalizer_version", &version.to_string());
        }
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddingSpace {
    pub id: i64,
    pub fingerprint: String,
    pub dimensions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn descriptor() -> SpaceDescriptor {
        SpaceDescriptor {
            model_id: "m".into(),
            model_sha256: "a".repeat(64),
            pooling: "last".into(),
            query_prefix: "Q: ".into(),
            document_prefix: String::new(),
            dimensions: 4,
            normalized: true,
            max_tokens: 512,
            require_trailing_eos: true,
        }
    }

    #[test]
    fn fingerprint_matches_the_golden_value() {
        // Golden vector: persisted fingerprints must never change silently.
        // Reproducible without Rust:
        // printf 'encoding=1:2\nmodel_id=1:m\nmodel_sha256=64:%s\npooling=4:last\nquery_prefix=3:Q: \ndocument_prefix=0:\ndimensions=1:4\nnormalized=4:true\nmax_tokens=3:512\nrequire_trailing_eos=4:true\nchunker_version=1:1\n' "$(printf 'a%.0s' $(seq 64))" | shasum -a 256
        assert_eq!(
            descriptor().fingerprint(),
            "37c52ca70f826e186f68a9c8f58054038b0ebe5baad3b058e22944d743f7c545"
        );
    }

    #[test]
    fn fingerprints_written_before_the_lexical_split_are_accepted() {
        // Encoding 1 also hashed the normalizer version, which never changed
        // the vectors: 0.1.x wrote normalizer 1, 0.2.0-alpha.1 normalizer 2.
        // Same printf as above with `encoding=1:1` and a trailing
        // `normalizer_version=1:1\n` or `normalizer_version=1:2\n`.
        let desc = descriptor();
        for stored in [
            "1126366fade8f536ef5ab0f8fdb8a758a4fe6fb601831050f20e4f05ad18b650",
            "d00dab2a989b011e42ab960875404bdf321c19d696cb5ba48bbc1fd63ea7c379",
            desc.fingerprint().as_str(),
        ] {
            assert!(desc.accepts(stored), "{stored}");
        }
        // Another model's legacy fingerprint is still a different space.
        let other = SpaceDescriptor {
            model_id: "x".into(),
            ..descriptor()
        };
        assert!(!other.accepts("1126366fade8f536ef5ab0f8fdb8a758a4fe6fb601831050f20e4f05ad18b650"));
        assert!(!desc.accepts(&other.fingerprint()));
    }

    #[test]
    fn a_value_cannot_forge_a_field_boundary() {
        // Without the length prefix both encode to
        // "query_prefix=a\ndocument_prefix=b\ndocument_prefix=c\n".
        let a = SpaceDescriptor {
            query_prefix: "a\ndocument_prefix=b".into(),
            document_prefix: "c".into(),
            ..descriptor()
        };
        let b = SpaceDescriptor {
            query_prefix: "a".into(),
            document_prefix: "b\ndocument_prefix=c".into(),
            ..descriptor()
        };
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn every_field_changes_the_fingerprint() {
        let base = descriptor().fingerprint();
        let variants = [
            SpaceDescriptor {
                model_id: "x".into(),
                ..descriptor()
            },
            SpaceDescriptor {
                model_sha256: "b".repeat(64),
                ..descriptor()
            },
            SpaceDescriptor {
                pooling: "mean".into(),
                ..descriptor()
            },
            SpaceDescriptor {
                query_prefix: "".into(),
                ..descriptor()
            },
            SpaceDescriptor {
                document_prefix: "D: ".into(),
                ..descriptor()
            },
            SpaceDescriptor {
                dimensions: 8,
                ..descriptor()
            },
            SpaceDescriptor {
                normalized: false,
                ..descriptor()
            },
            SpaceDescriptor {
                max_tokens: 1024,
                ..descriptor()
            },
            SpaceDescriptor {
                require_trailing_eos: false,
                ..descriptor()
            },
        ];
        for variant in variants {
            assert_ne!(variant.fingerprint(), base, "{variant:?}");
        }
    }
}
