//! The messages of the unchecked exceptions TLC lets escape, which land in its output
//! verbatim. All are built by the JDK, not by TLC, and all are stable since JDK 9.

/// `ArrayIndexOutOfBoundsException`'s message as `Preconditions.outOfBounds` builds it.
pub fn index_out_of_bounds(index: i64, length: usize) -> String {
    format!("Index {index} out of bounds for length {length}")
}

/// `StringIndexOutOfBoundsException`'s message for `String.substring(begin, end)` outside
/// the string, as `Preconditions.checkFromToIndex` builds it. `length` is in UTF-16 code
/// units, which is what `String.length()` counts.
pub fn range_out_of_bounds(begin: i64, end: i64, length: usize) -> String {
    format!("Range [{begin}, {end}) out of bounds for length {length}")
}

/// `String.substring(begin, end)`: the UTF-16 code units in `[begin, end)`, or the message
/// of the `StringIndexOutOfBoundsException` it throws. Java counts and slices in code
/// units, so a surrogate pair is two positions and an index may split one. Java keeps the
/// unpaired surrogate a split leaves behind; a Rust `String` cannot hold one, so it becomes
/// the replacement character.
pub fn substring(s: &str, begin: i64, end: i64) -> Result<String, String> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let length = units.len();
    if begin < 0 || begin > end || end > length as i64 {
        return Err(range_out_of_bounds(begin, end, length));
    }
    Ok(String::from_utf16_lossy(&units[begin as usize..end as usize]))
}

/// A helpful `NullPointerException`'s message (JEP 358, on by default since JDK 15):
/// `Cannot invoke "<Class.method(sig)>" because "<expr>" is null`, where `expr` is the
/// source expression the bytecode attributes the null to.
pub fn null_method_call(method: &str, expr: &str) -> String {
    format!("Cannot invoke \"{method}\" because \"{expr}\" is null")
}

/// `ClassCastException`'s message for two classes that share a class loader, which is where
/// every `tlc2.value.impl` class lives when TLC runs from its jar.
pub fn class_cast(from: &str, to: &str) -> String {
    format!("class {from} cannot be cast to class {to} ({from} and {to} are in unnamed module of loader 'app')")
}

#[cfg(test)]
mod tests {
    /// What JDK 21's `String.substring(13, length() - 1)` answers, which is how TLC cuts
    /// `Permutations(` off a symmetry set's image.
    #[test]
    fn substring_counts_utf16_code_units() {
        let cut = |s: &str| super::substring(s, 13, s.encode_utf16().count() as i64 - 1);
        assert_eq!(cut("Permutations(S)"), Ok("S".to_string()));
        assert_eq!(cut("Permutations1"), Err("Range [13, 12) out of bounds for length 13".to_string()));
        assert_eq!(cut("Permutations"), Err("Range [13, 11) out of bounds for length 12".to_string()));
        // Two code units each, so the pair is length 14 and the cut leaves the first one.
        assert_eq!(cut("Permutations\u{3042}"), Err("Range [13, 12) out of bounds for length 13".to_string()));
        assert_eq!(cut("Permutations\u{3042}\u{3044}"), Ok(String::new()));
        // A supplementary character is a surrogate pair, so the cut can split it.
        assert_eq!(super::substring("\u{1f600}", 1, 2), Ok("\u{fffd}".to_string()));
    }
}
