//! TLC's `setToolObject(toolId, ...)` side data, as immutable tables indexed by graph ids.

use crate::modules::NativeId;
use crate::state::VarSlot;
use crate::value::Value;
use tlars_sany::graph::{ExprId, OpDefId};

#[derive(Clone)]
pub(crate) enum OpBinding {
    /// A `Value` tool object: a cfg `Op = v` or a `processConstantDefns` result
    /// (deep-normalised).
    Value(Value),
    /// Java-module override (`MethodValue`), evaluation only.
    Native(NativeId),
    /// cfg `Op <- Other`.
    Replaced(OpDefId),
    /// cfg `Op(args) = v` (`OpRcdValue`).
    Table(OpRcd),
    /// A definition TLC overrides with a Java class tlars does not implement yet.
    Unimplemented,
}

/// `OpRcdValue`: the cfg's `op(a, b) = v` lines for one name, in cfg order.
pub(crate) type OpRcd = std::sync::Arc<[(Box<[Value]>, Value)]>;

#[derive(Clone)]
pub(crate) enum DeclBinding {
    /// cfg `C = v`.
    Value(Value),
    /// cfg `C(args) = v` on a declared constant operator.
    Table(OpRcd),
    /// cfg `C <- Op`: evaluated on every use unless aggressiveConstantCaching (issue 648).
    Op(OpDefId),
    Var(VarSlot),
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
    pub(crate) fn body_chain(&self, graph: &tlars_sany::graph::SemanticGraph, body: ExprId) -> Option<&OpBinding> {
        let mut e = body;
        loop {
            if let Some(b) = self.body(e) {
                return Some(b);
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
