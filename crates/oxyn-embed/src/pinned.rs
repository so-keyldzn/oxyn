//! What Oxyn downloads, and what it accepts.
//!
//! Every value here was read from the published files on 2026-10-07
//! (RESEARCH-NOTES, ADR-0056). Changing one of them changes the model: the
//! generated code of `generated/`, the converted file's checksum and every
//! vector a caller has cached must move with it — see `codegen/regenerate.py`.

/// The Hugging Face repository of the model.
pub const MODEL_REPOSITORY: &str = "ibm-granite/granite-embedding-97m-multilingual-r2";

/// The revision every file is fetched at. A commit, not a branch: `main` can
/// be rewritten upstream, a commit cannot.
pub const MODEL_REVISION: &str = "835ad14087e140460703cf0fae09f97d469d65c2";

/// Identifies the vectors this crate produces.
///
/// A caller that keeps embeddings — of tables, of columns — stores this value
/// next to them and discards them when it changes: two models' vectors are
/// not comparable, and a cosine between them is noise that looks like a score.
pub const MODEL_ID: &str =
    "ibm-granite/granite-embedding-97m-multilingual-r2@835ad14087e140460703cf0fae09f97d469d65c2";

/// Length of an [`Embedding`](crate::Embedding).
pub const DIMENSIONS: usize = 384;

/// Tokens kept per text, `[CLS]` and `[SEP]` included; the rest is cut.
///
/// The model accepts more, but a catalog object's name and comment fit well
/// within 512 tokens, and attention costs grow with the square of the length.
pub const MAX_TOKENS: usize = 512;

/// Rows of the token embedding matrix (`vocab_size` of the pinned
/// `config.json`). A token id at or beyond it would make the generated graph's
/// lookup panic, so the embedder refuses it first.
pub(crate) const VOCABULARY: u32 = 180_000;

/// The padding token of the pinned `tokenizer.json` (`<|endoftext|>`). Its
/// value hardly matters — padded positions are masked — but it is the one
/// the model was trained with.
pub(crate) const PAD_ID: u32 = 179_935;

/// A file whose size and checksum are known in advance.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PinnedFile {
    /// Name in the upstream repository, and in the data directory.
    pub(crate) name: &'static str,
    /// Exact size in bytes: a larger download is cut off as soon as it
    /// exceeds it, a smaller one is refused.
    pub(crate) size: u64,
    /// SHA-256, lowercase hexadecimal.
    pub(crate) sha256: &'static str,
}

/// The weights, as published: bf16, 97 M parameters.
pub(crate) const SAFETENSORS: PinnedFile = PinnedFile {
    name: "model.safetensors",
    size: 194_889_568,
    sha256: "f3ea88b230492811046145513710e76b4cc8c2ad49e8708da0e7247e548903be",
};

/// The tokenizer, as published.
pub(crate) const TOKENIZER: PinnedFile = PinnedFile {
    name: "tokenizer.json",
    size: 25_301_672,
    sha256: "4f2842d568e2724370aec203652a42ac783c7937f8347a1a2cc7506d71f1582f",
};

/// The converted model: f32 weights and the residual constants, in burn-store's
/// format, written once after the download.
///
/// It is produced locally, yet its checksum is a constant: the conversion is
/// a bf16 → f32 widening, exact by construction, and the writer is given no
/// date nor random identifier (`load::convert`). Checking it against a
/// constant rather than against a value recorded at conversion time means a
/// file damaged later is caught the same way as a downloaded one, and a
/// conversion that went wrong is caught before it is ever loaded.
pub(crate) const CONVERTED: PinnedFile = PinnedFile {
    name: "model.bpk",
    size: 389_816_832,
    sha256: "d5ac67b8e7e85e63ba433faebbe3e5537732ab7e27dde9cf3422710c6abce719",
};

/// The release of this repository that carries a copy of the two upstream
/// files, byte for byte, under the same names and checksums.
///
/// It is the fallback when Hugging Face is unreachable or answers something
/// that fails verification: a mirror changes where the bytes come from, never
/// what is accepted. Published on 2026-10-07 as a **pre-release**, so that
/// it never becomes the "latest" release the updater reads (ADR-0051); its
/// two assets were checked that day to carry the pinned SHA-256 values.
pub const FALLBACK_RELEASE: &str =
    "https://github.com/so-keyldzn/oxyn/releases/download/embedding-model-835ad140";

/// The sources of a file, in the order they are tried.
pub(crate) fn sources(file: &PinnedFile) -> [String; 2] {
    [
        format!(
            "https://huggingface.co/{MODEL_REPOSITORY}/resolve/{MODEL_REVISION}/{}",
            file.name
        ),
        format!("{FALLBACK_RELEASE}/{}", file.name),
    ]
}
