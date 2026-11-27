# ORAG engine interface

A program that starts the `orag` executable (the desktop app is the first)
talks to it in three ways: the command line, a few environment variables and
the exit code, and the HTTP API (`docs/api.md`). This page is the contract for
the first three. `crates/orag/tests/interface.rs` checks the flag table
(section 9) against the real command line: every flag listed exists, and every
flag but a deprecated one shows in `--help`.

`orag` is the CPU build (on macOS, Metal); `orag-vulkan` is the Windows and
Linux build with Vulkan. Both take the same commands and flags.

## 1. Starting

```
orag serve --exit-on-stdin-close [--allow-origin <ORIGIN>]...
```

- `--exit-on-stdin-close`: the stop signal. The engine watches its stdin and
  shuts down gracefully when it reaches end of file, so a program stops it by
  closing the pipe, and an engine whose parent died does not linger.
- `--allow-origin <ORIGIN>`: also accept requests from this web origin
  (`scheme://host[:port]`, repeatable) and answer CORS for it. See "Allowed
  origins" and "Named origins" in `docs/api.md`. A malformed origin is refused
  at start.
- `--ui-port-hint <PORT>`: deprecated (see the flag table).

Set `ORAG_HOME` (section 4) to the data folder. `orag serve` takes an
exclusive lock on it; a second `serve` on the same folder fails, except that
a supervised one (`ORAG_SUPERVISED=1`) retries the lock for up to 5 seconds, for the restart after exit code 75.

## 2. Stdout

`orag serve` prints its first stdout line once the HTTP port is bound:

```
orag listening on http://<addr>
```

`<addr>` is the API's address (`bind` in `config.toml`; with port `0` the
real port). A program reads this line, not a fixed port. This is the only
stdout line a program may rely on (D-016).

A second line, `orag ui on http://<addr>`, is printed while the built-in page
is on. It is deprecated: it disappears in the next tag, and a program must not
rely on it.

Logs go to stderr.

## 3. Exit codes

| Code | Name | Meaning |
|---|---|---|
| `0` | | Stopped on request (stdin closed with `--exit-on-stdin-close`, or a termination signal) |
| `75` | `EXIT_RESTART` | Setup finished (the models were installed): start the engine again |
| `76` | `EXIT_GPU_RETRY` | The user asked for the graphics card to be tried again (`POST /v1/system/gpu-retry`); only an engine started with `ORAG_GPU_RETRY=1` ends this way. Start it again without the fallback variables |
| `3` | | `eval bench` and `eval retrieval` with `--exit-on-stdin-close`: stdin closed before the run ended |

Any other code is a failure; the reason is on stderr.

## 4. Environment

| Variable | Set by | Meaning |
|---|---|---|
| `ORAG_HOME` | The user, or the program that starts `orag` | The data folder: `config.toml`, the database, the models. An absolute path; empty or relative is an error. Default `~/.orag` |
| `ORAG_SUPERVISED` | The supervisor (`orag start`, the desktop app) | `1`: the program restarts the engine on exit code 75. A model download or import that finishes setup then ends the process with 75 (the page says the restart is automatic); without it the user restarts by hand. The data-folder lock is retried for up to 5 seconds |
| `ORAG_GPU_RETRY` | The desktop app | `1`: the program handles exit code 76. The engine then offers `POST /v1/system/gpu-retry` (`gpu.retry` in the API) when it fell back to the CPU. `orag start` removes it |
| `ORAG_GPU_FALLBACK_REASON` | The desktop app | Why this CPU engine runs without the graphics card (`loader-missing`, `no-eligible-device`, `probe-timeout`, `start-timeout`, `start-failed`, `crashed-twice`, `exit-<n>`). The engine reports it as `gpu.reason` in the API; it changes nothing else |
| `ORAG_GPU_DEVICE` | The desktop app | `orag-vulkan` only: the index of the GPU to use, as `gpu-probe` lists it (`devices[].index`). An index that is not listed is an error. `gpu = "off"` in `config.toml` still wins |
| `ORAG_VULKAN_ALLOW_SOFTWARE` | Tests | `1`: `orag-vulkan` may choose a software rasteriser (lavapipe) as the device. Not for users |

A program must remove a variable it does not mean to pass on: the desktop
app's benchmark children start with `ORAG_GPU_DEVICE` and
`ORAG_VULKAN_ALLOW_SOFTWARE` removed.

## 5. config.toml

`$ORAG_HOME/config.toml` is read once at start. Its keys are the table in
"Configuration" in `README.md`. A program that starts the engine must not rely
on a new key reaching an existing file: the file is written commented out on
first start and never rewritten, so a key added in a later version is absent
from a folder made earlier. A program that needs a setting passes it by a flag
or variable from this page, or writes the key itself.

## 6. gpu-probe

```
orag-vulkan gpu-probe
```

Prints one JSON line on stdout and exits 0:

```json
{"backend":"vulkan","devices":[{"index":0,"name":"NVIDIA GeForce RTX 3060","kind":"discrete","memory_total":12884901888,"memory_free":11811160064}],"chosen":0}
```

| Field | Meaning |
|---|---|
| `backend` | `vulkan` (`orag-vulkan`) or `cpu` (the CPU `orag`) |
| `devices[]` | The GPU devices, with `index` (llama.cpp's, what `ORAG_GPU_DEVICE` takes), `name`, `kind` (`discrete`, `integrated` or `other`), `memory_total` and `memory_free` in bytes. Empty on the CPU build |
| `chosen` | The `index` ORAG would use: the discrete device with the most free memory, at least 2 GiB free; `null` when there is none |

`gpu-probe --fit <gguf>` prints `{"backend":"vulkan","fit":"<outcome>"}`, where
the outcome is `fitted n_gpu_layers=<n>`, `does_not_fit` or `error`.
`gpu-probe --load <gguf>` prints `{"backend":"vulkan","offload":{...}}`:
`layers_on_gpu`, `layers_total`, `cpu_tensor_overrides`, `device` and `reason`.
Both fail on the CPU build. The probe can hang on a broken driver; the caller
should set a time limit and kill it.

## 7. eval bench and eval retrieval

The desktop app's graphics card benchmark runs these. Both take `--corpus`,
`--dataset`, `--out` and `--index-dir`, and read the models from `ORAG_HOME`.

`orag eval bench --tier <default|large> --index-dir <dir> --corpus <dir>
--dataset <file> --out <file>` answers the dataset's questions on this
executable's backend over an index built once:

- `--build-index` builds the index in an empty `--index-dir`; without it the
  directory's existing index is reused and nothing is embedded again.
- `--index-only` builds or checks the index and answers nothing (it needs the
  embedder only). `--first-only` answers only the first question, for a cold
  start. The two conflict.
- `--out` must not exist; the run is written there as JSON with
  `"schema": "orag-bench-run/1"`. `status` is `ok`, or `not-installed` (the
  tier's models are missing; `tier` and `missing` say which). On failure no
  file is left.
- stdout carries one JSON line per answer, `{"event":"answer","id":...,"ms":...}`,
  as it is given.
- `gpu = "off"` in `config.toml` is set aside: the run places the models by
  its own probe.

`orag eval retrieval --no-context-budget --gpu-auto --index-dir <dir> --corpus
<dir> --dataset <file> --out <file>` compares lexical, dense and hybrid
retrieval. `--no-context-budget` scores the first 8 candidates without loading
the answer model. `--gpu-auto` places the embedder by itself, setting
`gpu = "off"` aside. The report is printed and also written to `--out` as
JSON. `--index-dir` keeps the index in that directory, or queries the index an
earlier run left there.

`--exit-on-stdin-close` works as in section 1, with exit code 3.

## 8. Compatibility

Within a minor line (`0.6.x`), the flags, exit codes, variables and the first
stdout line on this page only gain options. A removal needs a minor version
bump and a CHANGELOG line marked **interface**. A flag marked "Deprecated" may
go in the version its Meaning names. Output fields of the JSON a command prints
may be added; a program must ignore fields it does not know.

## 9. Flag reference

"Since" is the first version that had the flag, as best git shows; `0.6` where
it is not clear.

| Command | Flag | Since | Meaning |
|---|---|---|---|
| `serve` | `--exit-on-stdin-close` | 0.3 | Stop gracefully when stdin reaches end of file |
| `serve` | `--allow-origin` | 0.6 | Also accept and answer CORS for this web origin (`scheme://host[:port]`); repeatable |
| `serve` | `--ui-port-hint` | 0.4 | Deprecated: tries this port first for the built-in page; accepted and ignored once the page is removed (later in 0.6) |
| `gpu-probe` | `--fit` | 0.6 | Run llama.cpp's memory fit once for this model file; conflicts with `--load` |
| `gpu-probe` | `--load` | 0.6 | Load this model file as a generator would and print what went to the GPU |
| `eval bench` | `--corpus` | 0.6 | The documents the questions are about |
| `eval bench` | `--dataset` | 0.6 | The questions |
| `eval bench` | `--index-dir` | 0.6 | Where the index is built (`--build-index`, an empty directory) or reused |
| `eval bench` | `--tier` | 0.6 | The generation tier: `default` or `large` |
| `eval bench` | `--build-index` | 0.6 | Build the index in `--index-dir` |
| `eval bench` | `--index-only` | 0.6 | Only build or check the index; answer nothing |
| `eval bench` | `--first-only` | 0.6 | Answer only the first question (another cold start) |
| `eval bench` | `--out` | 0.6 | The run's JSON file; must not exist yet |
| `eval bench` | `--exit-on-stdin-close` | 0.6 | Exit with code 3 when stdin closes |
| `eval retrieval` | `--corpus` | 0.1 | The documents |
| `eval retrieval` | `--dataset` | 0.1 | The labeled questions |
| `eval retrieval` | `--out` | 0.1 | Also write the report as JSON |
| `eval retrieval` | `--no-context-budget` | 0.3 | Score the first 8 candidates without loading the answer model |
| `eval retrieval` | `--index-dir` | 0.6 | Keep the index in this directory, or query the index an earlier run left there |
| `eval retrieval` | `--gpu-auto` | 0.6 | Place the embedder by itself, setting `gpu = "off"` aside |
| `eval retrieval` | `--exit-on-stdin-close` | 0.6 | Exit with code 3 when stdin closes |
