//! TLC's `setToolObject(toolId, ...)` side data, as immutable tables indexed by graph ids.

use crate::modules::NativeId;
use crate::value::{OpRcd, Value};
use tlars_sany::graph::{ExprId, OpDefId};

#[derive(Clone)]
pub(crate) enum OpBinding {
    /// A `Value` tool object: a cfg `Op = v` or a `processConstantDefns` result
    /// (deep-normalised).
    Value(Value),
    /// Java-module override (`MethodValue`), evaluation only.
    Native(NativeId),
    /// An `OpDefNode` tool object: the target of a cfg `Op <- Other`, and what `defns`
    /// holds for a definition the cfg says nothing about.
    Replaced(OpDefId),
    /// cfg `Op(args) = v` (`OpRcdValue`).
    Table(OpRcd),
    /// A definition TLC overrides with a Java class tlars does not implement yet. The level
    /// is the override's `minLevel`, which is all `getLevelBound` reads off a `MethodValue`
    /// or `EvaluatingValue` (`SymbolNodeValueLookupProvider.java:270-276`).
    Unimplemented { min_level: u8 },
}

#[derive(Clone)]
pub(crate) enum DeclBinding {
    /// cfg `C = v`.
    Value(Value),
    /// cfg `C(args) = v` on a declared constant operator.
    Table(OpRcd),
    /// cfg `C <- Op`: evaluated on every use unless aggressiveConstantCaching (issue 648).
    Op(OpDefId),
    /// cfg `C <- Op` where `Op` is a Java-module override (`MethodValue`).
    Native(NativeId),
}

pub(crate) struct Bindings {
    pub(crate) op_defs: Box<[Option<OpBinding>]>,
    /// Tool objects on definition bodies, by `ExprId`: the Java overrides
    /// (`opDef.getBody().setToolObject`, `SpecProcessor.java:1067`).
    pub(crate) bodies: Box<[Option<OpBinding>]>,
    pub(crate) decls: Box<[Option<DeclBinding>]>,
    /// `processConstants`: Numeral / String literal values by `ExprId`.
    pub(crate) literals: Box<[Option<Value>]>,
}

impl Bindings {
    pub(crate) fn op_def(&self, id: OpDefId) -> Option<&OpBinding> {
        self.op_defs[id.index()].as_ref()
    }
    pub(crate) fn body(&self, id: ExprId) -> Option<&OpBinding> {
        self.bodies[id.index()].as_ref()
    }
    /// The tool object `lookup` finds on a definition's body, following INSTANCE
    /// substitutions down to the instantiated body (`SymbolNodeValueLookupProvider.java:99-111`).
    ///
    /// TLC has one tool-object slot per node, so this finds either a Java override
    /// (`processModuleOverrides`) or the literal `processConstants` left on a numeral or
    /// string body. That is why `SelectSeq(<<1>>, LAMBDA e : 1)` reaches the module with the
    /// value `1` instead of the operator.
    pub(crate) fn body_chain(&self, graph: &tlars_sany::graph::SemanticGraph, body: ExprId) -> Option<OpBinding> {
        let mut e = body;
        loop {
            if let Some(b) = self.body(e) {
                return Some(b.clone());
            }
            if let Some(v) = self.literal(e) {
                return Some(OpBinding::Value(v.clone()));
            }
            match graph.expr(e) {
                tlars_sany::graph::ExprNode::SubstIn { body, .. } => e = *body,
                _ => return None,
            }
        }
    }
    pub(crate) fn literal(&self, id: ExprId) -> Option<&Value> {
        self.literals[id.index()].as_ref()
    }
}
