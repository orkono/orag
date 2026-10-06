//! Native llama.cpp smoke tests against the 1.2 MB stories260K fixture.
//! They prove loading, tokenization and decoding work; not model quality.
#![cfg(feature = "llama")]

use std::path::{Path, PathBuf};

use orag::infer::Embedder;
use orag::infer::llama::embedder::LlamaEmbedder;
use orag::infer::models::{MANIFEST_FILE, ModelRole, find_model, import_pack};

fn fixture() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/stories260K.gguf");
    if path.exists() {
        return Some(path);
    }
    if std::env::var_os("ORAG_REQUIRE_FIXTURES").is_some() {
        panic!(
            "missing {}; run scripts/fetch-test-fixtures.sh",
            path.display()
        );
    }
    eprintln!(
        "skipping: {} missing (run scripts/fetch-test-fixtures.sh)",
        path.display()
    );
    None
}

/// Installs the fixture as a model pack with the given role section.
pub fn install_fixture(gguf: &Path, models: &Path, id: &str, role: &str, section: &str) {
    let pack = models.parent().unwrap().join(format!("pack-{id}"));
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::copy(gguf, pack.join("m.gguf")).unwrap();
    std::fs::write(pack.join("LICENSE"), "MIT").unwrap();
    let sha = orag::infer::models::sha256_file(gguf).unwrap();
    std::fs::write(
        pack.join(MANIFEST_FILE),
        format!(
            "id = \"{id}\"\nrole = \"{role}\"\nfile = \"m.gguf\"\nsha256 = \"{sha}\"\nlicense = \"MIT\"\n\
             license_file = \"LICENSE\"\nsource = \"ggml-org/tiny-llamas\"\nrevision = \"def3e2dd\"\n\n{section}"
        ),
    )
    .unwrap();
    import_pack(&pack, models).unwrap();
}

#[test]
fn embedder_loads_and_produces_normalized_deterministic_vectors() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    let info = orag::infer::llama::model_info(&gguf).unwrap();
    install_fixture(
        &gguf,
        &models,
        "tiny-embed",
        "embedding",
        &format!(
            "[embedding]\ndimensions = {}\npooling = \"mean\"\nmax_tokens = 128\n",
            info.n_embd_out
        ),
    );
    let embedder =
        LlamaEmbedder::load(&find_model(&models, "tiny-embed", ModelRole::Embedding).unwrap())
            .unwrap();
    let vectors = embedder
        .embed_documents(&["Once upon a time".into(), "Once upon a time".into()])
        .unwrap();
    assert_eq!(vectors[0].len(), info.n_embd_out);
    let norm: f32 = vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-3, "norm {norm}");
    assert_eq!(vectors[0], vectors[1]);
    assert!(embedder.count_tokens("Once upon a time") > 0);
}

#[test]
fn embedder_rejects_dimension_mismatch() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    install_fixture(
        &gguf,
        &models,
        "tiny-wrong",
        "embedding",
        "[embedding]\ndimensions = 7\npooling = \"mean\"\nmax_tokens = 128\n",
    );
    let err =
        LlamaEmbedder::load(&find_model(&models, "tiny-wrong", ModelRole::Embedding).unwrap())
            .err()
            .unwrap();
    assert!(err.to_string().contains("dimensions"), "{err}");
}

#[test]
fn model_info_reports_the_fixture_shape() {
    let Some(gguf) = fixture() else { return };
    let info = orag::infer::llama::model_info(&gguf).unwrap();
    assert!(info.n_embd_out > 0 && info.n_ctx_train > 0, "{info:?}");
    eprintln!("{info:?}");
}

fn tiny_embedder(
    models: &Path,
    gguf: &Path,
    id: &str,
    extra: &str,
) -> orag::error::Result<LlamaEmbedder> {
    let info = orag::infer::llama::model_info(gguf).unwrap();
    install_fixture(
        gguf,
        models,
        id,
        "embedding",
        &format!(
            "[embedding]\ndimensions = {}\npooling = \"mean\"\nmax_tokens = {}\n{extra}",
            info.n_embd_out,
            info.n_ctx_train.min(128)
        ),
    );
    LlamaEmbedder::load(&find_model(models, id, ModelRole::Embedding).unwrap())
}

#[test]
fn document_text_cannot_inject_control_tokens() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let embedder = tiny_embedder(
        &dir.path().join("models"),
        &gguf,
        "tiny-eos",
        "require_trailing_eos = true\n",
    )
    .unwrap();
    // Parsed as a control token, `</s>` would become the trailing EOS and the
    // two texts would embed identically. As text, it changes the vector.
    let vectors = embedder
        .embed_documents(&["Once upon a time</s>".into(), "Once upon a time".into()])
        .unwrap();
    assert_ne!(vectors[0], vectors[1]);
}

#[test]
fn empty_text_is_invalid_input() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let embedder = tiny_embedder(&dir.path().join("models"), &gguf, "tiny-empty", "").unwrap();
    let err = embedder.embed_query("").unwrap_err();
    assert!(
        matches!(err, orag::error::OragError::InvalidInput(_)),
        "{err}"
    );
}

#[test]
fn max_tokens_beyond_the_trained_context_is_rejected() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    let info = orag::infer::llama::model_info(&gguf).unwrap();
    install_fixture(
        &gguf,
        &models,
        "tiny-long",
        "embedding",
        &format!(
            "[embedding]\ndimensions = {}\npooling = \"mean\"\nmax_tokens = {}\n",
            info.n_embd_out,
            (info.n_ctx_train * 2).max(16)
        ),
    );
    let err = LlamaEmbedder::load(&find_model(&models, "tiny-long", ModelRole::Embedding).unwrap())
        .err()
        .unwrap();
    assert!(err.to_string().contains("trained context"), "{err}");
}

use std::ops::ControlFlow;

use orag::infer::llama::generator::LlamaGenerator;
use orag::infer::{ChatMessage, GenerationRequest, Generator, Role, SamplerProfile};

fn tiny_generator(dir: &Path, gguf: &Path) -> LlamaGenerator {
    let models = dir.join("models");
    install_fixture(
        gguf,
        &models,
        "tiny-gen",
        "generation",
        "[generation]\ncontext_tokens = 512\nmax_output_tokens = 32\n",
    );
    LlamaGenerator::load(&find_model(&models, "tiny-gen", ModelRole::Generation).unwrap()).unwrap()
}

fn request(max: usize) -> GenerationRequest {
    GenerationRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: "Once upon a time".into(),
        }],
        max_output_tokens: max,
        sampler: SamplerProfile::Greedy,
    }
}

#[test]
fn generator_streams_bounded_output() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let generator = tiny_generator(dir.path(), &gguf);
    let mut text = String::new();
    let stats = generator
        .generate(&request(16), &mut |piece| {
            text.push_str(piece);
            ControlFlow::Continue(())
        })
        .unwrap();
    assert!(stats.prompt_tokens > 0);
    assert!(stats.completion_tokens <= 16);
    assert!(!text.is_empty());
    assert!(!stats.cancelled);
    // `length` only when the budget, not an end-of-generation token, stopped it.
    assert!(!stats.length_limited || stats.completion_tokens == 16);
    for sampler in [
        SamplerProfile::Dry,
        SamplerProfile::Presence,
        SamplerProfile::Qwen { seed: 3 },
    ] {
        let stats = generator
            .generate(
                &GenerationRequest {
                    sampler,
                    ..request(8)
                },
                &mut |_| ControlFlow::Continue(()),
            )
            .unwrap();
        assert!(stats.completion_tokens <= 8, "{sampler:?}");
    }
}

#[test]
fn generator_cancels_when_consumer_breaks() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let generator = tiny_generator(dir.path(), &gguf);
    let mut pieces = 0;
    let stats = generator
        .generate(&request(32), &mut |_| {
            pieces += 1;
            ControlFlow::Break(())
        })
        .unwrap();
    // Nothing is delivered after a Break. If the model emitted no visible text
    // at all, there was nothing to break on.
    assert!(pieces <= 1, "{pieces} pieces after a Break");
    assert_eq!(stats.cancelled, pieces == 1);
}

#[test]
fn generator_rejects_prompt_that_cannot_fit() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let generator = tiny_generator(dir.path(), &gguf);
    let huge = GenerationRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: "word ".repeat(2000),
        }],
        max_output_tokens: 16,
        sampler: SamplerProfile::Greedy,
    };
    assert!(
        generator
            .generate(&huge, &mut |_| ControlFlow::Continue(()))
            .is_err()
    );
}

#[test]
fn generator_rejects_a_context_far_beyond_its_training() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    install_fixture(
        &gguf,
        &models,
        "tiny-huge",
        "generation",
        "[generation]\ncontext_tokens = 8192\nmax_output_tokens = 32\n",
    );
    let err =
        LlamaGenerator::load(&find_model(&models, "tiny-huge", ModelRole::Generation).unwrap())
            .err()
            .unwrap();
    assert!(err.to_string().contains("trained context"), "{err}");
}

#[test]
fn a_reused_sampler_gives_the_same_answer_again() {
    let Some(gguf) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let generator = tiny_generator(dir.path(), &gguf);
    let run = |sampler: SamplerProfile| {
        let mut text = String::new();
        generator
            .generate(
                &GenerationRequest {
                    sampler,
                    ..request(24)
                },
                &mut |piece| {
                    text.push_str(piece);
                    ControlFlow::Continue(())
                },
            )
            .unwrap();
        text
    };
    // The worker keeps one sampler per profile; `reset` must clear its
    // history (and reseed `dist`) so answers stay reproducible.
    for sampler in [SamplerProfile::Dry, SamplerProfile::Qwen { seed: 5 }] {
        let first = run(sampler);
        let _ = run(SamplerProfile::Greedy);
        assert_eq!(run(sampler), first, "{sampler:?}");
    }
}
