//! TLC's `setToolObject(toolId, ...)` side data, as immutable tables indexed by graph ids.

use crate::modules::NativeId;
use crate::state::VarSlot;
use crate::value::Value;
use tlars_sany::graph::{ExprId, OpDefId};

pub(crate) enum OpBinding {
    /// `processConstantDefns` result (deep-normalised).
    Folded(Value),
    /// Java-module override (`MethodValue`), evaluation only.
    Native(NativeId),
    /// cfg `Op <- Other`.
    Replaced(OpDefId),
    /// cfg `Op(args) = v` (`OpRcdValue`).
    Table(Box<[(Box<[Value]>, Value)]>),
}

pub(crate) enum DeclBinding {
    /// cfg `C = v`.
    Value(Value),
    /// cfg `C <- Op`: evaluated on every use unless aggressiveConstantCaching (issue 648).
    Op(OpDefId),
    Var(VarSlot),
}

pub(crate) struct Bindings {
    pub(crate) op_defs: Box<[Option<OpBinding>]>,
    pub(crate) decls: Box<[Option<DeclBinding>]>,
    /// `processConstants`: Numeral / String literal values by `ExprId`.
    pub(crate) literals: Box<[Option<Value>]>,
}

impl Bindings {
    pub(crate) fn op_def(&self, id: OpDefId) -> Option<&OpBinding> {
        self.op_defs[id.index()].as_ref()
    }
    pub(crate) fn literal(&self, id: ExprId) -> Option<&Value> {
        self.literals[id.index()].as_ref()
    }
}
