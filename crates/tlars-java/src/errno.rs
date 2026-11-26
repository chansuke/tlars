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
//!
//! Which locale is not read from this process's environment here: the caller hands in a
//! [`Locale`], so a run whose environment is fixed (the conformance harness) gets the same
//! text on every machine.
//!
//! The category masks below are glibc's, so the C library path is compiled for glibc
//! Linux only. Elsewhere the message is the C locale's text, the one `strerror` gives
//! before anything calls `setlocale`, taken from the standard library.

/// The locale names `setlocale(LC_ALL, "")` would pick for the two categories that decide
/// a `strerror` text: `LC_MESSAGES` chooses the catalog and `LC_CTYPE` its charset.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Locale {
    pub ctype: String,
    pub messages: String,
}

impl Locale {
    /// POSIX's lookup for each category (XBD 8.2): `LC_ALL`, then the category's own
    /// variable, then `LANG`, the first that is set and not empty; `C` when none is.
    pub fn from_environment(var: impl Fn(&str) -> Option<String>) -> Locale {
        let pick = |category: &str| {
            ["LC_ALL", category, "LANG"].iter().find_map(|name| var(name).filter(|v| !v.is_empty())).unwrap_or_else(|| "C".to_string())
        };
        Locale { ctype: pick("LC_CTYPE"), messages: pick("LC_MESSAGES") }
    }

    /// The locale the conformance recordings are made under, so a hermetic run matches them.
    pub fn c_utf8() -> Locale {
        Locale { ctype: "C.UTF-8".into(), messages: "C.UTF-8".into() }
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub use linux::last_error_string;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod linux {
    use super::Locale;
    use std::collections::BTreeMap;
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::sync::Mutex;

    /// `LC_CTYPE_MASK` and `LC_MESSAGES_MASK` (`bits/locale.h`: `__LC_CTYPE` 0,
    /// `__LC_MESSAGES` 5). Without `LC_CTYPE`, glibc transcodes the catalog's UTF-8 to the
    /// C locale's ASCII and every character comes out as `?`.
    const LC_CTYPE_MASK: c_int = 1 << 0;
    const LC_MESSAGES_MASK: c_int = 1 << 5;

    /// `LC_GLOBAL_LOCALE`, the process-wide locale, which is `C` here because nothing in
    /// this program calls `setlocale`.
    const LC_GLOBAL_LOCALE: *mut c_void = usize::MAX as *mut c_void;

    unsafe extern "C" {
        fn newlocale(category_mask: c_int, locale: *const c_char, base: *mut c_void) -> *mut c_void;
        fn strerror_l(errnum: c_int, locale: *mut c_void) -> *mut c_char;
    }

    /// One `locale_t` per distinct [`Locale`], built on first use and never freed: a run
    /// asks for one locale, so the map holds as many entries as runs with different
    /// environments in this process. Held as `usize` because `strerror_l` only reads it.
    fn handle(locale: &Locale) -> *mut c_void {
        static HANDLES: Mutex<BTreeMap<Locale, usize>> = Mutex::new(BTreeMap::new());
        let mut handles = HANDLES.lock().unwrap_or_else(|p| p.into_inner());
        let handle = *handles.entry(locale.clone()).or_insert_with(|| build(locale));
        // A locale the host does not have leaves the process locale, so the message is the
        // C one rather than nothing.
        if handle == 0 { LC_GLOBAL_LOCALE } else { handle as *mut c_void }
    }

    fn build(locale: &Locale) -> usize {
        let (Ok(ctype), Ok(messages)) = (CString::new(locale.ctype.as_str()), CString::new(locale.messages.as_str())) else {
            return 0;
        };
        // SAFETY: both names are nul-terminated. The first call gets a null base, which asks
        // for a fresh locale; the second modifies that one, which `newlocale` owns from then
        // on (or frees, returning null, on failure).
        unsafe {
            let base = newlocale(LC_CTYPE_MASK, ctype.as_ptr(), std::ptr::null_mut());
            if base.is_null() {
                return 0;
            }
            newlocale(LC_MESSAGES_MASK, messages.as_ptr(), base) as usize
        }
    }

    /// `getLastErrorString()`: `strerror(errno)` under `locale`.
    pub fn last_error_string(errno: i32, locale: &Locale) -> String {
        let locale = handle(locale);
        // SAFETY: `strerror_l` returns a nul-terminated string that stays valid as long as
        // the locale does, and this locale is never freed. The copy happens before anything
        // else can run on this thread, and `strerror_l` needs no lock of its own.
        unsafe { CStr::from_ptr(strerror_l(errno, locale)) }.to_string_lossy().into_owned()
    }
}

/// `getLastErrorString()` where tlars does not ask the C library.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn last_error_string(errno: i32, _locale: &Locale) -> String {
    c_locale_message(errno)
}

/// The C locale's text, which is what `std::io::Error` prints before its ` (os error N)`
/// suffix. Compiled everywhere so the fallback is built and tested on the host CI runs on.
#[cfg_attr(all(target_os = "linux", target_env = "gnu"), allow(dead_code))]
fn c_locale_message(errno: i32) -> String {
    let text = std::io::Error::from_raw_os_error(errno).to_string();
    let suffix = format!(" (os error {errno})");
    text.strip_suffix(&suffix).map(str::to_string).unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::Locale;

    #[test]
    fn c_locale_message_drops_the_os_error_suffix() {
        let text = super::c_locale_message(2);
        assert!(!text.contains("os error"), "{text}");
        #[cfg(target_os = "linux")]
        assert_eq!(text, "No such file or directory");
    }

    #[test]
    fn locale_follows_posix_precedence() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(Locale::from_environment(env(&[])), Locale { ctype: "C".into(), messages: "C".into() });
        let lang = Locale::from_environment(env(&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "ja_JP.UTF-8")]));
        assert_eq!(lang, Locale { ctype: "en_US.UTF-8".into(), messages: "ja_JP.UTF-8".into() });
        let all = Locale::from_environment(env(&[("LC_ALL", "C.UTF-8"), ("LC_MESSAGES", "ja_JP.UTF-8"), ("LANG", "")]));
        assert_eq!(all, Locale::c_utf8());
        let empty = Locale::from_environment(env(&[("LC_ALL", ""), ("LANG", "de_DE.UTF-8")]));
        assert_eq!(empty.messages, "de_DE.UTF-8");
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn c_utf8_is_the_english_text() {
        assert_eq!(super::last_error_string(2, &Locale::c_utf8()), "No such file or directory");
    }
}
