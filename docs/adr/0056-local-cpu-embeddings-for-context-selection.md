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

Those 795 MB were not what the code described. A memory harness run on
2026-10-08 (release, macOS 26.2, Apple M1 Max) found that
`BurnpackStore::from_file` reads the whole `model.bpk` into the heap — 434 MB
of `malloc` for a file the documents called memory-mapped — and that the
system allocator keeps what the conversion freed: about 1.19 GB of empty
`MALLOC_LARGE` regions remain after it, and `malloc_zone_pressure_relief`
returns none of them. With a real mapping (`ea98128`), the loaded model
weighs **157–268 MB of footprint** (157–179 MB loaded, 248–268 MB after
embedding 256 names), against 553–643 MB before; after unloading, about
**240 MB** stay held by the system allocator.

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
context, and a score carries no word into a prompt. It does decide **which**
relations are rendered, and can raise their number — see the ranking rules
below.
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
  `memmap` — the feature does not make `BurnpackStore::from_file` map the
  file: it reads it into the heap, which is why `oxyn-embed` maps it itself
  (below);
- `memmap2` `0.9.11`, the version burn-store already pulls, for that mapping;
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
range. A single `#[allow]` sits on `mod model;` in `generated/mod.rs`, and it
names exactly the three lints the graph trips: `clippy::unwrap_used`,
`clippy::unnecessary_cast` and `clippy::too_many_arguments`. The graph holds
**69** `.unwrap()` — `cargo clippy -p oxyn-embed --lib` reports 69 distinct
sites with the exemption removed (2026-10-07). The 138 written in
`generated/mod.rs` is the `--all-targets` count: `model.rs` is then compiled
twice, as the library and as the library's test harness, and every site is
reported once per build. Never a group such as `clippy::all`, which
would also silence `disallowed_methods` and `disallowed_types` — the walls
the workspace denies on purpose. `weights_map.rs` trips none and has no
exemption; the rest of the crate keeps the workspace's lints,
[I-09](../../CLAUDE.md#i-09) included.

**One `unsafe` block: mapping the verified model.** This ADR lifts the
workspace's refusal of `unsafe` ([SECURITY](../SECURITY.md#unsafe-policy))
for one function, `map_verified` in `crates/oxyn-embed/src/load.rs`, under a
`#[allow(unsafe_code)]` on that function alone. It calls
`memmap2::Mmap::map` on the `File` that `hash::open_verified` has just hashed
and accepted — one open handle, so the bytes mapped are the bytes of the
inode that was verified, not of whatever the path names a moment later. The
mapping is read-only and handed to burn-store as shared, file-backed bytes
(`Bytes::from_shared`, `AllocationProperty::File`).

* **What it buys.** The weights stay file pages the system can share, evict
  and reload, outside the process's footprint: 157–268 MB loaded instead of
  553–643 MB, measured on 2026-10-08. Without it, burn-store copies the
  389,816,832-byte file into the heap.
* **Why it is unsafe.** A mapping is sound only while nobody changes the file
  under it, and no type can promise that. Oxyn never writes `model.bpk` in
  place: the conversion writes a `.part` and renames it over,
  `ModelStore::remove` unlinks, and both leave an existing mapping on the old
  inode intact. Who keeps it true: every writer of `model.bpk` in
  `oxyn-embed` (`download.rs`) — a rename, never an in-place write.
* **The risk accepted.** A process of the same user that truncates the file
  in place while it is mapped makes the next read of a missing page kill
  Oxyn with `SIGBUS`; one that rewrites it in place gives wrong vectors. Both
  need write access to the data directory, which already holds everything
  Oxyn keeps.
* **The review.** The block carries its `// SAFETY:` naming that property and
  who keeps it, and goes through the `relecteur-securite` review the
  `unsafe` policy requires.

### Off by default, turned on by the human

The option is the workspace preference `semantic_ranking`, a boolean of
`WorkspacePreferences` ([ADR-0013](0013-preferences-workspace.md)), **off by
default**. It is added under ADR-0013's rule — a new field without a version
change —, so a payload written before it reads as off and never starts a
download. The default policy reserves its write to `Actor::Human`: no agent
turns it on, so no agent starts a download. Its screen is in
[UX-SPEC](../UX-SPEC.md#semantic-ranking). While it is off, nothing of `oxyn-embed` touches the
network, the disk or memory, and selection is exactly today's.

**The download is started by a command, not by the preference.** The
settings send `enable_semantic_ranking`, which saves the preference and starts
`ModelStore::download` in the background, with a progress report and
cancellation. A `true` read from disk at startup starts nothing: it only
decides whether a model already present is used. `disable_semantic_ranking`
saves it off, cancels a download, unloads the model and deletes its
directory (`ModelStore::remove`).

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
user agent. The system proxy is honoured, unlike the AI transports, which
disable it: what they send is the user's data, whereas this request sends
none, and the bytes it receives are accepted on their checksum alone —
whoever relays them can withhold them, not alter them. The workspace
`reqwest` enables its `system-proxy` feature for that: without it, reqwest
only reads the `HTTP(S)_PROXY` variables, which an application started from
the Finder does not inherit. The feature applies to every client of the
graph, so a client that must not follow a proxy says so: `oxyn-llm`'s only
client keeps calling `no_proxy()`. The updater (`tauri-plugin-updater`, same
`reqwest`) now follows the system proxy too; what it installs is still
accepted on its minisign signature, not on its route
([ADR-0051](0051-automatic-updates-from-github-releases.md)).

**One download at a time, across processes.** The download holds an exclusive
lock on the model directory's `.lock` file (`File::try_lock`); a second Oxyn
process asking for it is refused with `EmbedError::DownloadInProgress`, since
waiting is the fix — the files it writes are the same files. The
safetensors is hashed again right before the conversion parses it, and
removed when damaged. `tokenizer.json` is read once: the bytes hashed are the
bytes parsed.

**Errors carry fixed texts.** burn-store's messages name a file's full path,
and a tokenizer error can quote the text: the conversion, loading and
tokenizer errors of `oxyn-embed` replace them with fixed sentences, and the
message `oxyn-desktop` shows in the settings and writes to the journal is a
fixed sentence per `EmbedError` variant, filled only with a pinned file name,
an action or an I/O error kind. Tests feed every variant a path and a marker
and check that neither comes out.

The release `embedding-model-835ad140` is published as a **pre-release**:
GitHub's "latest release" is the most recent non-prerelease, non-draft one,
and the updater reads `releases/latest/download/latest.json`
([ADR-0051](0051-automatic-updates-from-github-releases.md)). Published as an
ordinary release, it would become "latest" and stop every update. It was
published on 2026-10-07 and carries, beside the two files,
`LICENSE-Apache-2.0.txt` and `NOTICE.md` (IBM's attribution, the revision and
the checksums); "latest" stayed `v0.0.7`.

The files live under `<data dir>/models/granite-embedding-97m-multilingual-r2-835ad140/`,
`models/` sitting next to the local store's file. The temporary workspace of
`make desktop-dev` uses `models-temporary-workspace/` in the same user data
directory instead: sharing `models/` let turning the option off in a
development session delete the installed Oxyn's model. It is not the system
temporary directory, which other accounts can write on Linux, and it
persists across development launches.
After the download, the bf16 safetensors is widened to f32 and written as
`model.bpk` (389,816,832 bytes), which loads by memory mapping since
`ea98128` — before it, burn-store read it into the heap; the safetensors is
deleted only once the converted file is verified. What stays on disk is
`model.bpk` and `tokenizer.json`, about 415 MB. Download and conversion
together take 14 s; the conversion peaks at **1.45 GB** of RSS, and the
system allocator then keeps about 1.19 GB of it in the process that ran it.

**The conversion runs in a child process.** `oxyn-embed` exposes it as a
self-contained step — `ModelStore::convert_in_place`, and
`ModelStore::download_with_converter` taking it as a closure, with the
directory lock held across it and the converted file verified after it
whatever the step reports — and a child reports a failure as an exit code
(`EmbedError::exit_code`, one stable code per variant, 70 to 78), never as
text. `oxyn-desktop` runs it in a child process
(`crates/oxyn-desktop/src/embedding_converter.rs`), so the 1.45 GB peak and
what the allocator keeps afterwards end with that process instead of staying
in Oxyn's:

* **The child is Oxyn itself**, relaunched from `std::env::current_exe` with
  the internal argument `--convert-embedding-model <models directory>`: the
  same binary in `make desktop-dev` and once installed, on every platform, and
  no second executable to ship. `main` recognises the argument first —
  before the journal, the store, Tauri or any window —, calls
  `ModelStore::convert_in_place` and exits with its code.
* **It converts only where Oxyn would.** The root must be exactly one of the
  two directories Oxyn computes from its data directory, `models/` or
  `models-temporary-workspace/`, and the only argument after the flag;
  anything else is refused before the disk is touched, with the code of a
  conversion failure (76). Any process can launch Oxyn with arguments: this
  one is not a way to convert or write elsewhere.
* **It inherits almost nothing.** The environment is cleared except `HOME`
  and `XDG_DATA_HOME` — so the child computes the same data directory — and
  `SystemRoot`, which a Windows process needs to load system libraries;
  standard input, output and error are null; the working directory is the
  models directory; no console window on Windows. The single spawn site is
  exempted from `clippy.toml`'s ban on `std::process::Command::new` because
  of the `env_clear` right after it, like `oxyn-ai`'s `external/spawn.rs`.
* **The parent stays in charge.** It holds the model directory's lock for the
  whole download; the child does not take it. It awaits the child through
  Tokio without blocking a thread; cancelling kills and reaps it
  (`kill_on_drop` as well), and the `.part` it leaves is removed like an
  interrupted download's. An exit code goes through
  `EmbedError::from_exit_code`; a signal reads as a conversion failure; both
  reach the settings as fixed sentences.
* **Measured** by hand on 2026-10-08, Apple Silicon, dev profile: the child
  converted the pinned weights in 3.7 s, exit 0, with a maximum resident set
  of 1.45 GB in the child; a directory outside the two was refused with
  exit 76, at 9 MB, creating nothing. The parent's memory after a conversion
  has not been measured yet: that it keeps nothing of it follows from the
  conversion running in another process.

The checksum of `model.bpk` is a constant too —
`d5ac67b8e7e85e63ba433faebbe3e5537732ab7e27dde9cf3422710c6abce719` —, so a
file damaged later is refused like a downloaded one. That takes one step
around burn-store: `BurnpackStore` records each parameter's `ParamId`, a
random `u64` drawn when the model is built, so two conversions of the same
weights would differ. The conversion writes through `burn_pack::Writer` with
every `param_id` set to `None`; the ids only serve to resume training, and the
widening from bf16 is exact, so the file is a function of the weights alone.
No file is parsed before its size and checksum are checked.

**An accepted risk: `model.bpk` is mapped.** The hash and the mapping use one
open handle, so replacing the file between the two changes nothing: the
verified inode is the one mapped. What remains is an in-place change of that
inode while it is mapped — a truncation kills Oxyn with `SIGBUS` at the next
read of a missing page, a rewrite gives wrong vectors — by a process of the
same user with write access to the data directory, which already holds
everything Oxyn keeps. Reading the 390 MB into the heap would close it, at
the cost the mapping exists to avoid; the risk is written in the `// SAFETY:`
of `map_verified`.

### Memory budget

| State | Budget | Measured (2026-10-08, footprint) |
|---|---|---|
| Option never turned on | **0** — nothing loaded, nothing mapped | — |
| Model loaded | **≤ 300 MB** of footprint | 157–179 MB loaded, 248–268 MB after embedding 256 names |
| After unloading | **≤ 250 MB** kept by the system allocator, not by Oxyn's code | about 240 MB |
| Conversion | **in a child process**: Oxyn's own process keeps none of it, by construction | 1.45 GB peak in the child, 3.7 s (dev profile); about 1.19 GB kept afterwards by a process that converts, released when the child exits. The parent's memory not yet measured |

The memory kept after unloading is the system allocator's, not a leak: macOS
`malloc` keeps freed large regions as `MALLOC_LARGE (empty)` and gives them
back neither on unload nor on `malloc_zone_pressure_relief`. Three ways to
release it were measured on 2026-10-08 and set aside:

* `MallocLargeCache=0` returns those regions, but inference becomes 31 %
  slower — 2.51 s on the measured workload, past the 2-second bound;
* `malloc_zone_pressure_relief` after unloading returns nothing measurable;
* `mimalloc` as the global allocator keeps about 70 MB after unloading, but it
  replaces the allocator of every Rust allocation in the application —
  drivers, Arrow buffers, the catalog — which takes its own ADR and its own
  benchmarks, not a line in this one.

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

A comment is cut at 2,048 bytes before it is embedded: the model reads 512
tokens at most. Relation vectors are kept in memory only, in `oxyn-desktop`,
keyed by a **SHA-256 of `MODEL_ID` and the embedded text**: the cache keeps no
table name nor comment, the text lives for one question only, and the bound
does not depend on the comments' length. They are never written to disk,
never logged, never sent. At most 50,000 vectors — the catalog cache's
ceiling — in two generations: 50,000 × 1,536 bytes plus 2 × 32,768 buckets of
41 bytes, about **79.5 MB**.

The ranking, in `oxyn-ai`:

1. `@` mentions first, as today;
2. relations with a lexical score, by that score; **equal scores by cosine**,
   then by path;
3. relations without a lexical score, **by cosine**, then by path, until
   `ContextPolicy::max_relations`.

**Scores can widen the selection, not only reorder it.** Without scores, a
question that matches some names keeps those matches and nothing else (the
order of the paths decides only when nothing matches). With scores, rule 3
completes the matches with the relations the search missed, up to
`max_relations` — 24. More of the schema is then described and sent, always
under the connection's tier and within the token budget; the settings say so
([UX-SPEC](../UX-SPEC.md#semantic-ranking)). A test of `oxyn-ai` fixes both
counts and that the catalog fill wants the same selection.

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

**Whether it ran is traced, in counts only.** At `OXYN_LOG=oxyn=debug`, a
question's semantic step writes `semantic ranking done` with the relations
listed, scored, served from the cache and embedded for the question, those
left unscored by the deadline, the elapsed milliseconds and whether the
question paid the model's cold load. A step that does not run writes
`semantic ranking skipped; lexical ranking only` with a fixed `reason` — `option
off`, `no models directory`, `model not ready`, `empty question`, `model
absent`, `model damaged`, `model directory unreadable`, `catalog empty`, `no
time left in the catalog fill` —, and a failed one `semantic ranking failed;
lexical ranking only` with the fixed sentence of its `EmbedError` variant.
The reason is a `&'static str` chosen in the code and every other field a
number or a boolean, so nothing of the question or the schema reaches the
journal ([I-03](../../CLAUDE.md#i-03)); a test captures a skipped step's
journal with markers in the question and a relation and checks they are
absent.

## Consequences

* **+** A question in one language finds tables named in another, and a
  question that names no table gets the closest ones instead of the
  alphabetically first: the 24 relations kept stop being arbitrary.
* **+** Nothing leaves the machine to compute it: no provider, no key, no
  installation. The privacy tier is untouched under every value.
* **+** Lexical matches keep their order and their place: a wrong vector can
  demote nothing that matched by name.
* **−** **More schema can leave.** Where the lexical selection kept only its
  matches, relations found by meaning complete it up to 24: a provider can
  read the description of tables the question never named. The tier and the
  token budget still bound what leaves; the number of relations described is
  what grows.
* **+** `oxyn-ai` does not depend on Burn: its tests and the external-agent
  path keep their compile time, and the gateway stays one function.
* **−** **157–268 MB of footprint while the model is loaded**, five minutes
  after the last question at most, plus the mapped file's pages the system
  keeps while it has room for them.
* **−** **About 240 MB stay held after unloading** — the system allocator's,
  which Oxyn cannot give back without slowing inference past the 2-second
  bound or replacing the application's allocator. Once the option has been
  used, an idle Oxyn is that much larger until it quits.
* **−** **One `unsafe` block**, and with it a crash Oxyn cannot catch: a
  `SIGBUS` if another process of the same user truncates `model.bpk` in place
  while it is mapped.
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
  network, the option cannot be turned on. The fallback is ours to keep: a
  deleted release is a 404 that the download error names.
* **−** **Without its features, burn-flex runs on one core, without SIMD**,
  several times slower — and nothing fails. Only the direct declaration in the
  root `Cargo.toml`, commented, keeps them.
* **−** **+80 s of cold compilation** in CI and for any build that compiles
  `oxyn-desktop` from scratch; 45 s of it on the critical path.
* **−** About 415 MB of disk for a user who turns the option on, a download
  of 220 MB at first use, and **1.45 GB of RSS at the peak** of the
  conversion, once — in a child process, which ends with what the allocator
  kept.
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
ranking keeps working unchanged, since it never depended on the scores, and
the workspace's `unsafe` exceptions shrink back to the one of
[ADR-0054](0054-bundle-sqlite-vec-in-the-sqlite-driver.md). Users
keep a directory of about 415 MB under `models/` that a removal release
should delete. Changing the engine (to candle, or to a remote endpoint)
replaces `Embedder` behind the same scores; changing the model changes
`pinned`, the generated code and `MODEL_ID`, and every cached vector is
dropped by its key.

**Reconsider if** Burn reaches 1.0 or stops breaking its API between minor
versions (the regeneration cost falls); if a smaller model matches this one's
quality on the measured question/table pairs; if the footprint measured in
`oxyn-desktop` exceeds the budget above — **300 MB** loaded, **250 MB** kept
after unloading; if the memory kept after unloading becomes a complaint, or
an allocator change is decided for another reason — `mimalloc`, measured at
about 70 MB kept, is then the candidate, with its own ADR and benchmarks; if
users have an embedding endpoint
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
| Loading `model.bpk` with `BurnpackStore::from_file`, without `unsafe` | Reads the whole file into the heap: 553–643 MB of footprint loaded instead of 157–268 MB (2026-10-08). The `unsafe` block costs one function and an accepted `SIGBUS` on a file only the user's own processes can write |
| Converting in Oxyn's own process | The conversion peaks at 1.45 GB and the system allocator keeps about 1.19 GB of it afterwards, for the rest of the session; a child process gives it all back when it exits |
| `MallocLargeCache=0` | Returns the regions the allocator keeps, but inference becomes 31 % slower — 2.51 s on the measured workload, past the 2-second bound (2026-10-08) |
| `malloc_zone_pressure_relief` after unloading | Returned nothing measurable of the ~240 MB kept (2026-10-08) |
| `mimalloc` as the global allocator, in this change | About 70 MB kept after unloading instead of ~240 MB (2026-10-08), but it changes the allocator of every Rust allocation in the application: a decision for its own ADR and benchmarks, named in the reconsideration condition |
