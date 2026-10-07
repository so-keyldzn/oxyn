# ADR-0056 — Local CPU embeddings rank the AI context, lexical first

**Status:** proposed · **Date:** 2026-10-07

## Context

Before a question reaches a model, `ContextBuilder::build` keeps at most 24
relations of the catalog ([AI-PROVIDERS](../AI-PROVIDERS.md#what-the-ai-sees-of-the-schema-and-when-it-is-read)).
Its first step, selection, is `oxyn_catalog::search()`: a lexical score on
names, fields and comments, then the order of the paths when no term matches.
A question asked in French about tables named in English ("quels clients ont
commandé hier ?" against `customers` and `orders`) matches nothing, and the 24
relations kept are the first 24 in alphabetical order. On a 5,000-table
database, that is the whole failure [ADR-0006](0006-ai-privacy-tiers.md)
names: wrong pruning produces wrong queries.

Ranking by meaning needs text embeddings. The user who asked for it has
neither Ollama nor an API key: an embedding endpoint behind
`OpenAiCompatibleProvider` would not work for them, nor for anyone who
installed nothing beside Oxyn. The embeddings therefore have to be computed
in the process, on the CPU, with nothing to install.

A spike measured it on 2026-10-07 (Apple Silicon arm64, 10 cores, release
build; recorded in [RESEARCH-NOTES](../RESEARCH-NOTES.md#local-embeddings--checked-on-2026-10-07)):

- the model `ibm-granite/granite-embedding-97m-multilingual-r2` (Apache-2.0,
  ModernBERT, 97,441,152 parameters in bf16, multilingual, published
  2026-04-20) converts from its ONNX export with burn-onnx 0.22.0 **without
  a missing operator**, and its Burn output matches onnxruntime's to a maximum
  gap of 4.2e-7 (cosine 1.0);
- **latency** about 14 ms for one sentence, about 440 ms for 256 table names;
- **memory**: peak RSS 1.15 GB when loading the published safetensors, about
  750 MB when loading a burn-store `.bpk` file in f32; the tokenizer loaded
  alone, 286 MB;
- **quality**: cosine between a question and the relevant table 0.909 in
  English and 0.865 in French, against at most 0.773 for irrelevant tables.
  The gap is about 0.1: enough to reorder, not enough to trust alone;
- **build**: 198 crates, none `-sys`, all under licenses `deny.toml` already
  accepts — the only MPL-2.0 one, `colored`, included. A cold
  compilation of the inference crate takes 80 s of wall time, 45 s of which is
  burn-flex on the critical path; the demo binary weighs 11.7 MB, 8.6 MB
  stripped;
- the pure-Rust tokenizer (`tokenizers` with `fancy-regex`) gives the same
  ids as the default Oniguruma build on 17 multilingual cases.

The `oxyn-embed` crate, measured the same day on the same machine in release,
confirms these orders of magnitude: a cold load in 0.67 s (hashing 415 MB,
mapping, warm-up), one question in 18 ms, 256 table names in 0.63 s (about
3.5 times slower in the dev profile: 2.2 s); **795 MB** once loaded; the real
download and conversion in 14 s, **peaking at 1.45 GB**.

## Decision

**Oxyn computes text embeddings locally, on the CPU, in a new crate
`oxyn-embed`, and uses them only to rank the relations the AI context keeps:
lexical first, semantic to break ties and to order what the lexical score
misses.** The option is off by default.

### The crate and its place

`crates/oxyn-embed` carries one subject: a pinned embedding model — its
download, its verification, its conversion and its CPU inference. It
depends on `oxyn-core` alone (for `CancelToken`) among Oxyn's crates. Its
public surface: `ModelStore` (the model's directory: `status`, `download`,
`remove`), `Embedder` (`load`, `embed`), `OnDemandEmbedder` (loads at the
first request, drops after inactivity), `Embedding` (384 `f32`,
L2-normalized), `cosine`, and the `pinned` constants.

**`oxyn-desktop` is the only crate that depends on `oxyn-embed`.** `oxyn-ai`
does not: `ContextBuilder` receives per-relation scores — a map from
`CatalogPath` to `f32`, carrying no text — computed by `oxyn-desktop`. The
gateway of [I-04](../../CLAUDE.md#i-04) stays the only place that renders a
context; a score can reorder relations, it cannot add a word to a prompt.
`oxyn-ai`, its tests and an external agent's path keep compiling without Burn,
and removing the semantic step touches neither crate's tests. The selection
the catalog fill reads (`wanted_relations`) takes the same scores, so what is
described and what is rendered stay one computation.

### The model, pinned by bytes

| | Value |
|---|---|
| Repository | `ibm-granite/granite-embedding-97m-multilingual-r2` |
| Revision | `835ad14087e140460703cf0fae09f97d469d65c2` (a commit, not a branch) |
| `model.safetensors` | 194,889,568 bytes, SHA-256 `f3ea88b230492811046145513710e76b4cc8c2ad49e8708da0e7247e548903be` |
| `tokenizer.json` | 25,301,672 bytes, SHA-256 `4f2842d568e2724370aec203652a42ac783c7937f8347a1a2cc7506d71f1582f` |
| `onnx/model.onnx` | 390,004,608 bytes, SHA-256 `68e592b160673d30250824c1116bc6ab33f70efb22b97c9e1d7ce1e69c1c9d70` — **generation source only, never downloaded by Oxyn** |
| Pooling | the `[CLS]` token, then L2 normalization; 384 dimensions |
| Truncation | 512 tokens, `[CLS]` and `[SEP]` included |

`oxyn_embed::pinned::MODEL_ID` names the model and its revision. Any vector
kept by a caller is stored next to it and discarded when it changes: two
models' vectors are not comparable.

### The engine: Burn, with generated code committed

The workspace pins `burn`, `burn-flex` and `burn-store` at exactly `=0.22.0`,
and `tokenizers` at exactly `=0.23.2`:

- `burn` with `default-features = false`, `std` and `flex`;
- `burn-flex`, the pure-Rust CPU backend, declared **directly** with `std`,
  `simd` and `rayon`. Through `burn/flex` alone it gets `std` only and runs on
  one core without SIMD; the direct declaration is what feature unification
  needs;
- `burn-store` with `default-features = false`, `std`, `safetensors` and
  `memmap`;
- `tokenizers` with `default-features = false` and `fancy-regex`: no
  Oniguruma, no C to build.

The model's Rust code is **generated once** by burn-onnx 0.22.0 from the
pinned ONNX export and committed under `crates/oxyn-embed/src/generated/`:
`model.rs` (the graph), `weights_map.rs` (safetensors keys onto its
parameters) and `residual.bpk`, the 43 parameters the safetensors does not
carry — rotary frequencies, LayerNorm biases, scalars that the ONNX export
folded into constants. There is no `build.rs`: burn-onnx, its protobuf stack
and the 390 MB ONNX file stay out of every build. `crates/oxyn-embed/codegen/`
regenerates all three; nothing in `generated/` is edited by hand.

**A test locks the output.** Without `residual.bpk`, the model still runs and
returns vectors that are silently wrong. The tests compare the embeddings of
fixed multilingual sentences with reference values computed by the upstream
model outside Oxyn, so that a regeneration, a Burn bump or a forgotten
constant fails the build instead of degrading every ranking.

**A lint exemption scoped to one module.** The generated code holds 69
`unwrap()` and some 250 `as` conversions (counted 2026-10-07). They depend on
tensor shapes, never on values coming from a server or a file:
`Embedder::embed` only feeds rectangular `[batch, length]` inputs with
`batch ≥ 1` and `2 ≤ length ≤ 512`, and the tests run both ends of that
range. The `#[allow]`s sit on `mod model;` and `mod weights_map;` in
`generated/mod.rs`, nowhere else; the rest of the crate keeps the workspace's
lints, [I-09](../../CLAUDE.md#i-09) included. No `unsafe` is written:
memory mapping lives inside burn-store.

### Off by default, turned on by the human

The option is a workspace preference
([ADR-0013](0013-preferences-workspace.md)), **off by default**, which the
default policy reserves to `Actor::Human`: no agent turns it on, so no agent
starts a download. While it is off, nothing of `oxyn-embed` touches the
network, the disk or memory, and selection is exactly today's.

Turning it on starts `ModelStore::download` in the background, with a
progress report and cancellation. Turning it off unloads the model and
deletes its directory (`ModelStore::remove`).

### The download: two sources, one set of accepted bytes

`model.safetensors` and `tokenizer.json`, 220,191,240 bytes together, are
fetched from:

1. `https://huggingface.co/<repository>/resolve/<revision>/<file>`;
2. if that fails or returns bytes that fail verification:
   `https://github.com/so-keyldzn/oxyn/releases/download/embedding-model-835ad140/<file>`,
   a release of this repository carrying the same files byte for byte.

**A file is accepted on its pinned size and SHA-256, never on its source.**
It is streamed to `<name>.part`, hashed as it arrives, cut off as soon as it
exceeds its pinned size, and renamed into place only when size and checksum
match; any failure removes the `.part`. HTTPS only, redirections included (at
most 5), TLS verified ([ADR-0052](0052-verified-tls-outside-local.md)), a
10 s connection timeout and 60 s of silence at most between two chunks. The
outgoing request carries no user data: a fixed URL and the `oxyn/<version>`
user agent.

The release `embedding-model-835ad140` is published as a **pre-release**:
GitHub's "latest release" is the most recent non-prerelease, non-draft one,
and the updater reads `releases/latest/download/latest.json`
([ADR-0051](0051-automatic-updates-from-github-releases.md)). Published as an
ordinary release, it would become "latest" and stop every update. It carries
the Apache-2.0 license text and the model's attribution beside the two files.

The files live under `<data dir>/models/granite-embedding-97m-multilingual-r2-835ad140/`.
After the download, the bf16 safetensors is widened to f32 and written as
`model.bpk` (389,816,832 bytes), which loads by memory mapping; the
safetensors is deleted only once the converted file is verified. What stays on
disk is `model.bpk` and `tokenizer.json`, about 415 MB. Download and
conversion together take 14 s and peak at **1.45 GB** of RSS, once.

The checksum of `model.bpk` is a constant too —
`d5ac67b8e7e85e63ba433faebbe3e5537732ab7e27dde9cf3422710c6abce719` —, so a
file damaged later is refused like a downloaded one. That takes one step
around burn-store: `BurnpackStore` records each parameter's `ParamId`, a
random `u64` drawn when the model is built, so two conversions of the same
weights would differ. The conversion writes through `burn_pack::Writer` with
every `param_id` set to `None`; the ids only serve to resume training, and the
widening from bf16 is exact, so the file is a function of the weights alone.
No file is parsed before its size and checksum are checked.

### Loaded on demand, dropped after five minutes

`OnDemandEmbedder` loads the model at the first request and drops it **five
minutes** after its last use. Every call to it blocks — milliseconds on a
loaded model, 0.67 s for a cold load — and runs on the blocking pool, never on
the UI thread nor on an async executor thread ([I-05](../../CLAUDE.md#i-05)).
burn-flex and `tokenizers` use rayon's global pool.

### What is embedded, and how it ranks

- the question: the text `ContextBuilder::focused_on` receives;
- a relation: its qualified name and its comment, as the catalog summary
  carries them — known as soon as a schema is listed, before any field is
  read, so the vector does not change when the fields load.

Relation vectors are kept in memory only, in `oxyn-desktop`, keyed by
`MODEL_ID` and the embedded text; they are never written to disk, never
logged, never sent. 384 `f32` weigh 1,536 bytes: at the catalog cache's
ceiling of 50,000 objects, at most 77 MB.

The ranking, in `oxyn-ai`, keeps the existing rules and adds one key:

1. `@` mentions first, as today;
2. relations with a lexical score, by that score; **equal scores by cosine**,
   then by path;
3. relations without a lexical score, **by cosine**, then by path — instead of
   by path alone.

No cosine threshold: a weak lexical match still outranks a strong semantic
one. With a gap of 0.1 between relevant and irrelevant tables, a threshold
would be a tuned number that holds for the measured cases and silently fails
elsewhere; the order alone degrades gracefully.

**The semantic step never fails a question and never makes it wait long.**
It is bounded at **2 seconds** per question, loading included, and embeds
missing relations in batches, checking the deadline between two batches.
What is not computed in time ranks as today, after what is; what is computed
stays cached for the next question. While the model is downloading, absent,
corrupt or failing to load, the ranking is today's, and the panel says that
semantic ranking is unavailable and why.

## Consequences

* **+** A question in one language finds tables named in another, and a
  question that names no table gets the closest ones instead of the
  alphabetically first: the 24 relations kept stop being arbitrary.
* **+** Nothing leaves the machine to compute it: no provider, no key, no
  installation. The privacy tier is untouched under every value.
* **+** Lexical results keep their order; the semantic step only orders what
  was already tied or unranked. A wrong vector can demote nothing that
  matched by name.
* **+** `oxyn-ai` does not depend on Burn: its tests and the external-agent
  path keep their compile time, and the gateway stays one function.
* **−** **795 MB of RAM while the model is loaded**, five minutes after
  the last question at most. On an 8 GB machine already holding large
  results, that is the difference between fitting and swapping.
* **−** **Burn is 0.x and breaks its API between minor versions.** Every bump
  regenerates `model.rs`, `weights_map.rs` and `residual.bpk`, and the
  checksum of `model.bpk` changes with burn-store's format — every user then
  converts again. The four exact pins exist so this never arrives through
  `cargo update`.
* **−** **Forgetting a residual constant gives wrong vectors, not an error.**
  Only the reference-output test catches it; a regeneration that updates that
  test's expected values instead of investigating defeats it.
* **−** A module of generated code exempt from the workspace's lints: 69
  `unwrap()` that nothing reviews line by line, held only by the shape
  argument above and the tests at both ends of the range.
* **−** burn-flex and `tokenizers` share rayon's **global** pool: a batch of
  embeddings competes for every core with whatever else uses it in the
  process, and can make another task wait.
* **−** **Hugging Face availability.** The fallback release covers an outage
  or a removal upstream, not a network that blocks both hosts: behind such a
  network, the option cannot be turned on. The fallback is ours to publish
  and keep, and its absence is a 404 that the download error names.
* **−** **Without its features, burn-flex runs on one core, without SIMD**,
  several times slower — and nothing fails. Only the direct declaration in the
  root `Cargo.toml`, commented, keeps them.
* **−** **+80 s of cold compilation** in CI and for any build that compiles
  `oxyn-desktop` from scratch; 45 s of it on the critical path.
* **−** About 415 MB of disk for a user who turns the option on, a download
  of 220 MB at first use, and **1.45 GB of RSS at the peak** of that
  download and conversion — above the 1 GB of the loaded model, once.
* **−** The checksum of `model.bpk` holds only because the conversion bypasses
  `BurnpackStore` and clears every `ParamId`: a Burn bump that changes
  `burn_pack::Writer` or the parameter collection can make the file
  non-reproducible, and every user's conversion is then refused until the
  constant is regenerated.
* **−** Measured on macOS arm64 only. The release also builds Linux x86_64 and
  arm64, where burn-flex's SIMD paths and the latency are unmeasured.

**Exit cost:** low, and bounded by the dependency direction. Removing the
feature deletes `crates/oxyn-embed`, the preference, the scores passed to
`ContextBuilder` and the in-memory cache in `oxyn-desktop`; the lexical
ranking keeps working unchanged, since it never depended on the scores. Users
keep a directory of about 415 MB under `models/` that a removal release
should delete. Changing the engine (to candle, or to a remote endpoint)
replaces `Embedder` behind the same scores; changing the model changes
`pinned`, the generated code and `MODEL_ID`, and every cached vector is
dropped by its key.

**Reconsider if** Burn reaches 1.0 or stops breaking its API between minor
versions (the regeneration cost falls); if a smaller model matches this one's
quality on the measured question/table pairs; if the resident memory measured
in `oxyn-desktop` exceeds **1 GB**; if users have an embedding endpoint
available where they work (the remote path then becomes a complement worth
writing); if candle, measured on the same pairs, matches the output and the
memory without generated code; or if ModernBERT's operators stop converting —
`MongoDB/mdbr-leaf-mt` is then the English-only fallback.

## Rejected alternatives

| Alternative | Reason for rejection |
|---|---|
| An embeddings endpoint (`/v1/embeddings`) through `OpenAiCompatibleProvider`, as a first step | Does not work for the user who asked, who has neither Ollama nor a key, nor for anyone who installed nothing beside Oxyn. It stays a complement for later, not an exclusion: a remote vector would fit behind the same per-relation scores |
| `ort` (onnxruntime) | No stable 2.0 on crates.io (2.0.0-rc.13, 2026-07-28); depends on `ort-sys`, and its default `download-binaries` feature fetches a prebuilt onnxruntime — a C++ binary in the process, downloaded at build time. Burn's output already matches onnxruntime's to 4.2e-7 without it |
| candle (`candle-transformers` 0.11.0, 2026-06-26) | Not measured by the spike. On paper it is the strongest alternative: its `models/modernbert.rs` implements both RoPE bases and the local/global attention from the configuration, so it would need neither generated code nor residual constants. Burn is the engine whose output, memory and build were measured; candle is the first to measure against the same pairs, named in the reconsideration condition |
| Shipping the weights in the installer | 220 MB more in every download and every update, for an option off by default |
| Hosting a converted `.bpk` ourselves | 390 MB in f32 instead of 195 MB in bf16 — twice the download —, a file tied to burn-store's format that changes with every Burn bump, and hosting that is ours alone instead of a fallback |
| `microsoft/harrier-oss-v1-270m` (MIT) | 268 M parameters in bf16, about 1 GB once widened to f32; no ONNX export in its repository (checked 2026-10-07) |
| `voyageai/voyage-4-nano` (Apache-2.0) | 346 M parameters, 3.6 times this model, with no measured gain on the question this ADR settles |
| `jinaai/jina-embeddings-v5-*` | CC-BY-NC-4.0: non-commercial, incompatible with the paid features Oxyn plans |
| `google/embeddinggemma-300m` | The Gemma license, with its own use restrictions, instead of an OSI license `deny.toml` could reason about; 303 M parameters |
| `MongoDB/mdbr-leaf-mt` (Apache-2.0, 22.6 M parameters) | English only: the French question of the context is exactly what it would miss. Kept as the fallback if ModernBERT stops converting |
| A cosine threshold that admits a relation on meaning alone | A gap of about 0.1 between relevant and irrelevant tables makes any threshold a tuned number; ordering degrades gracefully where a threshold fails silently |
| `oxyn-ai` depending on `oxyn-embed` and embedding inside `ContextBuilder::build` | Burn in every `oxyn-ai` build and test, and a gateway that blocks for seconds on a model load. Passing scores keeps the gateway a pure function of what it receives |
| A `build.rs` that runs burn-onnx at compile time | burn-onnx, its protobuf stack and a 390 MB ONNX file in every build, for code that changes only when the model or Burn does |
