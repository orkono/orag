# Changelog

All notable changes to ORAG are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Every change merged to `main`
bumps the version (see `.docs/orag-decisions.md`, D-017).

## [0.1.0-alpha.1] - 2026-10-02

- Owner decisions recorded: no API authentication (loopback only, Host/Origin checks), formats TXT/MD/DOCX/PDF, 5 MB document limit (configurable 1-10 MB), `config.toml` read once at startup (D-010, D-011, D-013, D-019).
- Development workflow rules (D-020): step branches merged into `main` without PRs, orchestrator-only git, `/code-review` before every push, mandatory tests and changelog per step.
- Plan: verified DOCX (zip + roxmltree) and PDF (pdf_oxide) ingestion task replaces the PDF spike; versions renumbered.

## [0.1.0-alpha.0] - 2026-10-01

- Architecture decision record (`.docs/orag-decisions.md`) reached by Claude/Codex review.
- v0.1 implementation plan (`.docs/plans/2026-10-01-orag-v0.1-implementation-plan.md`).
