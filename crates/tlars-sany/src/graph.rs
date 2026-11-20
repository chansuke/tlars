//! The frozen SANY semantic graph (`tla2sany.semantic.*`), as typed arenas.
//!
//! Invariants:
//! - Built once by `load`, then immutable (`&` only). `SemanticGraph: Send + Sync`.
//! - Ids index their own arena and can only be minted inside this crate, so an `OpDefId`
//!   can never index expressions and no caller can fabricate a dangling id.
//! - Declaration orders (`variable_decls`, `constant_decls`, `I!Op` defs) are computed here
//!   from SANY's Hashtable-backed `Context` and nowhere else.
//! - TLC's mutable "tool objects" are *not* here; `tlars-tool` keeps them in side tables.

use crate::{Atom, Interner};
use std::sync::Arc;

macro_rules! id {
    ($($name:ident),*) => {$(
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(u32);
        impl $name {
            pub(crate) fn new(i: u32) -> Self { Self(i) }
            pub fn index(self) -> usize { self.0 as usize }
        }
    )*};
}
id!(ModuleId, OpDefId, DeclId, ParamId, ThmId, ExprId);

/// SANY `SymbolNode` identity. `Context.lookup`'s `==` becomes a compare of this enum.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum SymbolRef {
    OpDef(OpDefId),
    Decl(DeclId),
    Param(ParamId),
    Thm(ThmId),
    Module(ModuleId),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Constant = 0,
    State = 1,
    Action = 2,
    Temporal = 3,
}

/// `tla2sany.st.Location`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Loc {
    /// `None` for a location without a module (`new Location(bl, bc, el, ec)`).
    pub module: Option<Atom>,
    pub begin_line: i32,
    pub begin_col: i32,
    pub end_line: i32,
    pub end_col: i32,
}

impl Loc {
    /// `Location.toString()`: `line L, col C to line L2, col C2 of module M`, or
    /// `In module M` when all four are zero.
    pub fn tlc_text(&self, interner: &Interner) -> String {
        let module = self.module.map_or_else(|| "null".to_string(), |m| interner.text(m).to_string());
        if self.module.is_some() && self.begin_line == 0 && self.begin_col == 0 && self.end_line == 0 && self.end_col == 0 {
            return format!("In module {module}");
        }
        format!(
            "line {}, col {} to line {}, col {} of module {module}",
            self.begin_line, self.begin_col, self.end_line, self.end_col
        )
    }
}

/// `BuiltInOperators` + `ASTConstants` internal names (`$ConjList`, `$FcnApply`, ...).
/// TLC's operator codes (`BuiltInOPs.getOpCode`), which the evaluator derives from a
/// built-in operator's name. Generated from the pinned sources.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuiltinOp {
    ConjList,
    DisjList,
    Land,
    Lor,
    Lnot,
    Implies,
    Equiv,
    Eq,
    Noteq,
    In,
    Notin,
    BoundedExists,
    BoundedForall,
    UnboundedExists,
    UnboundedChoose,
    BoundedChoose,
    Prime,
    Unchanged,
    Enabled,
    SquareAct,
    AngleAct,
    IfThenElse,
    Case,
    Except,
    FcnApply,
    FcnConstructor,
    RcdConstructor,
    RcdSelect,
    SetEnumerate,
    SetOfAll,
    SubsetOf,
    Tuple,
    CartesianProd,
    Subset,
    Union,
    Cup,
    Cap,
    SetMinus,
    Subseteq,
    Domain,
    SetOfFcns,
    SetOfRcds,
    Always,
    Eventually,
    LeadsTo,
    WF,
    SF,
    // ... remaining rows generated
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpDefKind {
    User,
    /// A built-in operator (`BuiltInOperators`), known by its name (`\\in`, `$FcnApply`,
    /// ...). TLC maps names to its operator codes (`BuiltInOPs`), which is the evaluator's
    /// business.
    BuiltIn,
    /// `I == INSTANCE M` itself.
    ModuleInstance,
}

pub struct OpDefNode {
    pub name: Atom,
    pub kind: OpDefKind,
    pub params: Box<[ParamId]>,
    pub body: Option<ExprId>,
    pub module: ModuleId,
    pub loc: Loc,
    pub local: bool,
    pub recursive: bool,
    /// For an `I!Op` created by INSTANCE: the definition it was cloned from.
    pub instantiated_from: Option<OpDefId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeclKind {
    Constant,
    Variable,
    NewConstant,
    NewVariable,
}

pub struct OpDeclNode {
    pub name: Atom,
    pub kind: DeclKind,
    pub arity: i32,
    pub module: ModuleId,
    pub loc: Loc,
}

pub struct FormalParamNode {
    pub name: Atom,
    pub arity: i32,
    pub loc: Loc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExprOrOpArg {
    Expr(ExprId),
    OpArg(SymbolRef),
    /// A null argument: the guard of `CASE`'s `OTHER` arm (`$Pair(null, e)`).
    Absent,
}

impl ExprOrOpArg {
    /// SANY guarantees operands of boolean connectives are expressions.
    pub fn expect_expr(self) -> ExprId {
        match self {
            ExprOrOpArg::Expr(e) => e,
            ExprOrOpArg::OpArg(_) | ExprOrOpArg::Absent => panic!("operator argument where SANY guarantees an expression"),
        }
    }
}

/// `OpApplNode` bound symbols: `quantSymbolLists` + `tupleOrs` + `ranges`, and
/// `unboundedBoundSymbols`. Order is source order (drives `ContextEnumerator`).
pub struct BoundSymbols {
    pub lists: Box<[BoundList]>,
    pub unbounded: Box<[ParamId]>,
}

pub struct BoundList {
    pub vars: Box<[ParamId]>,
    /// `<<x, y>> \in S`.
    pub tuple: bool,
    pub domain: ExprId,
}

/// One `op <- e` of an INSTANCE (`Subst`), implicit `x <- x` included.
pub struct Subst {
    pub decl: DeclId,
    pub expr: ExprOrOpArg,
    pub implicit: bool,
}

/// One variant per SANY `ExprNode` class.
pub enum ExprNode {
    OpAppl {
        op: SymbolRef,
        args: Box<[ExprOrOpArg]>,
        bound: Option<Box<BoundSymbols>>,
        loc: Loc,
    },
    LetIn {
        defs: Box<[OpDefId]>,
        body: ExprId,
        loc: Loc,
    },
    SubstIn {
        substs: Box<[Subst]>,
        body: ExprId,
        instantiating: ModuleId,
        instantiated: ModuleId,
        loc: Loc,
    },
    APSubstIn {
        substs: Box<[Subst]>,
        body: ExprId,
        loc: Loc,
    },
    Label {
        name: Atom,
        params: Box<[ParamId]>,
        body: ExprId,
        loc: Loc,
    },
    At {
        except_ref: ExprId,
        except_component: ExprId,
        loc: Loc,
    },
    /// Digits as written (`\b`, `\o`, `\h` already decoded). TLC's 2^31-1 check happens in
    /// `processConstants`, not here.
    Numeral {
        digits: Box<str>,
        loc: Loc,
    },
    Decimal {
        text: Box<str>,
        loc: Loc,
    },
    String {
        value: Atom,
        loc: Loc,
    },
}

pub struct AssumeNode {
    pub expr: ExprId,
    pub is_axiom: bool,
    pub def: Option<ThmId>,
    pub loc: Loc,
}

pub struct ThmOrAssumpDefNode {
    pub name: Atom,
    pub body: ExprId,
    pub params: Box<[ParamId]>,
}

/// Proofs are not here: TLC does not evaluate them.
pub struct ModuleNode {
    pub name: Atom,
    pub loc: Loc,
    pub extends: Box<[ModuleId]>,
    /// `ModuleNode.getConstantDecls()`: reversed Hashtable enumeration of the module Context.
    pub constant_decls: Box<[DeclId]>,
    /// `ModuleNode.getVariableDecls()`: reversed Hashtable enumeration. This is TLC `vars[]`.
    pub variable_decls: Box<[DeclId]>,
    /// `getOpDefs()`: insertion order (linked list, not the Hashtable).
    pub op_defs: Box<[OpDefId]>,
    /// Includes extended, excludes instantiated (`getAssumptions`).
    pub assumptions: Box<[AssumeNode]>,
    pub inner_modules: Box<[ModuleId]>,
    /// `getTheorems()` bodies that are expressions; `ASSUME ... PROVE` theorems are left out.
    pub theorems: Box<[ExprId]>,
    pub is_standard: bool,
    pub is_instantiated: bool,
    /// `ModuleNode.isParameterFree()`; with `is_instantiated` gates constant folding.
    pub parameter_free: bool,
}

/// The whole spec as SANY sees it after a successful `load`.
pub struct SemanticGraph {
    modules: Vec<ModuleNode>,
    op_defs: Vec<OpDefNode>,
    decls: Vec<OpDeclNode>,
    params: Vec<FormalParamNode>,
    thms: Vec<ThmOrAssumpDefNode>,
    exprs: Vec<ExprNode>,
    /// Level checker output, indexed by `ExprId` / `OpDefId`.
    expr_levels: Vec<Level>,
    op_levels: Vec<Level>,
    root: ModuleId,
    interner: Arc<Interner>,
}

/// The arenas `parser::freeze` fills, in id order.
#[derive(Default)]
pub(crate) struct GraphParts {
    pub(crate) modules: Vec<ModuleNode>,
    pub(crate) op_defs: Vec<OpDefNode>,
    pub(crate) decls: Vec<OpDeclNode>,
    pub(crate) params: Vec<FormalParamNode>,
    pub(crate) thms: Vec<ThmOrAssumpDefNode>,
    pub(crate) exprs: Vec<ExprNode>,
    pub(crate) expr_levels: Vec<Level>,
    pub(crate) op_levels: Vec<Level>,
}

impl SemanticGraph {
    pub(crate) fn from_parts(parts: GraphParts, root: ModuleId, interner: Arc<Interner>) -> SemanticGraph {
        let GraphParts { modules, op_defs, decls, params, thms, exprs, expr_levels, op_levels } = parts;
        SemanticGraph { modules, op_defs, decls, params, thms, exprs, expr_levels, op_levels, root, interner }
    }

    pub fn root(&self) -> ModuleId {
        self.root
    }
    pub fn module(&self, id: ModuleId) -> &ModuleNode {
        &self.modules[id.index()]
    }
    /// `ExternalModuleTable.getModuleNode(name)`.
    pub fn module_by_name(&self, name: Atom) -> Option<ModuleId> {
        self.modules.iter().position(|m| m.name == name).map(|i| ModuleId::new(i as u32))
    }
    /// Every module, in the order SANY analysed them (`ExternalModuleTable`); inner modules
    /// follow.
    pub fn modules(&self) -> impl Iterator<Item = (ModuleId, &ModuleNode)> {
        self.modules.iter().enumerate().map(|(i, m)| (ModuleId::new(i as u32), m))
    }
    /// Arena sizes, for tables indexed by id.
    pub fn op_def_count(&self) -> usize {
        self.op_defs.len()
    }
    pub fn decl_count(&self) -> usize {
        self.decls.len()
    }
    pub fn expr_count(&self) -> usize {
        self.exprs.len()
    }
    pub fn op_def(&self, id: OpDefId) -> &OpDefNode {
        &self.op_defs[id.index()]
    }
    pub fn decl(&self, id: DeclId) -> &OpDeclNode {
        &self.decls[id.index()]
    }
    pub fn param(&self, id: ParamId) -> &FormalParamNode {
        &self.params[id.index()]
    }
    pub fn thm(&self, id: ThmId) -> &ThmOrAssumpDefNode {
        &self.thms[id.index()]
    }
    pub fn expr(&self, id: ExprId) -> &ExprNode {
        &self.exprs[id.index()]
    }
    pub fn level(&self, id: ExprId) -> Level {
        self.expr_levels[id.index()]
    }
    pub fn op_level(&self, id: OpDefId) -> Level {
        self.op_levels[id.index()]
    }
    pub fn loc(&self, id: ExprId) -> Loc {
        match self.expr(id) {
            ExprNode::OpAppl { loc, .. }
            | ExprNode::LetIn { loc, .. }
            | ExprNode::SubstIn { loc, .. }
            | ExprNode::APSubstIn { loc, .. }
            | ExprNode::Label { loc, .. }
            | ExprNode::At { loc, .. }
            | ExprNode::Numeral { loc, .. }
            | ExprNode::Decimal { loc, .. }
            | ExprNode::String { loc, .. } => *loc,
        }
    }
    /// `ExprNode.toString()`: a numeral or decimal prints as written, every other expression
    /// as its location (`SemanticNode.toString`).
    pub fn expr_tlc_text(&self, id: ExprId) -> String {
        match self.expr(id) {
            ExprNode::Numeral { digits, .. } => digits.to_string(),
            ExprNode::Decimal { text, .. } => text.to_string(),
            _ => self.loc(id).tlc_text(&self.interner),
        }
    }
    /// Resolve `a!b!c` / plain names the way `SpecProcessor` resolves cfg identifiers.
    pub fn resolve_root_name(&self, path: &[Atom]) -> Option<SymbolRef> {
        todo!("M3: SpecProcessor's name resolution for cfg identifiers")
    }
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }
}
