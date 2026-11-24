//! `tlc2.module._TLCTrace`: `_TLCTraceSerialize` and `_TLCTraceDeserialize`, which write and
//! read a counterexample as a gzipped [`crate::value::stream`] file.
//!
//! The module's TLA+ bodies are decoys (`_TLCTrace.tla` defines both as `TRUE`), so every
//! byte of the file comes from the Java class.

use crate::error::{EvalError, EvalResult};
use crate::eval::Ev;
use crate::fp64::Fp;
use crate::modules::file::{EISDIR, io_exception, io_message, resolve, string_argument};
use crate::value::{Value, empty_rcd, stream};
use std::io::Write as _;

/// `ioSerialize(value, absolutePath)` (`_TLCTrace.java:48-62`).
///
/// The stream is opened, and its gzip header written, before the value is touched, and it is
/// closed in a `finally`: a value that cannot be written leaves the file holding an empty
/// gzip member rather than what was there before.
pub(crate) fn serialize(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let path = string_argument(ev, &a[1]);
    let mut file = create(ev, &path)?;
    // "Calling IValue#fingerprint guarantees that value can be serialized because
    // fingerprinting also finalizes any internal data structures."
    let written = match a[0].fingerprint(ev, Fp::zero()) {
        Ok(_) => stream::write(ev, &a[0]),
        Err(e) => Err(e),
    };
    let bytes = match written {
        Ok(bytes) => bytes,
        Err(e) => {
            let _ = file.write_all(&tlars_java::gzip(&[]));
            return Err(e);
        }
    };
    file.write_all(&tlars_java::gzip(&bytes)).map_err(|e| io_exception(io_message(&ev.tool().globals().locale, &path, &e)))?;
    Ok(Value::Bool(true))
}

/// `new FileOutputStream(file)`, which opens for writing and truncates. Unlike `Json`, this
/// module does not create the parent directory.
fn create(ev: &mut Ev<'_>, path: &str) -> EvalResult<std::fs::File> {
    let target = resolve(ev, path);
    // `FileOutputStream`'s native open refuses a directory, which Unix `open(2)` with
    // `O_WRONLY` does too, with the same `EISDIR`.
    if target.is_dir() {
        return Err(io_exception(io_message(&ev.tool().globals().locale, path, &std::io::Error::from_raw_os_error(EISDIR))));
    }
    std::fs::File::create(&target).map_err(|e| io_exception(io_message(&ev.tool().globals().locale, path, &e)))
}

/// `ioDeserialize(absolutePath)` (`_TLCTrace.java:64-83`).
///
/// A file that is not there answers the empty record, because constant processing evaluates
/// this even when the run only writes a trace. `read(Map)` is `readExternal()`, so the
/// tokens the file carries are the writing run's and the text is interned here.
pub(crate) fn deserialize(ev: &mut Ev<'_>, a: &[Value]) -> EvalResult<Value> {
    let path = string_argument(ev, &a[0]);
    let source = resolve(ev, &path);
    // `File.exists()`, which a directory passes; opening one for reading then fails.
    if !source.exists() {
        // Reading the static `RecordValue.EmptyRcd` initialises `tlc2.value.impl.RecordValue`,
        // whose field-name statics take the next intern tokens. Constant processing evaluates
        // this operator, so in a run that only writes a trace this is where the class loads.
        ev.tool().graph().interner().ensure(tlars_sany::JavaClass::RecordValue);
        return Ok(Value::Record(empty_rcd()));
    }
    if source.is_dir() {
        return Err(io_exception(io_message(&ev.tool().globals().locale, &path, &std::io::Error::from_raw_os_error(EISDIR))));
    }
    let bytes = std::fs::read(&source).map_err(|e| io_exception(io_message(&ev.tool().globals().locale, &path, &e)))?;
    let plain = tlars_java::gunzip(&bytes).map_err(gzip_failure)?;
    stream::read_external(ev, &plain)
}

/// What `MethodValue.eval` reports for the exception `GZIPInputStream` raised: its message,
/// or, where that message is null, the Java stack trace, which is not something to reproduce.
fn gzip_failure(e: tlars_java::GzipError) -> EvalError {
    match e.message {
        Some(message) => io_exception(message.to_string()),
        None => EvalError::unimplemented("reading a trace file too short to hold a gzip header".into()),
    }
}
