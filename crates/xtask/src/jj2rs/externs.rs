//! Signatures of the hand-written support classes the translated parser calls
//! (`tlars-sany/src/parser/support`). Kept in one table so the emitter can type calls into
//! them; a call missing here is an emission error, never a guess.

use super::emit::{ClassInfo, Classes};

pub(crate) fn classes() -> Classes {
    let mut c = Classes::new();
    c.insert("Object".into(), ClassInfo::default());
    c
}
