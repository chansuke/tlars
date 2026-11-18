//! Recursive-descent parser for the Java subset of the generated parser. It reads a whole
//! class body; members it cannot parse are reported, not fatal, so the survey can count them.

use super::ast::{Expr, Method, Stmt, SwitchArm, Type, Var};
use super::lex::{Tok, Token};

pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) line: usize,
    pub(crate) kind: MemberKind,
}

pub(crate) enum MemberKind {
    Method(Method),
    Constructor(Method),
    Field { decl: Stmt, is_static: bool, is_final: bool },
    Class { members: Vec<Member> },
    StaticInit(Vec<Stmt>),
    /// Fields, nested classes, initializers: kept as source line ranges for hand porting.
    Other(&'static str),
    /// A member this parser does not understand, with the error.
    Unparsed(String),
}

pub(crate) struct Parser<'t> {
    toks: &'t [Token],
    i: usize,
    /// `>` still owed by a `>>`/`>>>` token that closed only part of nested type arguments.
    owed_gt: usize,
}

type R<T> = Result<T, String>;

const MODIFIERS: &[&str] =
    &["public", "private", "protected", "static", "final", "synchronized", "abstract", "transient", "volatile", "native"];
const PRIMITIVES: &[&str] = &["int", "long", "short", "byte", "char", "boolean", "double", "float", "void"];

impl<'t> Parser<'t> {
    pub(crate) fn new(toks: &'t [Token]) -> Self {
        Parser { toks, i: 0, owed_gt: 0 }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }
    fn line(&self) -> usize {
        self.toks[self.i].line
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i].tok.clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    fn is_ident(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Ident(q) if q == s)
    }
    fn eat_punct(&mut self, p: &str) -> bool {
        if self.is_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_punct(&mut self, p: &str) -> R<()> {
        if self.eat_punct(p) { Ok(()) } else { Err(format!("line {}: expected {p:?}, found {:?}", self.line(), self.peek())) }
    }
    fn ident(&mut self) -> R<String> {
        match self.bump() {
            Tok::Ident(s) => Ok(s),
            t => Err(format!("line {}: expected identifier, found {t:?}", self.line())),
        }
    }

    /// Skips to the end of the current member (`;` at depth 0, or a balanced block).
    fn skip_member(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.bump() {
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => {
                    depth -= 1;
                    if depth <= 0 {
                        return;
                    }
                }
                Tok::Punct(";") if depth == 0 => return,
                Tok::Eof => return,
                _ => {}
            }
        }
    }

    // ---- members ----

    /// All members of the class whose body starts at the current `{`.
    pub(crate) fn class_body(&mut self) -> R<Vec<Member>> {
        self.expect_punct("{")?;
        let mut members = Vec::new();
        while !self.is_punct("}") && *self.peek() != Tok::Eof {
            let start = self.i;
            let line = self.line();
            match self.member() {
                Ok(m) => members.push(m),
                Err(e) => {
                    self.i = start;
                    let name = self.member_name_guess();
                    self.skip_member();
                    members.push(Member { name, line, kind: MemberKind::Unparsed(e) });
                }
            }
        }
        Ok(members)
    }

    fn member_name_guess(&self) -> String {
        let mut j = self.i;
        while j < self.toks.len() {
            if let (Tok::Ident(n), Tok::Punct("(" | "=" | ";")) = (&self.toks[j].tok, &self.toks[(j + 1).min(self.toks.len() - 1)].tok) {
                return n.clone();
            }
            j += 1;
        }
        "?".into()
    }

    fn member(&mut self) -> R<Member> {
        let line = self.line();
        if self.is_punct(";") {
            self.bump();
            return Ok(Member { name: ";".into(), line, kind: MemberKind::Other("empty") });
        }
        // Annotations (`@Override`, `@SuppressWarnings("...")`) carry no semantics here.
        while self.eat_punct("@") {
            self.ident()?;
            if self.is_punct("(") {
                self.args()?;
            }
        }
        let (mut is_static, mut is_final) = (false, false);
        while let Tok::Ident(m) = self.peek() {
            if !MODIFIERS.contains(&m.as_str()) {
                break;
            }
            is_static |= m == "static";
            is_final |= m == "final";
            self.bump();
        }
        if self.is_punct("{") {
            let body = self.block()?;
            return Ok(Member { name: "static {}".into(), line, kind: MemberKind::StaticInit(body) });
        }
        if self.is_ident("class") || self.is_ident("interface") {
            self.bump();
            let name = self.ident()?;
            while !self.is_punct("{") {
                self.bump();
            }
            let members = self.class_body()?;
            self.expect_punct("}")?;
            return Ok(Member { name, line, kind: MemberKind::Class { members } });
        }
        // Constructor: `Name (`.
        if matches!((self.peek(), self.peek_at(1)), (Tok::Ident(_), Tok::Punct("("))) {
            let name = self.ident()?;
            let params = self.params()?;
            self.throws()?;
            let body = self.block()?;
            return Ok(Member {
                name: name.clone(),
                line,
                kind: MemberKind::Constructor(Method { name, ret: Type::named("void"), params, body, is_static: false }),
            });
        }
        let field_start = self.i;
        let ret = self.ty()?;
        let name = self.ident()?;
        if self.is_punct("(") {
            let params = self.params()?;
            self.throws()?;
            let body = self.block()?;
            return Ok(Member {
                name: name.clone(),
                line,
                kind: MemberKind::Method(Method { name, ret, params, body, is_static }),
            });
        }
        // A field: re-read it as a local declaration from its type.
        self.i = field_start;
        let decl = self.local()?;
        self.expect_punct(";")?;
        Ok(Member { name, line, kind: MemberKind::Field { decl, is_static, is_final } })
    }

    fn params(&mut self) -> R<Vec<(Type, String)>> {
        self.expect_punct("(")?;
        let mut ps = Vec::new();
        while !self.eat_punct(")") {
            if self.is_ident("final") {
                self.bump();
            }
            let mut ty = self.ty()?;
            let name = self.ident()?;
            while self.eat_punct("[") {
                self.expect_punct("]")?;
                ty.dims += 1;
            }
            ps.push((ty, name));
            self.eat_punct(",");
        }
        Ok(ps)
    }

    fn throws(&mut self) -> R<()> {
        if self.is_ident("throws") {
            self.bump();
            self.ty()?;
            while self.eat_punct(",") {
                self.ty()?;
            }
        }
        Ok(())
    }

    /// A whole token stream that is one type.
    pub(crate) fn type_only(&mut self) -> R<Type> {
        let t = self.ty()?;
        if *self.peek() != Tok::Eof {
            return Err(format!("trailing tokens after type: {:?}", self.peek()));
        }
        Ok(t)
    }

    /// `A.B.C<...>[][]`.
    fn ty(&mut self) -> R<Type> {
        let mut ty = self.ty_no_dims()?;
        while self.is_punct("[") && matches!(self.peek_at(1), Tok::Punct("]")) {
            self.bump();
            self.bump();
            ty.dims += 1;
        }
        Ok(ty)
    }

    /// `A.B.C<...>`.
    fn ty_no_dims(&mut self) -> R<Type> {
        let mut name = self.ident()?;
        while self.is_punct(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            name = format!("{name}.{}", self.ident()?);
        }
        let mut args = Vec::new();
        if self.eat_punct("<") {
            while !self.close_angle()? {
                if self.eat_punct("?") {
                    // `? extends T` is read as `T`.
                    if self.is_ident("extends") || self.is_ident("super") {
                        self.bump();
                        args.push(self.ty()?);
                    } else {
                        args.push(Type::named("Object"));
                    }
                } else {
                    args.push(self.ty()?);
                }
                self.eat_punct(",");
            }
        }
        Ok(Type { name, args, dims: 0 })
    }

    /// Consumes one `>` closing type arguments, splitting `>>` and `>>>`.
    fn close_angle(&mut self) -> R<bool> {
        if self.owed_gt > 0 {
            self.owed_gt -= 1;
            return Ok(true);
        }
        match self.peek() {
            Tok::Punct(">") => self.owed_gt = 0,
            Tok::Punct(">>") => self.owed_gt = 1,
            Tok::Punct(">>>") => self.owed_gt = 2,
            Tok::Eof => return Err("unterminated type arguments".into()),
            _ => return Ok(false),
        }
        self.bump();
        Ok(true)
    }

    /// Whether a local variable declaration starts here: `Type Ident (= | ; | , | [)`.
    fn at_local_decl(&mut self) -> bool {
        let Tok::Ident(first) = self.peek() else { return false };
        if ["return", "throw", "new", "this", "true", "false", "null"].contains(&first.as_str()) {
            return false;
        }
        let save = self.i;
        if self.is_ident("final") {
            self.bump();
        }
        let ok = self.ty().is_ok()
            && self.owed_gt == 0
            && matches!(self.peek(), Tok::Ident(_))
            && matches!(self.peek_at(1), Tok::Punct("=" | ";" | "," | "["));
        self.i = save;
        self.owed_gt = 0;
        ok
    }

    // ---- statements ----

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect_punct("{")?;
        let mut stmts = Vec::new();
        while !self.eat_punct("}") {
            stmts.push(self.stmt()?);
        }
        Ok(stmts)
    }

    fn local(&mut self) -> R<Stmt> {
        if self.is_ident("final") {
            self.bump();
        }
        let ty = self.ty()?;
        let mut vars = Vec::new();
        loop {
            let name = self.ident()?;
            // C-style `SyntaxTreeNode zn[]`.
            let mut extra_dims = 0;
            while self.eat_punct("[") {
                self.expect_punct("]")?;
                extra_dims += 1;
            }
            let init = if self.eat_punct("=") { Some(self.var_init()?) } else { None };
            vars.push(Var { name, extra_dims, init });
            if !self.eat_punct(",") {
                break;
            }
        }
        Ok(Stmt::Local { ty, vars })
    }

    fn var_init(&mut self) -> R<Expr> {
        if self.is_punct("{") { self.array_lit() } else { self.expr() }
    }

    fn array_lit(&mut self) -> R<Expr> {
        self.expect_punct("{")?;
        let mut items = Vec::new();
        while !self.eat_punct("}") {
            items.push(self.var_init()?);
            self.eat_punct(",");
        }
        Ok(Expr::ArrayLit(items))
    }

    fn stmt(&mut self) -> R<Stmt> {
        if self.is_punct("{") {
            return Ok(Stmt::Block(self.block()?));
        }
        if self.eat_punct(";") {
            return Ok(Stmt::Empty);
        }
        if let (Tok::Ident(label), Tok::Punct(":")) = (self.peek().clone(), self.peek_at(1).clone())
            && label != "default"
        {
            self.bump();
            self.bump();
            return Ok(Stmt::Labeled { label, body: Box::new(self.stmt()?) });
        }
        if let Tok::Ident(kw) = self.peek().clone() {
            match kw.as_str() {
                "if" => {
                    self.bump();
                    self.expect_punct("(")?;
                    let cond = self.expr()?;
                    self.expect_punct(")")?;
                    let then = Box::new(self.stmt()?);
                    let els = if self.is_ident("else") {
                        self.bump();
                        Some(Box::new(self.stmt()?))
                    } else {
                        None
                    };
                    return Ok(Stmt::If { cond, then, els });
                }
                "do" => {
                    self.bump();
                    let body = Box::new(self.stmt()?);
                    if !self.is_ident("while") {
                        return Err(format!("line {}: expected while after do body", self.line()));
                    }
                    self.bump();
                    self.expect_punct("(")?;
                    let cond = self.expr()?;
                    self.expect_punct(")")?;
                    self.expect_punct(";")?;
                    return Ok(Stmt::DoWhile { body, cond });
                }
                "while" => {
                    self.bump();
                    self.expect_punct("(")?;
                    let cond = self.expr()?;
                    self.expect_punct(")")?;
                    return Ok(Stmt::While { cond, body: Box::new(self.stmt()?) });
                }
                "for" => {
                    self.bump();
                    self.expect_punct("(")?;
                    // Enhanced for: `(final? Type name : expr)`.
                    let save = self.i;
                    if self.is_ident("final") {
                        self.bump();
                    }
                    if let Ok(ty) = self.ty()
                        && let Tok::Ident(name) = self.peek().clone()
                        && matches!(self.peek_at(1), Tok::Punct(":"))
                    {
                        self.bump();
                        self.bump();
                        let iter = self.expr()?;
                        self.expect_punct(")")?;
                        return Ok(Stmt::ForEach { ty, name, iter, body: Box::new(self.stmt()?) });
                    }
                    self.i = save;
                    self.owed_gt = 0;
                    let mut init = Vec::new();
                    if !self.is_punct(";") {
                        if self.at_local_decl() {
                            init.push(self.local()?);
                        } else {
                            init.push(Stmt::Expr(self.expr()?));
                            while self.eat_punct(",") {
                                init.push(Stmt::Expr(self.expr()?));
                            }
                        }
                    }
                    self.expect_punct(";")?;
                    let cond = if self.is_punct(";") { None } else { Some(self.expr()?) };
                    self.expect_punct(";")?;
                    let mut update = Vec::new();
                    while !self.is_punct(")") {
                        update.push(self.expr()?);
                        self.eat_punct(",");
                    }
                    self.expect_punct(")")?;
                    return Ok(Stmt::For { init, cond, update, body: Box::new(self.stmt()?) });
                }
                "switch" => {
                    self.bump();
                    self.expect_punct("(")?;
                    let scrutinee = self.expr()?;
                    self.expect_punct(")")?;
                    self.expect_punct("{")?;
                    let mut arms: Vec<SwitchArm> = Vec::new();
                    while !self.eat_punct("}") {
                        let mut arm = SwitchArm { labels: Vec::new(), is_default: false, body: Vec::new() };
                        loop {
                            if self.is_ident("case") {
                                self.bump();
                                arm.labels.push(self.expr()?);
                                self.expect_punct(":")?;
                            } else if self.is_ident("default") {
                                self.bump();
                                self.expect_punct(":")?;
                                arm.is_default = true;
                            } else {
                                break;
                            }
                        }
                        while !self.is_ident("case") && !self.is_ident("default") && !self.is_punct("}") {
                            arm.body.push(self.stmt()?);
                        }
                        arms.push(arm);
                    }
                    return Ok(Stmt::Switch { scrutinee, arms });
                }
                "break" | "continue" => {
                    self.bump();
                    let label = if let Tok::Ident(l) = self.peek().clone() {
                        self.bump();
                        Some(l)
                    } else {
                        None
                    };
                    self.expect_punct(";")?;
                    return Ok(if kw == "break" { Stmt::Break(label) } else { Stmt::Continue(label) });
                }
                "return" => {
                    self.bump();
                    let e = if self.is_punct(";") { None } else { Some(self.expr()?) };
                    self.expect_punct(";")?;
                    return Ok(Stmt::Return(e));
                }
                "throw" => {
                    self.bump();
                    let e = self.expr()?;
                    self.expect_punct(";")?;
                    return Ok(Stmt::Throw(e));
                }
                "try" => {
                    self.bump();
                    let body = self.block()?;
                    let mut catches = Vec::new();
                    while self.is_ident("catch") {
                        self.bump();
                        self.expect_punct("(")?;
                        let ty = self.ty()?;
                        let name = self.ident()?;
                        self.expect_punct(")")?;
                        catches.push((ty, name, self.block()?));
                    }
                    let finally = if self.is_ident("finally") {
                        self.bump();
                        Some(self.block()?)
                    } else {
                        None
                    };
                    return Ok(Stmt::Try { body, catches, finally });
                }
                _ => {}
            }
        }
        if self.at_local_decl() {
            let s = self.local()?;
            self.expect_punct(";")?;
            return Ok(s);
        }
        let e = self.expr()?;
        self.expect_punct(";")?;
        Ok(Stmt::Expr(e))
    }

    // ---- expressions (Java precedence) ----

    pub(crate) fn expr(&mut self) -> R<Expr> {
        let lhs = self.cond()?;
        for op in ["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", ">>>="] {
            if self.is_punct(op) {
                let Tok::Punct(op) = self.bump() else { unreachable!() };
                let rhs = self.var_init()?;
                return Ok(Expr::Assign(op, Box::new(lhs), Box::new(rhs)));
            }
        }
        Ok(lhs)
    }

    fn cond(&mut self) -> R<Expr> {
        let c = self.binary(0)?;
        if self.eat_punct("?") {
            let t = self.expr()?;
            self.expect_punct(":")?;
            let f = self.cond()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(t), Box::new(f)));
        }
        Ok(c)
    }

    fn binary(&mut self, level: usize) -> R<Expr> {
        const LEVELS: &[&[&str]] = &[
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!="],
            &["<", ">", "<=", ">="],
            &["<<", ">>", ">>>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        while let Some(op) = LEVELS[level].iter().find(|op| self.is_punct(op)) {
            self.bump();
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// `(Type) expr` where `Type` is a primitive, or a capitalized name possibly with `[]`.
    fn at_cast(&self) -> bool {
        if !self.is_punct("(") {
            return false;
        }
        let Tok::Ident(name) = self.peek_at(1) else { return false };
        let mut j = 2;
        while matches!((self.peek_at(j), self.peek_at(j + 1)), (Tok::Punct("."), Tok::Ident(_))) {
            j += 2;
        }
        let mut dims = 0;
        while matches!((self.peek_at(j), self.peek_at(j + 1)), (Tok::Punct("["), Tok::Punct("]"))) {
            j += 2;
            dims += 1;
        }
        if !matches!(self.peek_at(j), Tok::Punct(")")) {
            return false;
        }
        let primitive = PRIMITIVES.contains(&name.as_str());
        let class_like = name.chars().next().is_some_and(char::is_uppercase);
        let next_starts_operand = matches!(
            self.peek_at(j + 1),
            Tok::Ident(_) | Tok::Int(_) | Tok::Str(_) | Tok::Char(_) | Tok::Punct("(" | "!" | "~")
        );
        primitive || (class_like && (dims > 0 || next_starts_operand))
    }

    fn unary(&mut self) -> R<Expr> {
        for op in ["!", "-", "+", "~", "++", "--"] {
            if self.is_punct(op) {
                let Tok::Punct(op) = self.bump() else { unreachable!() };
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        if self.at_cast() {
            self.bump();
            let ty = self.ty()?;
            self.expect_punct(")")?;
            return Ok(Expr::Cast(ty, Box::new(self.unary()?)));
        }
        let mut e = self.primary()?;
        loop {
            if self.eat_punct(".") {
                let name = self.ident()?;
                if self.is_punct("(") {
                    let args = self.args()?;
                    e = Expr::Call { target: Some(Box::new(e)), name, args };
                } else {
                    e = Expr::Field(Box::new(e), name);
                }
            } else if self.eat_punct("[") {
                let idx = self.expr()?;
                self.expect_punct("]")?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.is_punct("++") || self.is_punct("--") {
                let Tok::Punct(op) = self.bump() else { unreachable!() };
                e = Expr::Postfix(op, Box::new(e));
            } else {
                return Ok(e);
            }
        }
    }

    fn args(&mut self) -> R<Vec<Expr>> {
        self.expect_punct("(")?;
        let mut args = Vec::new();
        while !self.eat_punct(")") {
            args.push(self.expr()?);
            if !self.is_punct(")") {
                self.expect_punct(",")?;
            }
        }
        Ok(args)
    }

    fn primary(&mut self) -> R<Expr> {
        let line = self.line();
        match self.bump() {
            Tok::Int(s) => Ok(Expr::Int(s)),
            Tok::Char(s) => Ok(Expr::Char(s)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Punct("(") => {
                let e = self.expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Ident(name) => match name.as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "null" => Ok(Expr::Null),
                "this" | "super" if self.is_punct("(") => Ok(Expr::Call { target: None, name, args: self.args()? }),
                "this" => Ok(Expr::This),
                "new" => {
                    let ty = self.ty_no_dims()?;
                    if self.is_punct("[") {
                        let mut dims = 0;
                        let mut len = None;
                        while self.eat_punct("[") {
                            if !self.is_punct("]") {
                                len = Some(Box::new(self.expr()?));
                            }
                            self.expect_punct("]")?;
                            dims += 1;
                        }
                        let init = if self.is_punct("{") {
                            match self.array_lit()? {
                                Expr::ArrayLit(items) => Some(items),
                                _ => unreachable!(),
                            }
                        } else {
                            None
                        };
                        return Ok(Expr::NewArray { ty: Type { dims, ..ty }, len, init });
                    }
                    let args = self.args()?;
                    if self.is_punct("{") {
                        return Err(format!("line {line}: anonymous class"));
                    }
                    Ok(Expr::New { ty, args })
                }
                _ if self.is_punct("(") => Ok(Expr::Call { target: None, name, args: self.args()? }),
                _ => Ok(Expr::Name(name)),
            },
            t => Err(format!("line {line}: unexpected {t:?} in expression")),
        }
    }

}
