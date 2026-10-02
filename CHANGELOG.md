# Changelog

All notable changes to ORAG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Every change merged to `main`
bumps the version (see `.docs/orag-decisions.md`, D-017).

## [0.1.0-alpha.2] - 2026-10-02

- Plan fixes from the Codex review: parser output is capped in the parent; the Linux memory cap is checked and verified by a Linux test; an explicit JSON `format` can no longer bypass filename rules; an end-to-end test covers interrupted isolated ingestion; at most 4 uploads are accepted at once (`429 busy`).
- Resume procedure inspects the current branch and uncommitted work before switching.
- Uploads whose whole body is not received within 60 s get `408 upload_timeout`; the JSON filename-first rule lands in Task 17; parser stderr is drained instead of cut off; the parent's output buffer never grows past its cap; a refused memory limit is an internal error; the test-only parse delay works in debug builds only; Task 16's dev-dependency commands are fixed; only the upload route accepts large bodies (64 KiB elsewhere); D-010 documents the JSON format rules.
- Documented the per-service resource budget, upgrade notes for default-model changes, and partial indexing; the Task 22 fallback now updates both model defaults.
- Version numbers in the plan shifted by one; Task 1 now produces `0.1.0-alpha.3`.

## [0.1.0-alpha.1] - 2026-10-02

- Owner decisions recorded: no API authentication (loopback only, Host/Origin checks), formats TXT/MD/DOCX/PDF, 5 MB document limit (configurable 1-10 MB), `config.toml` read once at startup (D-010, D-011, D-013, D-019).
- Development workflow rules (D-020): step branches merged into `main` without PRs, orchestrator-only git, `/code-review` before every push, mandatory tests and changelog per step.
- Plan: verified DOCX (zip + roxmltree) and PDF (pdf_oxide) ingestion task replaces the PDF spike; versions renumbered.

## [0.1.0-alpha.0] - 2026-10-01

- Architecture decision record (`.docs/orag-decisions.md`) reached by Claude/Codex review.
- v0.1 implementation plan (`.docs/plans/2026-10-01-orag-v0.1-implementation-plan.md`).
