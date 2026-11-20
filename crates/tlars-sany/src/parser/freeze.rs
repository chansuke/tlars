//! Copies the translated semantic graph (`ExternalModuleTable` and the nodes it reaches,
//! `Rc<RefCell<_>>` objects) into the immutable arenas of [`crate::graph::SemanticGraph`].
//! Each SANY node gets one id, by identity, so sharing in SANY's graph (an `OpDefNode`
//! referenced from many applications, `I!Op` definitions cloned by INSTANCE) is sharing of
//! ids here.

use super::generated::{ExternalModuleTable, SemanticNodeObj, Subst};
use super::java::*;
use super::support::{Location, UniqueString};
use crate::graph::{self, ExprId, ExprOrOpArg, GraphParts, Level, Loc, ModuleId, SymbolRef};
use crate::{Atom, Interner};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A node's identity: the address of its shared cell.
fn key(n: &J<SemanticNodeObj>) -> usize {
    n.ptr_key()
}

struct Freezer {
    parts: GraphParts,
    /// Modules by id; `None` until `fill_module` (modules refer to each other).
    module_slots: Vec<Option<graph::ModuleNode>>,
    /// Expressions by id; an id is taken before the children are copied, because `@`
    /// (`AtNode`) refers to the EXCEPT that contains it.
    expr_slots: Vec<Option<graph::ExprNode>>,
    modules: BTreeMap<usize, ModuleId>,
    op_defs: BTreeMap<usize, graph::OpDefId>,
    decls: BTreeMap<usize, graph::DeclId>,
    params: BTreeMap<usize, graph::ParamId>,
    thms: BTreeMap<usize, graph::ThmId>,
    exprs: BTreeMap<usize, ExprId>,
    /// Definitions whose bodies are copied after the modules (bodies refer to anything).
    pending_defs: Vec<(J<SemanticNodeObj>, graph::OpDefId)>,
    pending_thms: Vec<(J<SemanticNodeObj>, graph::ThmId)>,
    pending_modules: Vec<(J<SemanticNodeObj>, ModuleId)>,
}

fn atom(u: UniqueString) -> Atom {
    u.as_atom().expect("a name")
}

fn level(l: i32) -> Level {
    match l {
        0 => Level::Constant,
        1 => Level::State,
        2 => Level::Action,
        3 => Level::Temporal,
        other => panic!("level {other}"),
    }
}

fn loc(l: &J<Location>) -> Loc {
    Loc {
        module: l.sourceAsUniqueString().as_atom(),
        begin_line: l.beginLine(),
        begin_col: l.beginColumn(),
        end_line: l.endLine(),
        end_col: l.endColumn(),
    }
}

fn class(n: &J<SemanticNodeObj>) -> &'static str {
    n.jread(|o| o.jclass_name())
}

fn nodes(a: JArr<J<SemanticNodeObj>>) -> Vec<J<SemanticNodeObj>> {
    if a.is_null() { Vec::new() } else { a.jget().clone() }
}

impl Freezer {
    fn module(&mut self, m: &J<SemanticNodeObj>) -> ModuleId {
        if let Some(id) = self.modules.get(&key(m)) {
            return *id;
        }
        let id = ModuleId::new(self.module_slots.len() as u32);
        self.modules.insert(key(m), id);
        self.module_slots.push(None);
        self.pending_modules.push((m.clone(), id));
        id
    }

    fn fill_module(&mut self, m: &J<SemanticNodeObj>, id: ModuleId) -> JResult<()> {
        let extends: Vec<ModuleId> = nodes(m.jread(|o| o.ModuleNode_extendees().clone())).iter().map(|e| self.module(e)).collect();
        let constant_decls = nodes(m.getConstantDecls()?).iter().map(|d| self.decl(d)).collect::<JResult<_>>()?;
        let variable_decls = nodes(m.getVariableDecls()?).iter().map(|d| self.decl(d)).collect::<JResult<_>>()?;
        let op_defs = nodes(m.getOpDefs()?).iter().map(|d| self.op_def(d)).collect::<JResult<_>>()?;
        let mut assumptions = Vec::new();
        for a in nodes(m.getAssumptions()?) {
            let def = a.getDef_0_via_AssumeNode()?;
            let def = if def.is_null() { None } else { Some(self.thm(&def)?) };
            assumptions.push(graph::AssumeNode {
                expr: self.expr(&a.getAssume()?)?,
                is_axiom: a.getIsAxiom()?,
                def,
                loc: loc(&a.getLocation_0_via_SemanticNode()?),
            });
        }
        let inner_modules = nodes(m.getInnerModules()?).iter().map(|i| self.module(i)).collect();
        // `processConstants` visits a theorem only when it is an expression, not an
        // `ASSUME ... PROVE` (`SpecProcessor.java:2083-2087`).
        let mut theorems = Vec::new();
        for t in nodes(m.getTheorems()?) {
            let body = t.getTheorem()?;
            if body.jread(|o| o.is_instance_of("ExprNode")) {
                theorems.push(self.expr(&body)?);
            }
        }
        self.module_slots[id.index()] = Some(graph::ModuleNode {
            name: atom(m.getName_0_via_SymbolNode()?),
            loc: loc(&m.getLocation_0_via_SemanticNode()?),
            extends: extends.into(),
            constant_decls,
            variable_decls,
            op_defs,
            assumptions: assumptions.into(),
            inner_modules,
            theorems: theorems.into(),
            is_standard: m.isStandardModule_0_via_SemanticNode()?,
            is_instantiated: m.isInstantiated()?,
            parameter_free: m.isParameterFree()?,
        });
        Ok(())
    }

    fn decl(&mut self, d: &J<SemanticNodeObj>) -> JResult<graph::DeclId> {
        if let Some(id) = self.decls.get(&key(d)) {
            return Ok(*id);
        }
        let kind = match d.jread(|o| *o.SemanticNode_kind()) {
            2 => graph::DeclKind::Constant,
            3 => graph::DeclKind::Variable,
            24 => graph::DeclKind::NewConstant,
            25 => graph::DeclKind::NewVariable,
            other => panic!("OpDeclNode kind {other}"),
        };
        let module = d.getOriginallyDefinedInModuleNode_0_via_OpDefOrDeclNode()?;
        let module = self.module(&module);
        let id = graph::DeclId::new(self.parts.decls.len() as u32);
        self.decls.insert(key(d), id);
        self.parts.decls.push(graph::OpDeclNode {
            name: atom(d.getName_0_via_SymbolNode()?),
            kind,
            arity: d.getArity_0_via_SymbolNode()?,
            module,
            loc: loc(&d.getLocation_0_via_SemanticNode()?),
        });
        Ok(id)
    }

    fn param(&mut self, p: &J<SemanticNodeObj>) -> JResult<graph::ParamId> {
        if let Some(id) = self.params.get(&key(p)) {
            return Ok(*id);
        }
        let id = graph::ParamId::new(self.parts.params.len() as u32);
        self.params.insert(key(p), id);
        self.parts.params.push(graph::FormalParamNode {
            name: atom(p.getName_0_via_SymbolNode()?),
            arity: p.getArity_0_via_SymbolNode()?,
            loc: loc(&p.getLocation_0_via_SemanticNode()?),
        });
        Ok(id)
    }

    fn op_def(&mut self, d: &J<SemanticNodeObj>) -> JResult<graph::OpDefId> {
        if let Some(id) = self.op_defs.get(&key(d)) {
            return Ok(*id);
        }
        let kind = match d.jread(|o| *o.SemanticNode_kind()) {
            5 => graph::OpDefKind::User,
            6 => graph::OpDefKind::ModuleInstance,
            7 => graph::OpDefKind::BuiltIn,
            other => panic!("OpDefNode kind {other}"),
        };
        let id = graph::OpDefId::new(self.parts.op_defs.len() as u32);
        self.op_defs.insert(key(d), id);
        let params = nodes(d.getParams()?).iter().map(|p| self.param(p)).collect::<JResult<_>>()?;
        let module = d.getOriginallyDefinedInModuleNode_0_via_OpDefOrDeclNode()?;
        let module = if module.is_null() { ModuleId::new(u32::MAX) } else { self.module(&module) };
        self.parts.op_defs.push(graph::OpDefNode {
            name: atom(d.getName_0_via_SymbolNode()?),
            kind,
            params,
            body: None,
            module,
            loc: loc(&d.getLocation_0_via_SemanticNode()?),
            local: d.isLocal_0_via_SymbolNode()?,
            recursive: d.getInRecursive()?,
            instantiated_from: None,
        });
        self.parts.op_levels.push(level(d.getLevel()?));
        // After this definition has its slot: the source is another definition.
        let source = d.getSource__OpDefNode()?;
        if !source.is_null() && !source.same(d) {
            let from = self.op_def(&source)?;
            self.parts.op_defs[id.index()].instantiated_from = Some(from);
        }
        self.pending_defs.push((d.clone(), id));
        Ok(id)
    }

    fn thm(&mut self, t: &J<SemanticNodeObj>) -> JResult<graph::ThmId> {
        if let Some(id) = self.thms.get(&key(t)) {
            return Ok(*id);
        }
        let id = graph::ThmId::new(self.parts.thms.len() as u32);
        self.thms.insert(key(t), id);
        let params = nodes(t.getParams()?).iter().map(|p| self.param(p)).collect::<JResult<_>>()?;
        self.parts.thms.push(graph::ThmOrAssumpDefNode { name: atom(t.getName_0_via_SymbolNode()?), body: ExprId::new(u32::MAX), params });
        self.pending_thms.push((t.clone(), id));
        Ok(id)
    }

    fn symbol(&mut self, s: &J<SemanticNodeObj>) -> JResult<SymbolRef> {
        Ok(match class(s) {
            "OpDefNode" => SymbolRef::OpDef(self.op_def(s)?),
            "OpDeclNode" => SymbolRef::Decl(self.decl(s)?),
            "FormalParamNode" => SymbolRef::Param(self.param(s)?),
            "ThmOrAssumpDefNode" => SymbolRef::Thm(self.thm(s)?),
            "ModuleNode" => SymbolRef::Module(self.module(s)),
            other => panic!("symbol {other}"),
        })
    }

    fn expr_or_op_arg(&mut self, e: &J<SemanticNodeObj>) -> JResult<ExprOrOpArg> {
        if e.is_null() {
            return Ok(ExprOrOpArg::Absent);
        }
        if class(e) == "OpArgNode" {
            return Ok(ExprOrOpArg::OpArg(self.symbol(&e.getOp()?)?));
        }
        Ok(ExprOrOpArg::Expr(self.expr(e)?))
    }

    fn substs(&mut self, substs: JArr<J<Subst>>) -> JResult<Box<[graph::Subst]>> {
        let substs: Vec<J<Subst>> = if substs.is_null() { Vec::new() } else { substs.jget().clone() };
        substs
            .iter()
            .map(|s| Ok(graph::Subst { decl: self.decl(&s.getOp()?)?, expr: self.expr_or_op_arg(&s.getExpr()?)?, implicit: s.isImplicit()? }))
            .collect()
    }

    fn expr(&mut self, e: &J<SemanticNodeObj>) -> JResult<ExprId> {
        if let Some(id) = self.exprs.get(&key(e)) {
            return Ok(*id);
        }
        let id = ExprId::new(self.expr_slots.len() as u32);
        self.exprs.insert(key(e), id);
        self.expr_slots.push(None);
        self.parts.expr_levels.push(level(e.getLevel()?));
        let l = loc(&e.getLocation_0_via_SemanticNode()?);
        let node = match class(e) {
            "OpApplNode" => {
                let op = self.symbol(&e.getOperator()?)?;
                let args = nodes(e.getArgs()?).iter().map(|a| self.expr_or_op_arg(a)).collect::<JResult<_>>()?;
                let lists = e.getBdedQuantSymbolLists_0_via_OpApplNode()?;
                let unbounded = nodes(e.getUnbdedQuantSymbols()?);
                let bound = if lists.is_null() && unbounded.is_empty() {
                    None
                } else {
                    let lists: Vec<JArr<J<SemanticNodeObj>>> = if lists.is_null() { Vec::new() } else { lists.jget().clone() };
                    let tuples = e.isBdedQuantATuple()?;
                    let ranges = nodes(e.getBdedQuantBounds()?);
                    let mut out = Vec::new();
                    for (i, list) in lists.into_iter().enumerate() {
                        out.push(graph::BoundList {
                            vars: nodes(list).iter().map(|v| self.param(v)).collect::<JResult<_>>()?,
                            tuple: tuples.at(i as i32),
                            domain: self.expr(&ranges[i])?,
                        });
                    }
                    let unbounded = unbounded.iter().map(|v| self.param(v)).collect::<JResult<_>>()?;
                    Some(Box::new(graph::BoundSymbols { lists: out.into(), unbounded }))
                };
                graph::ExprNode::OpAppl { op, args, bound, loc: l }
            }
            "LetInNode" => {
                let defs = nodes(e.getLets()?).iter().map(|d| self.op_def(d)).collect::<JResult<_>>()?;
                graph::ExprNode::LetIn { defs, body: self.expr(&e.getBody_0_via_LetInNode()?)?, loc: l }
            }
            "SubstInNode" => graph::ExprNode::SubstIn {
                substs: self.substs(e.getSubsts_0_via_SubstInNode()?)?,
                body: self.expr(&e.getBody_0_via_SubstInNode()?)?,
                instantiating: self.module(&e.getInstantiatingModule_0_via_SubstInNode()?),
                instantiated: self.module(&e.getInstantiatedModule_0_via_SubstInNode()?),
                loc: l,
            },
            "APSubstInNode" => graph::ExprNode::APSubstIn {
                substs: self.substs(e.getSubsts_0_via_APSubstInNode()?)?,
                body: self.expr(&e.getBody_0_via_APSubstInNode()?)?,
                loc: l,
            },
            "LabelNode" => graph::ExprNode::Label {
                name: atom(e.getName_0_via_LabelNode()?),
                params: nodes(e.jread(|o| o.LabelNode_params().clone())).iter().map(|p| self.param(p)).collect::<JResult<_>>()?,
                body: self.expr(&e.getBody_0_via_LabelNode()?)?,
                loc: l,
            },
            "AtNode" => graph::ExprNode::At {
                except_ref: self.expr(&e.getExceptRef()?)?,
                except_component: self.expr(&e.getExceptComponentRef()?)?,
                loc: l,
            },
            "NumeralNode" => graph::ExprNode::Numeral { digits: e.java_string().into(), loc: l },
            "DecimalNode" => graph::ExprNode::Decimal { text: e.java_string().into(), loc: l },
            "StringNode" => graph::ExprNode::String { value: atom(e.getRep()?), loc: l },
            other => panic!("expression {other}"),
        };
        self.expr_slots[id.index()] = Some(node);
        Ok(id)
    }

    /// `Tool.getSymmetryPerms` names an offending symmetry set with
    /// `((SyntaxTreeNode) node.getTreeNode()).getHumanReadableImage()` on the arguments of
    /// the SYMMETRY definition's body (`Tool.java:3860, 3923-3925`). The syntax tree goes
    /// away with the parse, so the image for just those nodes is copied out now.
    fn record_body_arg_images(&mut self, body: &J<SemanticNodeObj>) -> JResult<()> {
        if class(body) != "OpApplNode" {
            return Ok(());
        }
        for arg in nodes(body.getArgs()?).iter() {
            if arg.is_null() {
                continue;
            }
            let id = self.expr_or_op_arg(arg)?;
            let ExprOrOpArg::Expr(id) = id else { continue };
            let image = arg.getTreeNode__SemanticNode()?.getHumanReadableImage()?;
            self.parts.body_arg_images.insert(id, Arc::from(image.java_string().as_str()));
        }
        Ok(())
    }

    /// Copies bodies until no definition, theorem or module is left half-done.
    fn drain(&mut self) -> JResult<()> {
        loop {
            if let Some((m, id)) = self.pending_modules.pop() {
                self.fill_module(&m, id)?;
            } else if let Some((d, id)) = self.pending_defs.pop() {
                let body = d.getBody_0_via_OpDefNode()?;
                if !body.is_null() {
                    let b = self.expr(&body)?;
                    self.parts.op_defs[id.index()].body = Some(b);
                    self.record_body_arg_images(&body)?;
                }
            } else if let Some((t, id)) = self.pending_thms.pop() {
                let body = t.getBody_0_via_ThmOrAssumpDefNode()?;
                let b = self.expr(&body)?;
                self.parts.thms[id.index()].body = b;
            } else {
                return Ok(());
            }
        }
    }
}

/// The graph of every module in `table`, rooted at its root module.
pub(crate) fn freeze(table: &J<ExternalModuleTable>, interner: Arc<Interner>) -> JResult<graph::SemanticGraph> {
    let mut f = Freezer {
        parts: GraphParts::default(),
        module_slots: Vec::new(),
        expr_slots: Vec::new(),
        modules: BTreeMap::new(),
        op_defs: BTreeMap::new(),
        decls: BTreeMap::new(),
        params: BTreeMap::new(),
        thms: BTreeMap::new(),
        exprs: BTreeMap::new(),
        pending_defs: Vec::new(),
        pending_thms: Vec::new(),
        pending_modules: Vec::new(),
    };
    for m in nodes(table.getModuleNodes()?) {
        f.module(&m);
    }
    let root = f.module(&table.getRootModule()?);
    f.drain()?;
    f.parts.modules = f.module_slots.into_iter().map(|m| m.expect("every module filled")).collect();
    f.parts.exprs = f.expr_slots.into_iter().map(|e| e.expect("every expression filled")).collect();
    Ok(graph::SemanticGraph::from_parts(f.parts, root, interner))
}
