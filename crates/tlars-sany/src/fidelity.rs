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
        // `tlc2.tool.Action.UNNAMED_ACTION` (`Action.java:35`), the name every action without
        // a definition reports. TLC runs this initialiser before `TLCGetSet`'s: a run that
        // prints `{TLCGet("action").name} \cup DOMAIN TLCGet("revision")` puts
        // `"UnnamedAction"` ahead of `"timestamp"`.
        JavaClass::Action => &["UnnamedAction"],
        // `tlc2.module.TLCGetSet`'s `UniqueString` fields, in declaration order. Their tokens
        // are what orders a `TLCGet("stats")` / `"config"` / `"revision"` / `"spec"` record's
        // fields, and `DOMAIN` of one prints them in that order.
        JavaClass::TlcRuntime => &[
            "exit",
            "pause",
            "config",
            "spec",
            "coverage",
            "action",
            "install",
            "id",
            "behavior",
            "all",
            "all:named",
            "mode",
            "deadlock",
            "seed",
            "fingerprint",
            "worker",
            "traces",
            "depth",
            "aril",
            "sched",
            "revision",
            "timestamp",
            "date",
            "count",
            "tag",
            "calver",
            "impliedinits",
            "impliedactions",
            "invariants",
            "impliedtemporals",
            "temporals",
            "actions",
            "inits",
            "variables",
            "actionconstraints",
            "constraints",
            "level",
            "stats",
            "duration",
            "generated",
            "diameter",
            "distinct",
            "initial",
            "queue",
            "retries",
            "distinctvalues",
            "levelmean",
            "levelvariance",
        ],
        // `tlc2.value.impl.RecordValue`'s field-name statics, in declaration order
        // (`RecordValue.java:44-53`). The class loads the first time a `RecordValue` is
        // built, which is after `TLCGetSet`'s initialiser because module overrides are
        // registered before anything is evaluated. Names a parsed module or the cfg already
        // interned keep their earlier token, which is why `"name"` and `"location"` normally
        // sort ahead of every key here.
        JavaClass::RecordValue => &[
            "beginLine",
            "beginColumn",
            "endLine",
            "endColumn",
            "module",
            "name",
            "location",
            "context",
            "parameters",
            "_action",
        ],
        // `tlc2.value.impl.CounterExample`'s `UniqueString` statics, in declaration order
        // (`CounterExample.java:48-50`). The class initialises when the first counterexample
        // is built, which is the first `TLCExt!CounterExample` that is evaluated or the
        // postcondition check of a violated run; `instanceof` in `ToTrace` does not
        // initialise it.
        JavaClass::CounterExample => &["state", "action", "console"],
    }
}
