//! The messages of the unchecked exceptions TLC lets escape, which land in its output
//! verbatim. Both are built by the JDK, not by TLC, and both are stable since JDK 9.

/// `ArrayIndexOutOfBoundsException`'s message as `Preconditions.outOfBounds` builds it.
pub fn index_out_of_bounds(index: i64, length: usize) -> String {
    format!("Index {index} out of bounds for length {length}")
}

/// `ClassCastException`'s message for two classes that share a class loader, which is where
/// every `tlc2.value.impl` class lives when TLC runs from its jar.
pub fn class_cast(from: &str, to: &str) -> String {
    format!("class {from} cannot be cast to class {to} ({from} and {to} are in unnamed module of loader 'app')")
}
