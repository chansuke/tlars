//! The TLA+ parser: TLC's JavaCC `TLAplusParser`, its lexer and SANY's parser support
//! classes, translated by `xtask jj2rs` into `generated.rs`. Error text ("Was expecting
//! ...", the residual `bpa()` stack) and the parser's quirks are TLC's by construction.
//!
//! The same translation covers SANY's module loading (`tla2sany.modanalyzer`) and semantic
//! analysis with level checking (`tla2sany.semantic`); `driver` is `SANY.parse` over them.
//!
//! `java` is the Java semantics the translation relies on; `support` holds the classes it
//! calls that are not translated (Java library, `UniqueString`, `Location`, ...), and
//! `files` how modules are found and read.

mod driver;
mod files;
mod generated;
mod java;
mod support;


#[cfg(test)]
mod semantic_tests;
#[cfg(test)]
mod tests;
