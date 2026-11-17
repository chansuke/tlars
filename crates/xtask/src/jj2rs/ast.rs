//! AST of the Java subset in the generated parser: enough to emit Rust, nothing more.

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Type {
    pub(crate) name: String,
    pub(crate) dims: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Method {
    pub(crate) name: String,
    pub(crate) ret: Type,
    pub(crate) params: Vec<(Type, String)>,
    pub(crate) body: Vec<Stmt>,
    pub(crate) line: usize,
}

#[derive(Clone, Debug)]
pub(crate) enum Stmt {
    Local { ty: Type, vars: Vec<(String, Option<Expr>)> },
    Expr(Expr),
    If { cond: Expr, then: Box<Stmt>, els: Option<Box<Stmt>> },
    While { cond: Expr, body: Box<Stmt> },
    DoWhile { body: Box<Stmt>, cond: Expr },
    For { init: Vec<Stmt>, cond: Option<Expr>, update: Vec<Expr>, body: Box<Stmt> },
    Switch { scrutinee: Expr, arms: Vec<SwitchArm> },
    Labeled { label: String, body: Box<Stmt> },
    Break(Option<String>),
    Continue(Option<String>),
    Return(Option<Expr>),
    Throw(Expr),
    Try { body: Vec<Stmt>, catches: Vec<(Type, String, Vec<Stmt>)>, finally: Option<Vec<Stmt>> },
    Block(Vec<Stmt>),
    Empty,
}

/// `case A: case B: stmts` (labels empty for `default:`).
#[derive(Clone, Debug)]
pub(crate) struct SwitchArm {
    pub(crate) labels: Vec<Expr>,
    pub(crate) is_default: bool,
    pub(crate) body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub(crate) enum Expr {
    Name(String),
    Int(String),
    Char(String),
    Str(String),
    Bool(bool),
    Null,
    This,
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call { target: Option<Box<Expr>>, name: String, args: Vec<Expr> },
    New { ty: String, args: Vec<Expr> },
    NewArray { ty: Type, len: Option<Box<Expr>>, init: Option<Vec<Expr>> },
    ArrayLit(Vec<Expr>),
    Cast(Type, Box<Expr>),
    Unary(&'static str, Box<Expr>),
    /// `x++` / `x--`.
    Postfix(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Assign(&'static str, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    InstanceOf(Box<Expr>, Type),
}
