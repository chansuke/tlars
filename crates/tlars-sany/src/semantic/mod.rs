//! `Generator` (semantic analysis), SANY `Context`, `LevelNode` level checking, `Errors`.
//! Ported 1:1 from `tla2sany/semantic/*`.

use crate::graph::{DeclId, ModuleId, OpDefId, SymbolRef};
use crate::{Atom, Interner};

/// SANY's symbol table (`tla2sany.semantic.Context`). The `Hashtable` *is* the order oracle:
/// `getVariableDecls`/`getConstantDecls`/`getByClass` enumerate it, and `ModuleNode`
/// reverses the result (`Context.java:300-336`, `ModuleNode.java:384-409`). It holds every
/// symbol of the module, including names merged in from EXTENDS, so an unrelated definition
/// can trigger a rehash that reorders variables. No other code computes declaration order.
pub(crate) struct Context {
    table: tlars_java::Hashtable<Atom, SymbolRef>,
    /// Insertion-ordered list used by `getOpDefs` (not the Hashtable).
    insertion: Vec<SymbolRef>,
}

impl Context {
    pub(crate) fn add_symbol(&mut self, interner: &Interner, name: Atom, sym: SymbolRef) {
        let _hash = interner.java_hash(name);
        todo!()
    }
    pub(crate) fn variable_decls_reversed(&self) -> Vec<DeclId> {
        todo!()
    }
    pub(crate) fn constant_decls_reversed(&self) -> Vec<DeclId> {
        todo!()
    }
    pub(crate) fn op_defs_by_class(&self) -> Vec<OpDefId> {
        todo!()
    }
}

/// `tla2sany.semantic.Errors`: messages in SANY's exact format (location, blank line, text).
#[derive(Default)]
pub(crate) struct Errors {
    errors: Vec<String>,
    warnings: Vec<String>,
}

/// Mutable graph under construction. Only `semantic` writes it; `freeze()` yields the
/// immutable `SemanticGraph`.
pub(crate) struct GraphBuilder {
    // arenas mirror graph::SemanticGraph
}

pub(crate) fn generate(builder: &mut GraphBuilder, module: ModuleId, errors: &mut Errors) {
    // TODO: Generator.generate: definitions, INSTANCE (clone bodies into SubstIn per I!Op,
    // implicit `x <- x`), LET, RECURSIVE sections, labels and `!` selectors, proofs
    // (parsed, level-checked, ignored by TLC), PlusCal hash validation hook.
    todo!()
}

pub(crate) fn level_check(builder: &mut GraphBuilder, module: ModuleId, errors: &mut Errors) {
    // TODO: LevelNode.levelCheck incl. levelParams/levelConstraints/argLevelConstraints,
    // optimistic iteration over recursive sections (levels never decrease).
    todo!()
}
