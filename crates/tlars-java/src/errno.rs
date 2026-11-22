//! The text the JDK puts in a `FileNotFoundException`, which is the C library's, not
//! Java's.
//!
//! When a file cannot be opened, the JDK throws `new FileNotFoundException(path + " (" +
//! getLastErrorString() + ")")`, and `getLastErrorString` (`jni_util.c`) is `strerror` of
//! `errno`. The JVM calls `setlocale(LC_ALL, "")` at startup, so that text is in the
//! environment's locale: under `LC_ALL=ja_JP.UTF-8` a missing file reads
//! `x.json (そのようなファイルやディレクトリはありません)`. TLC prints the message
//! unchanged inside TLC_MODULE_VALUE_JAVA_METHOD_OVERRIDE, so matching TLC byte for byte
//! means asking the same C library the same question.
//!
//! `std::io::Error`'s own `Display` cannot answer it: it formats under the C locale and
//! appends `(os error N)`. So this is the one place the workspace leaves Rust's standard
//! library, through two POSIX calls. There is no unit test for the text, because the right
//! answer is whatever the host's locale says; the check that matters is the forty-odd
//! `tlc/standard/json` corpus cases, which compare these bytes with TLC's.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::OnceLock;

/// `LC_CTYPE_MASK | LC_MESSAGES_MASK` (`bits/locale.h`: `__LC_CTYPE` 0, `__LC_MESSAGES` 5).
/// `LC_MESSAGES` picks the catalog and `LC_CTYPE` its charset: without the latter, glibc
/// transcodes the catalog's UTF-8 to the C locale's ASCII and every character comes out
/// as `?`.
const LC_CTYPE_MESSAGES_MASK: c_int = (1 << 0) | (1 << 5);

/// `LC_GLOBAL_LOCALE`, the process-wide locale, which is `C` here because nothing in this
/// program calls `setlocale`.
const LC_GLOBAL_LOCALE: *mut c_void = usize::MAX as *mut c_void;

unsafe extern "C" {
    fn newlocale(category_mask: c_int, locale: *const c_char, base: *mut c_void) -> *mut c_void;
    fn strerror_l(errnum: c_int, locale: *mut c_void) -> *mut c_char;
}

/// The locale `setlocale(LC_ALL, "")` would select, built once. Held as a `usize` because a
/// `locale_t` is a pointer that `strerror_l` only reads. It is never freed: one locale per
/// process, alive as long as the process is.
fn environment_locale() -> *mut c_void {
    static LOCALE: OnceLock<usize> = OnceLock::new();
    let handle = *LOCALE.get_or_init(|| {
        // SAFETY: the name is a nul-terminated literal and the base is null, which asks
        // `newlocale` for a fresh locale rather than one to modify.
        let locale = unsafe { newlocale(LC_CTYPE_MESSAGES_MASK, c"".as_ptr(), std::ptr::null_mut()) };
        locale as usize
    });
    // A locale the host does not have leaves the process locale, so the message is the C
    // one rather than nothing.
    if handle == 0 { LC_GLOBAL_LOCALE } else { handle as *mut c_void }
}

/// `getLastErrorString()`: `strerror(errno)` under the environment's locale.
pub fn last_error_string(errno: i32) -> String {
    let locale = environment_locale();
    // SAFETY: `strerror_l` returns a nul-terminated string that stays valid as long as the
    // locale does, and this locale outlives the process. The copy happens before anything
    // else can run on this thread, and `strerror_l` needs no lock of its own.
    unsafe { CStr::from_ptr(strerror_l(errno, locale)) }.to_string_lossy().into_owned()
}
