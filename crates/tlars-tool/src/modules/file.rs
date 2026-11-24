//! What the file-handling Java modules (`Json`, `_TLCTrace`) share: how a path argument
//! becomes a path on this machine, and the message of a `FileNotFoundException`.

use crate::error::EvalError;
use crate::eval::Ev;
use crate::value::Value;
use std::path::PathBuf;

/// `EISDIR` on Linux.
pub(crate) const EISDIR: i32 = 21;

/// An `IOException` a module raises on purpose. It is a checked exception, not an
/// `EvalException`, so `MethodValue.eval` wraps it in the message naming the Java method
/// (`MethodValue.java:147-169`); any kind that `modules::wrap` wraps will do.
pub(crate) fn io_exception(message: String) -> EvalError {
    EvalError::detailed(message, None)
}

/// How the JDK builds a `FileNotFoundException`'s message: the path as the spec wrote it,
/// then the C library's text for `errno` in parentheses (`io_util_md.c`,
/// `throwFileNotFoundException`). The path is not made absolute, so a relative one is
/// reported relative.
pub(crate) fn io_message(locale: &tlars_java::Locale, path: &str, e: &std::io::Error) -> String {
    format!("{path} ({})", tlars_java::last_error_string(e.raw_os_error().unwrap_or(0), locale))
}

/// The path argument, which `MethodValue.eval` has already cast to `StringValue`
/// (`JavaParam::StringValue`), so anything else failed before the method ran.
pub(crate) fn string_argument(ev: &Ev<'_>, v: &Value) -> String {
    match v {
        Value::Str(a) => ev.tool().graph().interner().text(*a).to_string(),
        other => unreachable!("the path is cast to StringValue first, not {}", other.java_class()),
    }
}

/// `new File(path)` opened by the process: a relative path resolves against the directory
/// the run was started in, not against `user.dir`, because the JDK hands the path to
/// `open(2)` unchanged.
pub(crate) fn resolve(ev: &Ev<'_>, path: &str) -> PathBuf {
    ev.tool().globals().cwd.join(path)
}
