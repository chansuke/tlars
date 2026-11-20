//! Java standard-library behaviour that leaks into TLC's observable output.
//!
//! Membership rule: an algorithm belongs here iff it reproduces a JDK or TLC utility class
//! bit for bit and takes and returns only integers, bytes, strings or caller-supplied
//! payloads, never a TLA+ value. This crate knows nothing about TLA+.
//!
//! | Java                          | Used by TLC for                                    |
//! |-------------------------------|----------------------------------------------------|
//! | `String.hashCode`             | SANY `Context` Hashtable bucket order              |
//! | `java.util.Hashtable`         | variable / constant / `I!Op` declaration order     |
//! | `HashSet<Integer>.toString`   | message-code sets in CLI errors                    |
//! | `java.util.Random`            | RandomElement, simulation, `-fp`/`-seed` defaults  |
//! | `Double.toString` (JDK 21)    | collision-probability line (`ProbabilityToString`) |
//! | `BigDecimal(MathContext(2))`  | "based on the actual fingerprints" probability     |
//! | `DecimalFormat("###,###.###")`| progress lines (`MP.format`)                       |
//! | `SimpleDateFormat`            | timestamps, `TLC_FINISHED` durations, metadir      |
//! | `tlc2.pprint.*`               | state values wrapped at `Values.width` (80)        |
//! | JVM exception messages        | the casts and array reads TLC lets escape          |
//!
//! Golden files under `tests/golden` come from `tools/golden/JavaGolden.java` run on the
//! pinned JDK 21, with `$TLA2TOOLS_JAR` on the class path for the one corpus that asks TLC
//! itself rather than the JDK (`pprint.txt`).

mod exception;
mod hashmap;
mod hashset;
mod hashtable;
mod number;
mod parse;
mod pprint;
mod random;
pub mod time;

pub use exception::{class_cast, index_out_of_bounds};
pub use hashmap::{JavaHashMap, capacity_for_mappings, table_size_for};
pub use hashset::JavaIntHashSet;
pub use hashtable::{Elements, Hashtable};
pub use number::{decimal_format_grouped, double_to_string, reciprocal_two_digits};
pub use parse::{NumberFormatError, parse_double, parse_int, parse_long};
pub use pprint::pretty_print;
pub use random::JavaRandom;

/// `java.lang.String.hashCode()`: `s[0]*31^(n-1) + ...` over UTF-16 code units, with `int`
/// wrapping arithmetic.
pub fn string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(unit));
    }
    h
}
