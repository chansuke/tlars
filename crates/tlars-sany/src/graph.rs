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
    pub module: Atom,
    pub begin_line: u32,
    pub begin_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Loc {
    /// `line L, col C to line L2, col C2 of module M`, or `In module M` when all zero.
    pub fn tlc_text(&self, interner: &Interner) -> String {
        todo!()
    }
}

/// `BuiltInOperators` + `ASTConstants` internal names (`$ConjList`, `$FcnApply`, ...).
/// Generated from the pinned sources; `BuiltInOPs.getOpCode` becomes this enum.
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
    BuiltIn(BuiltinOp),
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
}

impl ExprOrOpArg {
    /// SANY guarantees operands of boolean connectives are expressions.
    pub fn expect_expr(self) -> ExprId {
        match self {
            ExprOrOpArg::Expr(e) => e,
            ExprOrOpArg::OpArg(_) => panic!("operator argument where SANY guarantees an expression"),
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
    pub theorems: Box<[ThmId]>,
    pub inner_modules: Box<[ModuleId]>,
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

impl SemanticGraph {
    pub fn root(&self) -> ModuleId {
        self.root
    }
    pub fn module(&self, id: ModuleId) -> &ModuleNode {
        &self.modules[id.index()]
    }
    pub fn module_by_name(&self, name: Atom) -> Option<ModuleId> {
        todo!()
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
        todo!()
    }
    /// Resolve `a!b!c` / plain names the way `SpecProcessor` resolves cfg identifiers.
    pub fn resolve_root_name(&self, path: &[Atom]) -> Option<SymbolRef> {
        todo!()
    }
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }
}
