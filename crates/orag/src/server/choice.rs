//! The setup page's generator choice (step 82): the machine probe and the
//! tier options for `GET /v1/models`, and the check of the
//! `{"generation_model": "<id>"}` body of `POST /v1/models/download`. Both
//! read the same `options::evaluate`, so the page can never offer what the
//! server refuses.

use axum::http::StatusCode;
use serde::Deserialize;

use crate::infer::download::Catalog;
use crate::infer::download::options::{Candidate, Ineligible, Options, evaluate};
use crate::infer::models::{ModelRole, find_model};
use crate::probe::Machine;
use crate::server::errors::{ApiError, ApiResult};
use crate::server::{AppState, ModelsInfo};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StartBody {
    generation_model: Option<String>,
}

/// The generator a start names: no body, `{}` or `{"generation_model": null}`
/// leave the configured models; anything else is `400 invalid_input`.
pub fn requested_generator(body: &[u8]) -> ApiResult<Option<String>> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    serde_json::from_slice::<StartBody>(body)
        .map(|start| start.generation_model)
        .map_err(|_| {
            ApiError::invalid(
                "send no body, {} or {\"generation_model\": \"<id>\"} with one of the offered generators",
            )
        })
}

/// The machine now and what it is offered. Blocking: it reads the models
/// folder (installed packs, free disk).
pub fn machine_options(state: &AppState) -> (Machine, Options) {
    let models = &state.models;
    let downloads = &state.downloads;
    let machine = downloads.machine(&models.models_dir);
    let catalog = &downloads.catalog;
    let installed = |id: &str, role| find_model(&models.models_dir, id, role).is_ok();
    let candidates: Vec<Candidate> = downloads
        .tiers
        .iter()
        .filter_map(|tier| {
            let entry = generator(catalog, &tier.id)?;
            Some(Candidate {
                tier,
                bytes: entry.total_size(),
                installed: installed(&tier.id, ModelRole::Generation),
            })
        })
        .collect();
    let embedding_bytes = match catalog.find(&models.embedding) {
        Some(entry) if !installed(&models.embedding, ModelRole::Embedding) => entry.total_size(),
        _ => 0,
    };
    let options = evaluate(&machine, &candidates, embedding_bytes, &models.generation);
    (machine, options)
}

fn generator<'a>(
    catalog: &'a Catalog,
    id: &str,
) -> Option<&'a crate::infer::download::CatalogEntry> {
    catalog
        .find(id)
        .filter(|entry| entry.role == ModelRole::Generation)
}

/// Checks `id` against the options: `400 unknown_model` when it is not an
/// offered tier, `409 model_not_eligible` when this machine cannot take it.
pub fn check_choice(options: &Options, id: &str) -> ApiResult<()> {
    match options.eligible(id) {
        Ok(_) => Ok(()),
        Err(Ineligible::Unknown) => {
            let offered: Vec<&str> = options.options.iter().map(|o| o.id.as_str()).collect();
            Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unknown_model",
                format!(
                    "generation_model must be one of the offered generators: {}",
                    offered.join(", ")
                ),
            ))
        }
        Err(Ineligible::NotEligible(reason)) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "model_not_eligible",
            format!(
                "this computer cannot take that generator ({reason}); choose one the page marks as fitting"
            ),
        )),
    }
}

/// The configured models with `generation` as the generator.
pub fn with_generator(models: &ModelsInfo, generation: &str) -> ModelsInfo {
    ModelsInfo {
        generation: generation.to_string(),
        ..models.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_names_a_generator_or_nothing() {
        assert_eq!(requested_generator(b"").unwrap(), None);
        assert_eq!(requested_generator(b" {} ").unwrap(), None);
        assert_eq!(
            requested_generator(b"{\"generation_model\": null}").unwrap(),
            None
        );
        assert_eq!(
            requested_generator(b"{\"generation_model\": \"x\"}").unwrap(),
            Some("x".into())
        );
        for bad in [
            &b"{\"models\": [\"x\"]}"[..],
            b"[]",
            b"{\"generation_model\": 4}",
            b"nope",
        ] {
            let err = requested_generator(bad).unwrap_err();
            assert_eq!(err.code, "invalid_input");
        }
    }
}
