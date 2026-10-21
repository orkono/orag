//! `orag-vulkan` placed loads through the real role loaders (v0.6 plan,
//! Task 3), on whatever Vulkan device the environment exposes: CI runs them
//! with Mesa's lavapipe as the only device (`ORAG_VULKAN_ALLOW_SOFTWARE=1`,
//! `ORAG_REQUIRE_GPU=1`) and `--test-threads=1`, because llama.cpp's fit
//! and the test hooks are process-wide.
#![cfg(feature = "vulkan")]

use std::path::{Path, PathBuf};

use orag::gpu::{Placement, choose_device};
use orag::infer::llama::embedder::LlamaEmbedder;
use orag::infer::llama::generator::LlamaGenerator;
use orag::infer::models::{InstalledModel, MANIFEST_FILE, ModelRole, find_model, import_pack};
use orag::infer::{ChatMessage, Embedder, GenerationRequest, Generator, Role, SamplerProfile};

fn fixture() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-fixtures/stories260K.gguf");
    if path.exists() {
        return Some(path);
    }
    assert!(
        std::env::var_os("ORAG_REQUIRE_FIXTURES").is_none(),
        "missing {}",
        path.display()
    );
    None
}

/// The device the service would choose here, or `None` (then the test
/// skips, unless CI requires a device).
fn gpu() -> Option<Placement> {
    let devices = orag::infer::llama::devices::gpu_devices().unwrap();
    let allow = std::env::var_os("ORAG_VULKAN_ALLOW_SOFTWARE").is_some_and(|v| v == "1");
    let chosen = choose_device(&devices, allow).map(|device| Placement::Gpu {
        index: device.index,
        name: device.name.clone(),
    });
    assert!(
        chosen.is_some() || std::env::var_os("ORAG_REQUIRE_GPU").is_none(),
        "no GPU device: {devices:?}"
    );
    chosen
}

fn install(gguf: &Path, dir: &Path, id: &str, role: ModelRole, section: &str) -> InstalledModel {
    let (models, pack) = (dir.join("models"), dir.join(format!("pack-{id}")));
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::copy(gguf, pack.join("m.gguf")).unwrap();
    std::fs::write(pack.join("LICENSE"), "MIT").unwrap();
    let role_name = match role {
        ModelRole::Embedding => "embedding",
        ModelRole::Generation => "generation",
        ModelRole::Reranking => "reranking",
    };
    std::fs::write(
        pack.join(MANIFEST_FILE),
        format!(
            "id = \"{id}\"\nrole = \"{role_name}\"\nfile = \"m.gguf\"\nsha256 = \"{}\"\n\
             license = \"MIT\"\nlicense_file = \"LICENSE\"\nsource = \"s\"\nrevision = \"r\"\n\n{section}",
            orag::infer::models::sha256_file(gguf).unwrap()
        ),
    )
    .unwrap();
    import_pack(&pack, &models).unwrap();
    find_model(&models, id, role).unwrap()
}

fn embedder_model(gguf: &Path, dir: &Path) -> InstalledModel {
    let dims = orag::infer::llama::model_info(gguf).unwrap().n_embd_out;
    install(
        gguf,
        dir,
        "tiny-embed",
        ModelRole::Embedding,
        &format!("[embedding]\ndimensions = {dims}\npooling = \"mean\"\nmax_tokens = 128\n"),
    )
}

fn generator_model(gguf: &Path, dir: &Path) -> InstalledModel {
    install(
        gguf,
        dir,
        "tiny-gen",
        ModelRole::Generation,
        "[generation]\ncontext_tokens = 512\nmax_output_tokens = 16\n",
    )
}

/// Sets the test hook for one test; the tests run on one thread.
fn with_hook<T>(hook: Option<&str>, run: impl FnOnce() -> T) -> T {
    // SAFETY: this test binary runs with --test-threads=1, and nothing else
    // reads the environment while the hook is set.
    unsafe {
        match hook {
            Some(value) => std::env::set_var("ORAG_VULKAN_TEST_FIT", value),
            None => std::env::remove_var("ORAG_VULKAN_TEST_FIT"),
        }
    }
    let result = run();
    unsafe { std::env::remove_var("ORAG_VULKAN_TEST_FIT") };
    result
}

fn embed(embedder: &LlamaEmbedder) -> Vec<f32> {
    embedder
        .embed_documents(&["Once upon a time".into()])
        .unwrap()
        .remove(0)
}

fn generate(generator: &LlamaGenerator) -> String {
    let request = GenerationRequest {
        messages: vec![ChatMessage {
            role: Role::User,
            content: "Once upon a time".into(),
        }],
        max_output_tokens: 8,
        sampler: SamplerProfile::Greedy,
    };
    let mut text = String::new();
    generator
        .generate(&request, &mut |piece| {
            text.push_str(piece);
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
    text
}

#[test]
fn the_embedder_runs_on_the_gpu_and_agrees_with_the_cpu() {
    let (Some(gguf), Some(gpu)) = (fixture(), gpu()) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let model = embedder_model(&gguf, dir.path());
    let on_gpu = with_hook(None, || LlamaEmbedder::load_placed(&model, &gpu).unwrap());
    let offload = on_gpu.offload().unwrap().clone();
    assert!(
        offload.layers_on_gpu > 0 && offload.reason.is_none(),
        "{offload:?}"
    );
    let cpu = LlamaEmbedder::load_placed(&model, &Placement::CpuOnly { reason: "test" }).unwrap();
    let (a, b) = (embed(&on_gpu), embed(&cpu));
    let cosine: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    assert!(cosine > 0.99, "GPU and CPU vectors differ: cosine {cosine}");
}

#[test]
fn an_embedder_whose_gpu_context_fails_runs_cpu_only() {
    let (Some(gguf), Some(gpu)) = (fixture(), gpu()) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let model = embedder_model(&gguf, dir.path());
    let embedder = with_hook(Some("context_failure"), || {
        LlamaEmbedder::load_placed(&model, &gpu).unwrap()
    });
    let offload = embedder.offload().unwrap();
    assert_eq!(
        (offload.layers_on_gpu, offload.reason),
        (0, Some("alloc_failed"))
    );
    assert_eq!(embed(&embedder).len(), embedder.descriptor().dimensions);
}

#[test]
fn a_generator_whose_gpu_context_fails_runs_cpu_only() {
    let (Some(gguf), Some(gpu)) = (fixture(), gpu()) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let model = generator_model(&gguf, dir.path());
    let generator = with_hook(Some("context_failure"), || {
        LlamaGenerator::load_placed(&model, &gpu).unwrap()
    });
    let offload = generator.offload().unwrap();
    assert_eq!(
        (offload.layers_on_gpu, offload.reason),
        (0, Some("alloc_failed"))
    );
    assert!(!generate(&generator).is_empty());
}

#[test]
fn a_generator_on_the_gpu_generates() {
    let (Some(gguf), Some(gpu)) = (fixture(), gpu()) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let model = generator_model(&gguf, dir.path());
    let generator = with_hook(None, || LlamaGenerator::load_placed(&model, &gpu).unwrap());
    assert!(generator.offload().unwrap().layers_on_gpu > 0);
    assert!(!generate(&generator).is_empty());
}
