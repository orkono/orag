# ORAG --- Architecture & Design Notes

> **Status:** Long-term vision. Binding decisions for the current release
> live in `.docs/orag-decisions.md`; where the two disagree, the decision
> record wins. Implementation plan:
> `.docs/plans/2026-10-01-orag-v0.1-implementation-plan.md`.\
> **Project:** ORAG\
> **Repository:** `orkono/orag`\
> **Primary goal:** A fast, accurate, local-first RAG engine distributed
> as a single executable, with no mandatory Python runtime, Docker,
> Ollama, PostgreSQL, Qdrant, Elasticsearch, or other external service.

## 1. Product Direction

ORAG is intended to be a **zero-infrastructure, local-first RAG
engine**.

The desired user experience is:

``` bash
./orag
```

followed by a small HTTP API for document ingestion and querying.

Initial API scope:

``` text
POST /v1/documents
POST /v1/query
```

The first version should deliberately avoid becoming a large application
platform. The focus is:

1.  ingest documents,
2.  retrieve the correct evidence quickly,
3.  generate grounded answers,
4.  expose useful citations/trace information,
5.  run locally with minimal operational dependencies.

A useful positioning statement is:

> **Download one binary, add documents, and query them locally.**

Model weights are not expected to be compiled into the executable. The
**runtime is part of the binary**, while model files can be
downloaded/cached separately on first use and then used fully offline.

Example layout:

``` text
orag                         # single executable

~/.orag/
├── orag.db
└── models/
    ├── embedding/
    ├── reranker/
    ├── decision/
    ├── document-ai/
    └── generation/
```

------------------------------------------------------------------------

## 2. Language Choice

### Current direction: Rust

Python was initially considered because of its mature RAG/document
ecosystem. However, the single-binary requirement substantially changes
the trade-off.

Rust is currently preferred because it provides:

-   excellent single-binary distribution,
-   low runtime overhead,
-   predictable memory usage,
-   strong concurrency,
-   native access to SQLite,
-   increasingly capable local ML inference libraries,
-   Hugging Face `tokenizers` / `safetensors` ecosystem compatibility,
-   no Python/venv/pip requirement for users.

Go remains possible, but Rust currently has the stronger local ML
ecosystem for this project.

The primary Rust risk is **document parsing / Document AI**, not
chunking, retrieval, embeddings, or HTTP serving.

------------------------------------------------------------------------

## 3. High-Level Architecture

``` text
                         ORAG
                    single Rust binary
                           │
        ┌──────────────────┼──────────────────┐
        │                  │                  │
      HTTP API          Ingestion           Query
      (Axum)               │                  │
                           ▼                  ▼
                    Document Parser      Decision Layer
                           │                  │
                           ▼                  ▼
                     Document AST        Retrieval Plan
                           │                  │
                           ▼                  ▼
                        Chunker        Hybrid Retrieval
                           │                  │
                           ▼                  ▼
                       Embedding           Rerank
                           │                  │
                           └─────────┬────────┘
                                     ▼
                               Generation
                                     │
                                     ▼
                             Answer + Sources
```

The architecture should keep major components behind traits/interfaces
so that storage and inference implementations can evolve without
rewriting the RAG core.

------------------------------------------------------------------------

## 4. Storage

### Requirement

The default store should be:

-   embedded,
-   portable,
-   local,
-   easy to back up,
-   require no daemon,
-   ideally represented by a single database file.

### Current direction: SQLite

Desired database:

``` text
~/.orag/orag.db
```

SQLite can hold:

-   documents,
-   chunks,
-   metadata,
-   ingestion state,
-   full-text index,
-   vector data/index,
-   model/config metadata.

This gives ORAG an attractive operational property:

``` bash
cp orag.db orag-backup.db
```

### Full-text retrieval

SQLite FTS5 is the leading choice for lexical/BM25-style retrieval.

### Vector retrieval

`sqlite-vec` is an important initial candidate, but it has not yet been
accepted as the final vector engine.

The major open question is whether its search strategy remains fast
enough at ORAG's intended "medium scale". A benchmark/PoC is required
before committing to it.

The storage layer should therefore be abstracted approximately as:

``` rust
trait Retriever {
    fn search_dense(...);
    fn search_sparse(...);
    fn search_hybrid(...);
    fn get_chunks(...);
}
```

SQLite should be the **default implementation**, not a hard
architectural dependency.

Potential future backends could include Qdrant, PostgreSQL/pgvector, or
another ANN store without changing the higher-level RAG pipeline.

------------------------------------------------------------------------

## 5. Retrieval Strategy

A naive RAG pipeline:

``` text
query → embedding → top-k → LLM
```

is intentionally not the target.

ORAG should support adaptive retrieval:

``` text
                           Query
                             │
                       Decision Model
                             │
              ┌──────────────┼──────────────┐
              │              │              │
          lexical         vector         hybrid
              │              │              │
              └──────────────┼──────────────┘
                             ▼
                            RRF
                             │
                             ▼
                         candidates
                             │
                    rerank if required
                             │
                             ▼
                         context
                             │
                    sufficient?
                       │           │
                      yes          no
                       │           │
                       │      second retrieval
                       │           │
                       └─────┬─────┘
                             ▼
                         generation
```

Likely initial retrieval components:

-   SQLite FTS5 / BM25,
-   dense vector search,
-   Reciprocal Rank Fusion (RRF) or another simple fusion strategy,
-   optional reranking,
-   metadata filters.

The project should benchmark retrieval quality rather than assume
vector-only retrieval is sufficient.

------------------------------------------------------------------------

## 6. Decision Layer

Small decision models such as **Nimble** and **JEV** are of particular
interest.

The decision model should not merely classify a query once. It may
control several stages of the pipeline.

Potential decisions:

``` text
retrieval required?
query rewrite required?
lexical / dense / hybrid?
desired top-k?
reranking required?
context sufficient?
second retrieval required?
which generation model/profile?
```

Example:

``` text
"What is the default timeout?"
        │
        ▼
decision:
  retrieval = yes
  type = factual
  rewrite = no
  top_k = 5
  rerank = no
        │
        ▼
hybrid retrieval
        │
        ▼
context sufficient = yes
        │
        ▼
small generation model
```

More complex request:

``` text
"Compare the authentication architecture across these documents"
        │
        ▼
decision:
  type = multi-document
  rewrite = yes
  top_k = 30
  rerank = yes
        │
        ▼
retrieval → rerank → context evaluation
        │
        ├── insufficient → another retrieval round
        │
        └── sufficient → strong generation model
```

This **adaptive RAG controlled by small decision models** may become one
of ORAG's principal differentiators.

Nimble/JEV still require a dedicated evaluation before one becomes a
default.

------------------------------------------------------------------------

## 7. Local Inference Stack

ORAG should not require Ollama.

The inference engine itself should live inside the Rust process.

Current candidates:

### mistral.rs --- leading candidate

Strong fit for the main LLM/VLM runtime:

-   native Rust crate integration,
-   local LLM/VLM inference,
-   GGUF support,
-   quantization support,
-   CPU / CUDA / Metal options,
-   model load/unload,
-   continuous batching and modern serving capabilities.

Potential roles:

``` text
mistral.rs
├── decision model
├── generation model
└── Document AI / OCR VLM where architecture support permits
```

### llama.cpp via Rust bindings --- important fallback

Advantages:

-   extremely mature local inference ecosystem,
-   huge GGUF model ecosystem,
-   strong CPU/Metal/CUDA support.

Disadvantage:

-   C/C++ FFI rather than a Rust-native stack.

It remains a strong fallback/backend and should not be architecturally
excluded.

### Candle

Advantages:

-   Rust-native ML framework,
-   Hugging Face ecosystem,
-   useful for implementing custom model architectures.

Disadvantage:

-   lower-level than ORAG ideally wants;
-   unsupported architectures may require significant implementation
    work.

Candle is better considered an escape hatch for models that cannot be
run by the primary inference engine.

### ONNX Runtime (`ort`)

A second inference runtime inside the binary is acceptable.

A likely division is:

``` text
                 ORAG
                   │
        ┌──────────┴──────────┐
        │                     │
    mistral.rs               ort
        │                     │
      LLM/VLM            embedding
      decision             reranker
      OCR VLM          small classifiers
      generation
```

"Single binary" does **not** require "single ML runtime".

------------------------------------------------------------------------

## 8. Embeddings and Reranking

`fastembed-rs` is a strong candidate because it can provide local
embedding and reranking without Python.

Relevant model families include:

-   BGE,
-   BGE-M3,
-   Nomic Embed,
-   Qwen embedding models,
-   MiniLM,
-   multilingual rerankers,
-   BGE rerankers.

Exact default models should be chosen through retrieval benchmarks
rather than popularity alone.

The embedding and reranker layers should be replaceable.

------------------------------------------------------------------------

## 9. Document Ingestion

The ingestion layer should distinguish between:

1.  documents that can be parsed reliably with native parsers,
2.  documents requiring visual Document AI / OCR.

Proposed flow:

``` text
                        Document
                           │
                     format detector
                           │
              ┌────────────┴────────────┐
              │                         │
        native parser             visual document
              │                         │
              │                  Document AI model
              │                  OCR + layout
              │                         │
              └────────────┬────────────┘
                           ▼
                      Document AST
                           │
                           ▼
                        Chunker
                           │
                           ▼
                       Embedding
```

A PDF should **not automatically be OCRed**.

Suggested PDF path:

``` text
PDF
 │
 ▼
native extraction
 │
 ├── clean text/layout ───────────────► Document AST
 │
 ├── suspicious/broken layout ──┐
 │                               ▼
 └── scanned/no text ───────► Document AI
                                 │
                                 ▼
                            Document AST
```

This avoids expensive visual inference when it provides no benefit.

------------------------------------------------------------------------

## 10. Document AST

A normalized internal document representation is strongly recommended.

Example:

``` text
Document
├── metadata
├── Page
│   ├── Heading
│   ├── Paragraph
│   ├── Paragraph
│   ├── Table
│   ├── Formula
│   ├── Image
│   └── Footnote
└── Page
    └── ...
```

All parsers should target this representation:

``` text
PDF parser ───────┐
DOCX parser ──────┤
HTML parser ──────┤
Markdown parser ──┼──► Document AST ─► Chunker
OCR / Doc VLM ────┤
XLSX parser ──────┘
```

Benefits:

-   chunking is independent of input format,
-   native parsing and OCR output can be treated consistently,
-   tables/formulas/headings survive ingestion,
-   citations can retain page/block provenance,
-   parser implementations can evolve independently.

------------------------------------------------------------------------

## 11. Chunking

Chunking is **not considered a Rust ecosystem risk**.

Rust has excellent tokenizer support, including the Hugging Face
tokenizer ecosystem.

Initial strategies could include:

``` text
fixed
recursive
structure-aware
semantic
```

A preferred progression is:

1.  structure-aware boundaries,
2.  token budget enforcement,
3.  controlled overlap,
4.  optional semantic splitting.

Blind fixed-size chunking should not be the only/default long-term
strategy.

The Document AST allows chunks to retain structural context such as:

``` text
document
section
heading
page
table
block IDs
```

------------------------------------------------------------------------

# 12. OCR / Document AI Model Shortlist

Modern Document AI models are preferred over treating OCR solely as
`image → text`.

For RAG, the useful output may include:

-   text,
-   reading order,
-   headings,
-   layout,
-   tables,
-   formulas,
-   charts,
-   page structure.

The following models should remain in the ORAG evaluation set.

## 12.1 PaddleOCR-VL 1.5

**Approximate size:** 0.9B\
**License:** Apache 2.0\
**Current status:** Primary quality/size candidate.

Important capabilities:

-   OCR,
-   document parsing,
-   layout understanding,
-   tables,
-   formulas,
-   charts,
-   multilingual document understanding,
-   robustness to difficult document images.

Why it is attractive:

-   unusually small for its capability set,
-   suitable for local inference,
-   broad document-understanding scope,
-   permissive license,
-   good conceptual fit for a local RAG engine.

Primary concern:

-   its model architecture is not currently assumed to work directly in
    `mistral.rs`;
-   integration feasibility must be proven;
-   a custom architecture implementation or alternative runtime may be
    necessary.

**Current assessment:**\
One of the strongest default OCR/Document AI candidates if Rust-native
inference can be made practical.

------------------------------------------------------------------------

## 12.2 MinerU 2.5 Pro

**Approximate size:** 1.2B\
**License:** Apache 2.0\
**Architecture:** Qwen2-VL-derived / related architecture\
**Current status:** Primary integration candidate.

Capabilities of interest:

-   document parsing,
-   headings and paragraphs,
-   reading order,
-   tables,
-   formulas,
-   image/caption relationships,
-   complex PDFs.

The broader MinerU ecosystem supports many document types and
sophisticated parsing, but ORAG should distinguish:

> using the **MinerU model** from embedding the complete Python MinerU
> pipeline.

ORAG does not want a Python runtime dependency.

Why it is attractive:

-   small model,
-   permissive license,
-   VLM architecture closer to already-supported model families,
-   potentially easier Rust inference integration than more custom OCR
    architectures.

Concern:

-   multilingual quality needs direct ORAG evaluation;
-   the full MinerU pipeline contains functionality that would need to
    be reproduced selectively in Rust.

**Current assessment:**\
Possibly the best first PoC model because **integration risk may be
lower than PaddleOCR-VL**, even if PaddleOCR-VL ultimately wins on
quality/size.

------------------------------------------------------------------------

## 12.3 GLM-OCR

**Approximate size:** \~0.9B class\
**License:** model reported under MIT; surrounding components may use
Apache 2.0\
**Current status:** Strong shortlist candidate.

Capabilities:

-   modern document OCR,
-   layout-aware parsing,
-   structured document extraction.

Why it is attractive:

-   small,
-   permissive licensing,
-   potentially excellent open-source-project fit.

Concern:

-   official tooling is currently centered on
    Python/PyTorch/Transformers and common Python document-processing
    libraries;
-   Rust inference path requires validation.

**Current assessment:**\
Should be evaluated alongside PaddleOCR-VL and MinerU rather than
treated as a secondary afterthought.

------------------------------------------------------------------------

## 12.4 dots.mocr / dots.ocr

**Approximate size:** \~3B class for relevant variants\
**License:** custom model license; verify exact redistribution/use
constraints before adoption.\
**Current status:** Quality candidate, weaker default-distribution
candidate.

Capabilities:

-   OCR,
-   document parsing,
-   layout,
-   tables,
-   formulas,
-   structured output.

Why it is attractive:

-   strong reported document parsing quality.

Concerns:

-   larger than the \~0.9--1.2B candidates,
-   custom licensing is less attractive for a broadly reusable
    open-source infrastructure project,
-   Rust runtime compatibility requires evaluation.

**Current assessment:**\
Useful benchmark/reference model. Not currently preferred as ORAG's
default.

------------------------------------------------------------------------

## 12.5 DeepSeek-OCR

**Approximate size:** multi-billion parameter class depending on
release/configuration\
**Current status:** Evaluation/reference candidate.

Capabilities of interest:

-   OCR/document understanding,
-   visual text processing,
-   potentially strong complex-page understanding.

Concerns:

-   heavier than the smallest candidates,
-   license and exact redistribution terms must be checked for the
    selected checkpoint,
-   Rust inference support must be demonstrated,
-   may provide insufficient benefit over \~1B-class models for ORAG's
    default profile.

**Current assessment:**\
Worth benchmarking, but currently less attractive for the default
lightweight profile.

------------------------------------------------------------------------

## 12.6 olmOCR

**License:** Apache 2.0 for relevant open components/models; exact
selected artifact must still be verified.\
**Current status:** Quality/reference candidate.

Strengths:

-   strong focus on high-quality PDF/document extraction,
-   sophisticated document parsing approach,
-   useful benchmark/reference for difficult PDFs.

Concerns:

-   generally heavier operationally than the smallest OCR/VLM
    candidates,
-   its surrounding pipeline is not designed around ORAG's
    single-Rust-binary constraint,
-   integration cost may be substantially higher.

**Current assessment:**\
Important quality reference, but unlikely to be the first default
embedded Document AI model.

------------------------------------------------------------------------

## 12.7 Preliminary OCR Ranking --- Not Final

There is **no final winner yet**.

Current design intuition:

``` text
Quality/size interest:
    PaddleOCR-VL 1.5

Potential Rust integration advantage:
    MinerU 2.5 Pro

Licensing + small-model interest:
    GLM-OCR

Quality/reference alternatives:
    dots.mocr
    DeepSeek-OCR
    olmOCR
```

The final choice must be based on an ORAG-specific PoC rather than
upstream benchmark claims alone.

Required evaluation dimensions:

-   text accuracy,
-   Turkish,
-   English,
-   multilingual documents,
-   reading order,
-   multi-column PDFs,
-   tables,
-   formulas,
-   scanned PDFs,
-   photographs of documents,
-   latency,
-   peak RAM/VRAM,
-   Metal support,
-   CUDA support,
-   CPU fallback,
-   quantization,
-   Rust integration effort,
-   license,
-   model download size.

------------------------------------------------------------------------

## 13. Model Lifecycle / Memory

ORAG should not assume every model remains resident simultaneously.

Example ingestion lifecycle:

``` text
load Document AI model
        ↓
parse difficult pages
        ↓
unload Document AI model
        ↓
embedding
        ↓
index
```

Example query lifecycle:

``` text
decision model
      ↓
retrieval
      ↓
reranker
      ↓
generation model
      ↓
answer
```

This makes a multi-model architecture practical on machines with limited
RAM/VRAM.

The model manager should eventually support:

-   download,
-   cache,
-   load,
-   unload,
-   quantized variants,
-   hardware-aware profiles,
-   offline mode.

------------------------------------------------------------------------

## 14. Hardware / Model Profiles

Possible future profiles:

``` text
light
balanced
quality

cpu
apple-silicon
cuda
```

Example concept:

``` toml
[profile.light]
embedding = "..."
reranker = "..."
decision = "..."
document_ai = "..."
generation = "..."
```

Profiles should be configuration, not architectural forks.

------------------------------------------------------------------------

## 15. Initial API

### Add document

``` http
POST /v1/documents
```

Conceptual request:

``` json
{
  "content": "...",
  "metadata": {
    "source": "manual",
    "project": "foo"
  }
}
```

File upload/multipart ingestion will also be required for real document
support.

Conceptual response:

``` json
{
  "document_id": "abc123",
  "status": "indexed",
  "pages": 42,
  "chunks": 317,
  "warnings": []
}
```

Warnings are important. ORAG should not silently index obviously broken
extraction.

Example:

``` json
{
  "warnings": [
    "Pages 18-19 contained no reliable extractable text"
  ]
}
```

### Query

``` http
POST /v1/query
```

Conceptual request:

``` json
{
  "query": "How does authentication work?"
}
```

Conceptual response:

``` json
{
  "answer": "...",
  "sources": [
    {
      "document_id": "...",
      "chunk_id": "...",
      "page": 12,
      "score": 0.91
    }
  ],
  "trace": {
    "strategy": "hybrid",
    "reranked": true,
    "retrieval_rounds": 1
  }
}
```

A trace/debug facility is desirable because RAG quality is difficult to
improve when retrieval decisions are opaque.

------------------------------------------------------------------------

## 16. Proposed Rust Components

These are candidates, not frozen dependencies.

``` text
HTTP/API             Axum
Async                Tokio

Database             SQLite / rusqlite
Lexical retrieval    SQLite FTS5
Vector retrieval     sqlite-vec (PoC required)

Embedding            fastembed-rs / ONNX
Reranking            fastembed-rs / ONNX
ONNX runtime         ort

LLM/VLM runtime      mistral.rs (leading candidate)
Fallback backend     llama.cpp bindings
Custom model work    Candle where needed

Tokenization         tokenizers
Model formats        GGUF / Safetensors / ONNX as appropriate
```

------------------------------------------------------------------------

## 17. What ORAG Should Avoid Initially

The first version should avoid:

-   mandatory Docker,
-   mandatory Python,
-   mandatory Ollama,
-   PostgreSQL,
-   Elasticsearch,
-   external vector databases,
-   distributed architecture,
-   Kubernetes,
-   workflow engines,
-   UI/dashboard work,
-   implementing every document format before core retrieval quality is
    proven.

The project should remain small enough that its retrieval behavior can
be understood and benchmarked.

------------------------------------------------------------------------

## 18. Open Questions / Required PoCs

### P0 --- before architecture is considered stable

1.  **SQLite vector scalability**
    -   benchmark `sqlite-vec`,
    -   determine acceptable chunk/vector count,
    -   test filtered retrieval,
    -   measure p50/p95 latency,
    -   decide whether an embedded ANN alternative is needed.
2.  **OCR/Document AI inference in Rust**
    -   PaddleOCR-VL 1.5,
    -   MinerU 2.5 Pro,
    -   GLM-OCR,
    -   test direct `mistral.rs` compatibility,
    -   quantify work required for unsupported architectures,
    -   test Metal/CUDA/CPU,
    -   test quantization.
3.  **Decision models**
    -   evaluate Nimble,
    -   evaluate JEV,
    -   determine whether a dedicated decision model actually improves
        latency/quality over deterministic routing or a small general
        LLM.
4.  **Retrieval benchmark**
    -   FTS-only,
    -   dense-only,
    -   hybrid,
    -   hybrid + rerank,
    -   adaptive retrieval.

### P1

5.  Native PDF extraction quality.
6.  DOCX/PPTX/XLSX parser quality.
7.  Document AST schema.
8.  Structure-aware chunking.
9.  Model download/cache/version management.
10. Citation/provenance schema.

------------------------------------------------------------------------

## 19. Current Working Hypothesis

The architecture currently converges toward:

``` text
                          ORAG
                   local-first Rust binary
                             │
        ┌────────────────────┼────────────────────┐
        │                    │                    │
      Axum               Document AST          Models
                             │                    │
                    ┌────────┴───────┐     ┌──────┴──────┐
                    │                │     │             │
              native parsers     Doc VLM mistral.rs     ort
                    │                │     │             │
                    └────────┬───────┘  Decision      Embed
                             │          Generator     Rerank
                             ▼
                          Chunker
                             │
                             ▼
                    SQLite / FTS / Vector
                             │
                             ▼
                     Adaptive Retrieval
                             │
                             ▼
                         Generation
                             │
                             ▼
                     Answer + Evidence
```

The most important unresolved engineering risk is no longer "Can Rust
implement a RAG system?"

It can.

The important questions are:

1.  **Which embedded vector implementation gives ORAG sufficient
    scale?**
2.  **Which modern Document AI model can be embedded cleanly in the Rust
    inference stack?**
3.  **Do tiny decision models materially improve retrieval
    quality/latency?**

Those three PoCs should drive the next architectural decisions.
