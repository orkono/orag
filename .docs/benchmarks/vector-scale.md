# Vector-scale benchmark (D-003 acceptance gate)

The v1 scale promise: dense search over **100 000 chunks × 1024 dimensions**
answers with a warm **p95 below 250 ms** on the reference machines. The run
uses synthetic unit vectors in a throwaway database (sqlite-vec 0.1.9, exact
k-NN, k = 50).

What a sample measures: one search as the server runs it, including opening
its own read connection (every query does). "Warm" means 3 untimed queries
first, so the OS page cache holds the vectors. Every search must return k
hits, or the run fails. "insert s" counts SQLite inserts only, not vector
generation.

```bash
orag eval vector-scale --chunks 100000 --dimensions 1024 --queries 50
orag eval vector-scale --chunks 300000 --dimensions 1024 --queries 20   # informational
```

On Linux, `/tmp` can be RAM-backed (tmpfs); add `--work-dir <dir>` on the
disk that holds `ORAG_HOME` so the run measures that disk.

## macOS, Apple Silicon

- Machine: Apple M5 Max, 128 GB RAM, macOS 26.4
- Date: 2026-10-04
- Version: `orag 0.1.0-alpha.25` (release build)

100k (the gate):

| platform | version | chunks | dims | k | queries | insert s | p50 ms | p95 ms | max ms | target p95 | result |
|---|---|---|---|---|---|---|---|---|---|---|---|
| macos-aarch64 | 0.1.0-alpha.25 | 100000 | 1024 | 50 | 50 | 10.2 | 93.2 | 99.9 | 103.4 | 250 | PASS |

300k (informational, beyond the v1 promise):

| platform | version | chunks | dims | k | queries | insert s | p50 ms | p95 ms | max ms | target p95 | result |
|---|---|---|---|---|---|---|---|---|---|---|---|
| macos-aarch64 | 0.1.0-alpha.25 | 300000 | 1024 | 50 | 20 | 30.5 | 280.2 | 294.0 | 298.0 | 250 | FAIL |

Gate: **PASS** on this machine. The 300k run is above the target, as expected
for exact search: latency grows linearly with the number of vectors
(about 1 ms per 1 000 chunks at 1024 dimensions here).

This machine is faster and has more memory than the planned macOS
reference (Apple M-series, 16 GB). Owner decision (2026-10-05, recorded
under D-003): for v0.1.0 this machine is the macOS reference.

## Linux, x86-64 (GitHub-hosted runner)

- Machine: GitHub `ubuntu-22.04` runner, x86-64, 4 vCPU, 16 GB RAM
  (MemTotal 16 371 460 kB; reference-class check passed)
- Date: 2026-10-05, workflow `vector-scale` run 37294628273
- Version: `orag 0.1.0-alpha.29` (release build, `--work-dir "$RUNNER_TEMP"`)

100k (the gate):

| platform | version | chunks | dims | k | queries | insert s | p50 ms | p95 ms | max ms | target p95 | result |
|---|---|---|---|---|---|---|---|---|---|---|---|
| linux-x86_64 | 0.1.0-alpha.29 | 100000 | 1024 | 50 | 50 | 17.5 | 151.4 | 152.2 | 158.5 | 250 | PASS |

300k (informational, not a v1 promise):

| platform | version | chunks | dims | k | queries | insert s | p50 ms | p95 ms | max ms | target p95 | result |
|---|---|---|---|---|---|---|---|---|---|---|---|
| linux-x86_64 | 0.1.0-alpha.29 | 300000 | 1024 | 50 | 20 | 52.8 | 431.9 | 434.6 | 440.9 | 250 | FAIL |

Gate: **PASS**. About 1.5x the M5 Max latency, still well below the target.

## Pending before Task 24

The gate must pass on both references **with the version being released**
(the alpha.25 rows above do not count):

- macOS: this machine (Apple M5 Max), rerun on the release candidate build.
- Linux: the manual `vector-scale` workflow on GitHub's ubuntu-22.04 runner
  (x86-64, 4 vCPU, 16 GB). It stands in for the 8-core box: exact k-NN
  search uses one core.

## Follow-up after v0.1.0

- Apple M-series laptop with 16 GB RAM and an x86-64 8-core, 16 GB Linux
  machine, the hardware the D-003 promise names. Until both are recorded,
  release notes say the promise was measured on the two machines above. A
  FAIL takes the "Revisit" path below and corrects the published promise in
  the next release.

If the 100k run fails on a reference, do not change the target and do not start
Task 24. Add a "Gate failed" section with the numbers and take D-003's
"Revisit" path, in order: (1) int8 quantization with f32 rescoring,
(2) Matryoshka truncation to 512 dimensions, (3) ANN, each with its own plan.
