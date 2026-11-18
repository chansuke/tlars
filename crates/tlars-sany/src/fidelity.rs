//! Generated data that TLC's output depends on, copied out of the pinned tlaplus sources by
//! `xtask gen-fidelity` (and checked by `xtask gen-fidelity --check` in CI):
//! class-static `UniqueString`s, `BuiltInOperators`, the `Operators` table and synonyms,
//! `SyntaxTreeConstants` node names. (The embedded standard modules are `standard_modules.rs`.)
//! One list each; everything else derives from these.

use crate::JavaClass;

/// Static strings of `class`, in source order.
pub(crate) fn class_statics(class: JavaClass) -> &'static [&'static str] {
    match class {
        JavaClass::ContextReInit => &[/* generated */],
        JavaClass::SyntaxTreeNode => &["***I do not exist***", "--TLA+ BUILTINS--"],
        JavaClass::OpDefNode => &["Formal_0", "Formal_1", "Formal_2"],
        JavaClass::Operators => &[/* generated */],
        JavaClass::SyntaxTreeConstants => &[/* generated from SyntaxTreeConstants.java:151 */],
        JavaClass::Location => &["--unknown--"],
        JavaClass::TlcRuntime => &[/* TLCGetSet keys, UnnamedAction, ... */],
    }
}
