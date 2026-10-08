//! Checking a file on disk against its pinned size and checksum, before
//! anything parses it.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::EmbedError;
use crate::pinned::PinnedFile;

/// Read buffer: large enough that the hash, not the system calls, dominates.
const CHUNK: usize = 1 << 20;

/// What a file on disk turned out to be. `Valid` carries what the reader
/// kept: nothing for [`verify`], the bytes for [`read_verified`].
#[derive(Debug)]
pub(crate) enum Verdict<T = ()> {
    Missing,
    Valid(T),
    /// Present, but not the pinned file; the text says how.
    Invalid(String),
}

/// Lowercase hexadecimal of a digest.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Size only: the cheap check behind [`ModelStatus`](crate::ModelStatus).
pub(crate) fn size_matches(path: &Path, file: &PinnedFile) -> Result<Option<bool>, EmbedError> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.is_file() && meta.len() == file.size)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(EmbedError::io("inspect", path, err)),
    }
}

/// Size, then SHA-256 of the whole content.
///
/// **Blocks**: reads the whole file — about 0.2 s for the 390 MB of the
/// converted model on a recent laptop. The read stops at the pinned size plus
/// one byte, so a file that grew while being read is refused without being
/// read to its end.
pub(crate) fn verify(path: &Path, file: &PinnedFile) -> Result<Verdict, EmbedError> {
    scan(path, file, |_| {})
}

/// [`verify`], keeping the bytes it hashed: what the caller parses next is
/// exactly what was checked, even if the file is replaced in the meantime.
///
/// **Blocks**, and holds the whole file in memory: for the 25 MB tokenizer,
/// not for the model.
pub(crate) fn read_verified(
    path: &Path,
    file: &PinnedFile,
) -> Result<Verdict<Vec<u8>>, EmbedError> {
    let mut bytes = Vec::with_capacity(usize::try_from(file.size).unwrap_or(0));
    Ok(
        match scan(path, file, |chunk| bytes.extend_from_slice(chunk))? {
            Verdict::Valid(()) => Verdict::Valid(bytes),
            Verdict::Missing => Verdict::Missing,
            Verdict::Invalid(detail) => Verdict::Invalid(detail),
        },
    )
}

/// Opens `path` and verifies it, returning the open file when it is the
/// pinned one.
///
/// For a file that is used through its handle afterwards — the model,
/// memory-mapped: the bytes mapped are those of the very file that was
/// hashed, not of whatever sits at the path a moment later.
///
/// **Blocks**, like [`verify`].
pub(crate) fn open_verified(path: &Path, file: &PinnedFile) -> Result<Verdict<File>, EmbedError> {
    let handle = match File::open(path) {
        Ok(handle) => handle,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Verdict::Missing),
        Err(err) => return Err(EmbedError::io("open", path, err)),
    };
    Ok(match scan_handle(&handle, path, file, |_| {})? {
        Verdict::Valid(()) => Verdict::Valid(handle),
        Verdict::Missing => Verdict::Missing,
        Verdict::Invalid(detail) => Verdict::Invalid(detail),
    })
}

/// Reads `path` once, handing every chunk to `keep` as it is hashed.
fn scan(path: &Path, file: &PinnedFile, keep: impl FnMut(&[u8])) -> Result<Verdict, EmbedError> {
    let handle = match File::open(path) {
        Ok(handle) => handle,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Verdict::Missing),
        Err(err) => return Err(EmbedError::io("open", path, err)),
    };
    scan_handle(&handle, path, file, keep)
}

/// Size — of the open file, not of the path — then content, from the start.
fn scan_handle(
    handle: &File,
    path: &Path,
    file: &PinnedFile,
    mut keep: impl FnMut(&[u8]),
) -> Result<Verdict, EmbedError> {
    let meta = handle
        .metadata()
        .map_err(|err| EmbedError::io("inspect", path, err))?;
    if !meta.is_file() || meta.len() != file.size {
        return Ok(Verdict::Invalid(format!(
            "not a file of the expected {} bytes",
            file.size
        )));
    }
    let mut reader = handle.take(file.size.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| EmbedError::io("read", path, err))?;
        if read == 0 {
            break;
        }
        let chunk = buffer.get(..read).unwrap_or(&[]);
        hasher.update(chunk);
        keep(chunk);
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    if total != file.size {
        return Ok(Verdict::Invalid(format!(
            "{total} bytes read, {} expected",
            file.size
        )));
    }
    let actual = hex(&hasher.finalize());
    if actual == file.sha256 {
        Ok(Verdict::Valid(()))
    } else {
        Ok(Verdict::Invalid(format!(
            "sha256 {actual}, expected {}",
            file.sha256
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_and_two_digits_per_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    fn pinned(content: &[u8]) -> PinnedFile {
        let digest = Sha256::digest(content);
        PinnedFile {
            name: "f",
            size: u64::try_from(content.len()).unwrap_or(u64::MAX),
            sha256: Box::leak(hex(&digest).into_boxed_str()),
        }
    }

    #[test]
    fn a_file_is_valid_only_with_its_size_and_its_checksum() -> Result<(), EmbedError> {
        let dir = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
        let path = dir.path().join("f");
        let file = pinned(b"pinned content");

        assert!(matches!(verify(&path, &file)?, Verdict::Missing));

        std::fs::write(&path, b"pinned content").map_err(|e| EmbedError::io("write", &path, e))?;
        assert!(matches!(verify(&path, &file)?, Verdict::Valid(())));
        // The bytes kept are the bytes hashed.
        assert!(
            matches!(read_verified(&path, &file)?, Verdict::Valid(b) if b == b"pinned content")
        );

        // Same size, one byte flipped: only the checksum can tell.
        std::fs::write(&path, b"pinned contenT").map_err(|e| EmbedError::io("write", &path, e))?;
        assert!(matches!(verify(&path, &file)?, Verdict::Invalid(d) if d.contains("sha256")));

        // Truncated, as by a full disk.
        std::fs::write(&path, b"pinned").map_err(|e| EmbedError::io("write", &path, e))?;
        assert!(matches!(verify(&path, &file)?, Verdict::Invalid(_)));
        Ok(())
    }

    #[test]
    fn a_directory_in_place_of_the_file_is_invalid() -> Result<(), EmbedError> {
        let dir = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
        let file = pinned(b"x");
        assert!(matches!(verify(dir.path(), &file)?, Verdict::Invalid(_)));
        Ok(())
    }
}
