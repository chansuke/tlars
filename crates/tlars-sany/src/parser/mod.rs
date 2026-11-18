//! The TLA+ parser: TLC's JavaCC `TLAplusParser`, its lexer and SANY's parser support
//! classes, translated by `xtask jj2rs` into `generated.rs`. Error text ("Was expecting
//! ...", the residual `bpa()` stack) and the parser's quirks are TLC's by construction.
//!
//! `java` is the Java semantics the translation relies on; `support` holds the classes it
//! calls that are not translated (Java library, `UniqueString`, `Location`, ...).

mod generated;
mod java;
mod support;

