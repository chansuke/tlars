//! `tlc2.pprint.{PrettyPrint, Parse, Format}`: re-parse a value's flat `toString`; if the
//! parse does not consume everything, print it flat; if it fits in `width`, print flat;
//! otherwise break with Format's fixed pads (`{ `, ` }`, `<< `, ` >>`, ` @@`, ` :>`, ` |->`,
//! `SUBSET `). The width does not count the `/\ x = ` prefix.

/// Works on strings only, which is why it lives with the Java-library ports.
pub fn pretty_print(flat: &str, width: usize) -> String {
    let _ = (flat, width);
    todo!("M2: port Parse.java + Format.java")
}
