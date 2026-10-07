//! The bundled sqlite-vec extension, registered on every connection the driver
//! opens ([ADR-0054](../../../docs/adr/0054-bundle-sqlite-vec-in-the-sqlite-driver.md)).
//!
//! A database built by a vector tool holds `CREATE VIRTUAL TABLE … USING
//! vec0(…)`: without the module, even reading it fails with `no such module:
//! vec0`. The extension is compiled into the binary, never loaded from a file
//! the user — or the database — names: that would be arbitrary native code.
//!
//! This is the only `unsafe` of the crate. `rusqlite` 0.37 offers no safe way
//! to register a statically linked extension: `Connection::handle` and the call
//! of the C entry point are both `unsafe`. The rest of the crate goes through
//! [`register`], which is safe to call.

use std::ffi::{CStr, c_char, c_int};
use std::ptr;

use rusqlite::{Connection, ffi};

/// The C signature of an extension entry point, `sqlite3_extension_init` in
/// `sqlite3ext.h`.
type ExtensionInit = unsafe extern "C" fn(
    *mut ffi::sqlite3,
    *mut *mut c_char,
    *const ffi::sqlite3_api_routines,
) -> c_int;

/// Registers sqlite-vec's functions (`vec_f32`, `vec_distance_l2`…) and its
/// `vec0` and `vec_each` modules on `connection`.
///
/// Only `sqlite3_vec_init` is called: `sqlite3_vec_numpy_init`, which would add
/// `vec_npy_file` — a function reading any file on disk — stays unregistered.
///
/// # Errors
///
/// The engine's refusal (out of memory, a name already taken), with the message
/// sqlite-vec composes: a function or module name and the engine's message,
/// never a path nor a value — which is why `worker::open` keeps it, as a
/// permanent driver error.
#[allow(unsafe_code)] // ADR-0054: the only FFI of the crate
pub(crate) fn register(connection: &Connection) -> rusqlite::Result<()> {
    // The upstream crate declares `sqlite3_vec_init` as `fn()`, a placeholder
    // signature; the C definition (`sqlite-vec.c`, `SQLITE_VEC_API int
    // sqlite3_vec_init(sqlite3 *db, char **pzErrMsg, const sqlite3_api_routines
    // *pApi)`) is exactly `ExtensionInit`, read on 2026-10-06. The exact pin
    // `sqlite-vec = "=0.1.9"` (workspace `Cargo.toml`) keeps it so until a bump
    // re-reads it.
    // SAFETY: the symbol has `ExtensionInit`'s signature, guarded by that pin.
    let init = unsafe {
        std::mem::transmute::<unsafe extern "C" fn(), ExtensionInit>(sqlite_vec::sqlite3_vec_init)
    };
    let mut message: *mut c_char = ptr::null_mut();
    // - `handle()` is the live `sqlite3*` of `connection`, which the borrow keeps
    //   open for the whole call; the call stays on the thread that owns the
    //   connection (the worker), and the handle is used for nothing else.
    // - sqlite-vec is compiled with `SQLITE_CORE` (its `build.rs`): it calls the
    //   engine `rusqlite`'s `bundled` links, and ignores `pApi`, which can
    //   therefore be null.
    // - `pzErrMsg` must be valid: sqlite-vec writes it on failure without
    //   testing it. It points to `message`, alive until the end of the function.
    // - The modules and functions it registers carry static data only (no
    //   client data, no destructor): nothing outlives the connection.
    // SAFETY: the four conditions above hold for the whole call.
    let code = unsafe { init(connection.handle(), &mut message, ptr::null()) };
    let detail = if message.is_null() {
        None
    } else {
        // A non-null `message` was produced by `sqlite3_mprintf`, hence
        // NUL-terminated and owned by us.
        // SAFETY: it is copied, then released once, by the allocator that made it.
        unsafe {
            let text = CStr::from_ptr(message).to_string_lossy().into_owned();
            ffi::sqlite3_free(message.cast());
            Some(text)
        }
    };
    if code == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(code),
            detail,
        ))
    }
}
