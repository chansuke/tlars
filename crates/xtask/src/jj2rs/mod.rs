//! `xtask jj2rs`: translate the JavaCC-generated SANY parser (`TLAplusParser.java`,
//! `TLAplusParserTokenManager.java`) to Rust.
//!
//! `jj2rs survey <File.java>` parses every class member and reports what it could not read.
//! `jj2rs emit` translates the parser, its lexer and SANY's parser support classes into
//! `crates/tlars-sany/src/parser/generated.rs` and lists the constructs it could not emit;
//! `jj2rs emit --check` fails if that file is not what emission would write.

mod ast;
mod emit;
mod externs;
mod hier;
mod lex;
mod parse;

use parse::MemberKind;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Finds `class|interface|enum <name>` and returns the token index of the keyword, and
/// whether the declaration is `abstract`.
fn find_class(toks: &[lex::Token], name: &str) -> Option<(usize, bool)> {
    let is_decl = |t: &lex::Tok| matches!(t, lex::Tok::Ident(k) if k == "class" || k == "interface" || k == "enum");
    let pos = toks.windows(2).position(|w| is_decl(&w[0].tok) && w[1].tok == lex::Tok::Ident(name.into()))?;
    let is_abstract = toks[..pos].iter().rev().take(4).any(|t| t.tok == lex::Tok::Ident("abstract".into()));
    Some((pos, is_abstract))
}

/// Parses class `name` from `src`: its header and members.
fn read_class(src: &str, name: &str) -> Result<(ast::ClassHeader, Vec<parse::Member>), String> {
    let toks = lex::lex(src)?;
    let (start, is_abstract) = find_class(&toks, name).ok_or_else(|| format!("class {name} not found"))?;
    parse::Parser::new(&toks[start..]).class_decl(is_abstract)
}

pub(crate) fn survey(file: &Path) -> Result<(), String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let class = file.file_stem().and_then(|s| s.to_str()).ok_or("bad file name")?;
    let (_, members) = read_class(&src, class)?;
    let (mut methods, mut statements, mut unparsed) = (0, 0, Vec::new());
    let mut others: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for m in &members {
        match &m.kind {
            MemberKind::Method(method) | MemberKind::Constructor(method) => {
                methods += 1;
                statements += method.body.len();
            }
            MemberKind::Field { .. } => *others.entry("field").or_default() += 1,
            MemberKind::Class { .. } => *others.entry("class").or_default() += 1,
            MemberKind::StaticInit(_) => *others.entry("static init").or_default() += 1,
            MemberKind::Other(kind) => *others.entry(kind).or_default() += 1,
            MemberKind::Unparsed(e) => unparsed.push(format!("  {} (line {}): {e}", m.name, m.line)),
        }
    }
    println!(
        "{}: {methods} methods ({statements} top-level statements), other members {others:?}, {} unparsed",
        file.display(),
        unparsed.len()
    );
    for u in &unparsed {
        println!("{u}");
    }
    Ok(())
}

/// Translated classes: `(class, file under TLC's src)`. Superclasses and interfaces come
/// from the class declarations. The JavaCC-generated files are checked into TLC's tree
/// (identical to `javacc` output), so everything is read from the pinned source.
const TRANSLATED: &[(&str, &str)] = &[
    ("TLAplusParserConstants", "tla2sany/parser/TLAplusParserConstants.java"),
    ("SyntaxTreeConstants", "tla2sany/st/SyntaxTreeConstants.java"),
    ("Token", "tla2sany/parser/Token.java"),
    ("ParseException", "tla2sany/parser/ParseException.java"),
    ("ParseExceptionExtended", "tla2sany/parser/ParseExceptionExtended.java"),
    ("TokenMgrError", "tla2sany/parser/TokenMgrError.java"),
    ("SimpleCharStream", "tla2sany/parser/SimpleCharStream.java"),
    ("TLAplusParserTokenManager", "tla2sany/parser/TLAplusParserTokenManager.java"),
    ("SyntaxTreeNode", "tla2sany/parser/SyntaxTreeNode.java"),
    ("Operator", "tla2sany/parser/Operator.java"),
    ("Operators", "tla2sany/parser/Operators.java"),
    ("OSelement", "tla2sany/parser/OSelement.java"),
    ("OperatorStack", "tla2sany/parser/OperatorStack.java"),
    ("ParseError", "tla2sany/parser/ParseError.java"),
    ("ParseErrors", "tla2sany/parser/ParseErrors.java"),
    ("Strings", "tla2sany/utilities/Strings.java"),
    ("TLAplusParser", "tla2sany/parser/TLAplusParser.java"),
    // SANY's semantic analysis and level checking.
    ("APSubstInNode", "tla2sany/semantic/APSubstInNode.java"),
    ("ASTConstants", "tla2sany/semantic/ASTConstants.java"),
    ("AbortException", "tla2sany/semantic/AbortException.java"),
    ("AnyDefNode", "tla2sany/semantic/AnyDefNode.java"),
    ("ArgLevelParam", "tla2sany/semantic/ArgLevelParam.java"),
    ("AssumeNode", "tla2sany/semantic/AssumeNode.java"),
    ("AssumeProveNode", "tla2sany/semantic/AssumeProveNode.java"),
    ("AtNode", "tla2sany/semantic/AtNode.java"),
    ("BuiltInOperators", "tla2sany/semantic/BuiltInOperators.java"),
    ("Context", "tla2sany/semantic/Context.java"),
    ("DecimalNode", "tla2sany/semantic/DecimalNode.java"),
    ("DefStepNode", "tla2sany/semantic/DefStepNode.java"),
    ("Errors", "tla2sany/semantic/Errors.java"),
    ("ExprNode", "tla2sany/semantic/ExprNode.java"),
    ("ExprOrOpArgNode", "tla2sany/semantic/ExprOrOpArgNode.java"),
    ("ExternalModuleTable", "tla2sany/semantic/ExternalModuleTable.java"),
    ("FormalParamNode", "tla2sany/semantic/FormalParamNode.java"),
    ("FrontEnd", "tla2sany/semantic/FrontEnd.java"),
    ("Generator", "tla2sany/semantic/Generator.java"),
    ("InstanceNode", "tla2sany/semantic/InstanceNode.java"),
    ("LabelNode", "tla2sany/semantic/LabelNode.java"),
    ("LeafProofNode", "tla2sany/semantic/LeafProofNode.java"),
    ("LetInNode", "tla2sany/semantic/LetInNode.java"),
    ("LevelConstants", "tla2sany/semantic/LevelConstants.java"),
    ("LevelException", "tla2sany/semantic/LevelException.java"),
    ("LevelNode", "tla2sany/semantic/LevelNode.java"),
    ("ModuleNode", "tla2sany/semantic/ModuleNode.java"),
    ("NewSymbNode", "tla2sany/semantic/NewSymbNode.java"),
    ("NonLeafProofNode", "tla2sany/semantic/NonLeafProofNode.java"),
    ("NumeralNode", "tla2sany/semantic/NumeralNode.java"),
    ("OpApplNode", "tla2sany/semantic/OpApplNode.java"),
    ("OpArgNode", "tla2sany/semantic/OpArgNode.java"),
    ("OpDeclNode", "tla2sany/semantic/OpDeclNode.java"),
    ("OpDefNode", "tla2sany/semantic/OpDefNode.java"),
    ("OpDefOrDeclNode", "tla2sany/semantic/OpDefOrDeclNode.java"),
    ("OpDefOrLabelNode", "tla2sany/semantic/OpDefOrLabelNode.java"),
    ("ParamAndPosition", "tla2sany/semantic/ParamAndPosition.java"),
    ("ProofNode", "tla2sany/semantic/ProofNode.java"),
    ("SemanticNode", "tla2sany/semantic/SemanticNode.java"),
    ("StandardModules", "tla2sany/semantic/StandardModules.java"),
    ("StringNode", "tla2sany/semantic/StringNode.java"),
    ("Subst", "tla2sany/semantic/Subst.java"),
    ("SubstInNode", "tla2sany/semantic/SubstInNode.java"),
    ("SymbolMatcher", "tla2sany/semantic/SymbolMatcher.java"),
    ("SymbolNode", "tla2sany/semantic/SymbolNode.java"),
    ("SymbolTable", "tla2sany/semantic/SymbolTable.java"),
    ("TheoremNode", "tla2sany/semantic/TheoremNode.java"),
    ("ThmOrAssumpDefNode", "tla2sany/semantic/ThmOrAssumpDefNode.java"),
    ("UseOrHideNode", "tla2sany/semantic/UseOrHideNode.java"),
];

/// Members implemented by hand in `tlars-sany/src/parser/support.rs`, by signature key.
/// They stay callable from translated code; only their emission is skipped.
const HAND_WRITTEN: &[(&str, &str)] = &[
    // Reads through a `java.io.InputStream`; the hand-written entry point decodes the
    // source and uses the `Reader` constructor.
    ("TLAplusParser", "<init>(SanyOutput,byte[])"),
    // `catch (Throwable t)` around `System.arraycopy`.
    ("SimpleCharStream", "ExpandBuff(boolean)"),
    // Anonymous visitor classes; used by the linter and TLC, not by semantic analysis.
    ("SemanticNode", "walkChildren(ChildrenVisitor)"),
    ("SemanticNode", "isDefinedWith(SemanticNode)"),
    ("SemanticNode", "substituteFor(OpDefNode,OpDefNode)"),
];

/// Edits to the Java sources before translation, each an exact text that must occur once in
/// its file: stream pipelines and lambdas rewritten as the loops they mean, and a parameter
/// typed by an interface the translation does not model. `(file, before, after, why)`.
const PATCHES: &[(&str, &str, &str, &str)] = &[
    (
        "tla2sany/semantic/ModuleNode.java",
        "return Stream.of(getOpDefs()).filter(o -> o.getName().equals(name)).findFirst().orElse(null);",
        "for (OpDefNode o : getOpDefs()) { if (o.getName().equals(name)) { return o; } } return null;",
        "stream pipeline",
    ),
    (
        "tla2sany/semantic/SemanticNode.java",
        "return Arrays.asList(children).stream().filter(c -> c != null).collect(Collectors.toList());",
        "final List<SemanticNode> out = new ArrayList<>(); for (SemanticNode c : children) { if (c != null) { out.add(c); } } return out;",
        "stream pipeline",
    ),
    (
        "tla2sany/semantic/Generator.java",
        "final UniqueString[] cID = Stream.of(new UniqueString[] { name }, odn.getCompoundId())\n\t\t\t\t\t\t\t\t.flatMap(Stream::of).toArray(UniqueString[]::new);",
        "final UniqueString[] cIDTail = odn.getCompoundId(); final UniqueString[] cID = new UniqueString[cIDTail.length + 1]; cID[0] = name; System.arraycopy(cIDTail, 0, cID, 1, cIDTail.length);",
        "stream pipeline",
    ),
    (
        "tla2sany/semantic/Errors.java",
        "return this.messages.stream().filter(\n        msg -> msg.getCode().getSeverityLevel().equals(level)\n      ).collect(Collectors.toList());",
        "final List<ErrorDetails> out = new ArrayList<ErrorDetails>(); for (ErrorDetails msg : this.messages) { if (msg.getCode().getSeverityLevel().equals(level)) { out.add(msg); } } return out;",
        "stream pipeline",
    ),
    (
        "tla2sany/semantic/Errors.java",
        "return this.getErrorDetails().stream()\n        .map(ErrorDetails::toString).toArray(String[]::new);",
        "final List<ErrorDetails> ds = this.getErrorDetails(); final String[] out = new String[ds.size()]; for (int i = 0; i < out.length; i++) { out[i] = ds.get(i).toString(); } return out;",
        "stream pipeline",
    ),
    (
        "tla2sany/semantic/Errors.java",
        "return this.getWarningDetails().stream()\n        .map(ErrorDetails::toString).toArray(String[]::new);",
        "final List<ErrorDetails> ds = this.getWarningDetails(); final String[] out = new String[ds.size()]; for (int i = 0; i < out.length; i++) { out[i] = ds.get(i).toString(); } return out;",
        "stream pipeline",
    ),
    (
        "tla2sany/semantic/Errors.java",
        "return List.of(this.parameters);",
        "final List<Object> out = new ArrayList<Object>(); for (Object o : this.parameters) { out.add(o); } return out;",
        "List.of",
    ),
    (
        "tla2sany/semantic/Context.java",
        "public static boolean isBuiltIn(final ExploreNode exploreNode) {",
        "public static boolean isBuiltIn(final SemanticNode exploreNode) {",
        "ExploreNode is not modelled; the only caller passes a SemanticNode",
    ),
    (
        "tla2sany/semantic/Context.java",
        "public <T> Vector<T> getByClass(Class<T> template) {\n    final Vector<T> result = new Vector<>();",
        "public Vector<SymbolNode> getByClass(Class<?> template) {\n    final Vector<SymbolNode> result = new Vector<>();",
        "generic method; every SymbolNode class is one Rust type, so callers are unchanged",
    ),
    (
        "tla2sany/semantic/Context.java",
        "result.addElement(template.cast(elt.info));",
        "result.addElement(elt.info);",
        "Class.cast after isInstance is the identity",
    ),
    (
        "tla2sany/semantic/InstanceNode.java",
        "    iter = lcSet.keySet().iterator();\n    while (iter.hasNext()) {\n      SymbolNode param = (SymbolNode)iter.next();",
        "    Iterator lcIter = lcSet.keySet().iterator();\n    while (lcIter.hasNext()) {\n      SymbolNode param = (SymbolNode)lcIter.next();",
        "a raw Iterator reused for different element types",
    ),
    (
        "tla2sany/semantic/InstanceNode.java",
        "      iter = lcSet.keySet().iterator();\n      while (iter.hasNext()) {\n        SymbolNode param = (SymbolNode)iter.next();",
        "      Iterator lcIter = lcSet.keySet().iterator();\n      while (lcIter.hasNext()) {\n        SymbolNode param = (SymbolNode)lcIter.next();",
        "a raw Iterator reused for different element types",
    ),
    (
        "tla2sany/semantic/InstanceNode.java",
        "    iter = alcSet.keySet().iterator();\n    while (iter.hasNext()) {\n      ParamAndPosition pap = (ParamAndPosition)iter.next();",
        "    Iterator alcIter = alcSet.keySet().iterator();\n    while (alcIter.hasNext()) {\n      ParamAndPosition pap = (ParamAndPosition)alcIter.next();",
        "a raw Iterator reused for different element types",
    ),
    (
        "tla2sany/semantic/InstanceNode.java",
        "      iter = alcSet.keySet().iterator();\n      while (iter.hasNext()) {\n        ParamAndPosition pap = (ParamAndPosition)iter.next();",
        "      Iterator alcIter = alcSet.keySet().iterator();\n      while (alcIter.hasNext()) {\n        ParamAndPosition pap = (ParamAndPosition)alcIter.next();",
        "a raw Iterator reused for different element types",
    ),
    (
        "tla2sany/semantic/InstanceNode.java",
        "public final HashSet getLevelParams() {",
        "public final HashSet<SymbolNode> getLevelParams() {",
        "raw type; it overrides LevelNode.getLevelParams()",
    ),
    (
        "tla2sany/semantic/Generator.java",
        "HashSet opParams = new HashSet();",
        "HashSet<SymbolNode> opParams = new HashSet<>();",
        "raw type; it holds FormalParamNodes",
    ),
    (
        "tla2sany/semantic/ModuleNode.java",
        "Collections.sort(result);",
        "for (int i = 1; i < result.size(); i++) { final SymbolNode x = result.get(i); int j = i - 1; while (j >= 0 && result.get(j).compareTo(x) > 0) { result.set(j + 1, result.get(j)); j--; } result.set(j + 1, x); }",
        "Collections.sort is stable; so is this insertion sort",
    ),
];

/// The source of `file` with its [`PATCHES`] applied.
fn patched_source(tlc_src: &Path, file: &str) -> Result<String, String> {
    let path = tlc_src.join(file);
    let mut src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    for (f, before, after, why) in PATCHES.iter().filter(|p| p.0 == file) {
        match src.matches(before).count() {
            1 => src = src.replacen(before, after, 1),
            n => return Err(format!("{f}: patch ({why}) matches {n} times: {before:?}")),
        }
    }
    Ok(src)
}

/// Raw generic fields and the element type their uses imply (every read casts to it).
const RAW_OVERRIDES: &[(&str, &str, &str)] = &[
    ("ParseErrors", "loe", "Vector<ParseError>"),
    ("TLAplusParser", "jj_expentries", "Vector<int[]>"),
];

fn parse_class(tlc_src: &Path, file: &str, class: &str) -> Result<(ast::ClassHeader, Vec<parse::Member>), String> {
    let src = patched_source(tlc_src, file)?;
    let (header, members) = read_class(&src, class).map_err(|e| format!("{file}: {e}"))?;
    for m in &members {
        if let MemberKind::Unparsed(e) = &m.kind {
            return Err(format!("{class}.{}: {e}", m.name));
        }
    }
    Ok((header, members))
}

/// Nested classes (`JJCalls`, `Context.Pair`, `Generator.Function.pair`) become top-level
/// classes after their enclosing class, remembering it.
fn flatten(
    outer: Option<&str>,
    header: ast::ClassHeader,
    all: Vec<parse::Member>,
    parsed: &mut Vec<(String, ast::ClassHeader, Vec<parse::Member>)>,
    outers: &mut BTreeMap<String, (String, bool)>,
) {
    let name = header.name.clone();
    if let Some(o) = outer {
        outers.insert(name.clone(), (o.to_string(), !header.is_static));
    }
    let mut members = Vec::new();
    let mut nested = Vec::new();
    for m in all {
        match m.kind {
            MemberKind::Class { header, members: inner } => nested.push((header, inner)),
            _ => members.push(m),
        }
    }
    parsed.push((name.clone(), header, members));
    for (h, ms) in nested {
        flatten(Some(&name), h, ms, parsed, outers);
    }
}

/// A class's place in the hierarchy, from its declaration.
fn place(h: &ast::ClassHeader, translated: &BTreeSet<String>) -> hier::Place {
    let names = |ts: &[ast::Type]| -> Vec<String> {
        // `SyntaxTreeNode implements TreeNode`, and `TreeNode` is `SyntaxTreeNode` here.
        ts.iter().map(|t| emit::class_name(&t.name)).filter(|n| translated.contains(n) && *n != h.name).collect()
    };
    let (parent, extern_parent, interfaces) = if h.is_interface {
        (None, None, names(&h.extends))
    } else {
        let sup = h.extends.first().map(|t| emit::class_name(&t.name));
        match sup {
            Some(p) if translated.contains(&p) => (Some(p), None, names(&h.implements)),
            other => (None, other, names(&h.implements)),
        }
    };
    hier::Place {
        parent,
        extern_parent,
        interfaces,
        is_interface: h.is_interface,
        is_abstract: h.is_abstract,
        outer: None,
        is_inner: false,
        type_params: h.type_params.clone(),
    }
}

/// Runs [`emit_on_this_thread`] with a large stack: the reader and emitter recurse once per
/// nesting level, and some SANY methods nest `else if` chains hundreds deep.
pub(crate) fn emit(tlaplus_src: &Path, out: &Path, check: bool) -> Result<(), String> {
    let (src, out) = (tlaplus_src.to_path_buf(), out.to_path_buf());
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || emit_on_this_thread(&src, &out, check))
        .map_err(|e| format!("spawning the emitter thread: {e}"))?
        .join()
        .map_err(|_| "the emitter panicked".to_string())?
}

fn emit_on_this_thread(tlaplus_src: &Path, out: &Path, check: bool) -> Result<(), String> {
    let tlc_src = tlaplus_src.join("tlatools/org.lamport.tlatools/src");
    let mut parsed: Vec<(String, ast::ClassHeader, Vec<parse::Member>)> = Vec::new();
    let mut outers: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for (class, file) in TRANSLATED {
        // Nested classes (`JJCalls`, `LookaheadSuccess`) become top-level classes.
        let (header, all) = parse_class(&tlc_src, file, class)?;
        flatten(None, header, all, &mut parsed, &mut outers);
    }
    let translated: BTreeSet<String> = parsed.iter().map(|p| p.0.clone()).collect();
    let hierarchy = hier::Hier::new(
        parsed
            .iter()
            .map(|(c, h, _)| {
                let mut p = place(h, &translated);
                if let Some((o, inner)) = outers.get(c) {
                    p.outer = Some(o.clone());
                    p.is_inner = *inner;
                }
                (c.clone(), p)
            })
            .collect(),
    );
    // An interface with methods stands for a type only when one family implements it.
    let unmodelled: BTreeSet<String> = parsed
        .iter()
        .filter(|(c, h, ms)| {
            h.is_interface && ms.iter().any(|m| matches!(m.kind, MemberKind::Method(_))) && !hierarchy.iface_root.contains_key(c)
        })
        .map(|p| p.0.clone())
        .collect();
    hier::HIER.with(|h| *h.borrow_mut() = hierarchy);
    let (mut classes, extern_consts) = externs::classes();
    // `ErrorCode`'s constants, from the enum's declaration.
    let error_codes = std::fs::read_to_string(tlc_src.join("tla2sany/semantic/ErrorCode.java")).map_err(|e| e.to_string())?;
    let info = classes.get_mut("ErrorCode").expect("ErrorCode extern");
    for line in error_codes.lines() {
        let t = line.trim_start();
        if line.starts_with("  ") && !line.starts_with("   ") && t.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            let name: String = t.chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_').collect();
            if t[name.len()..].trim_start().starts_with('(') {
                info.fields.insert(name, (emit::T::obj("ErrorCode"), true));
            }
        }
    }
    for c in extern_consts {
        emit::CONSTS.with(|s| s.borrow_mut().insert(c));
    }
    let overrides: BTreeMap<(String, String), ast::Type> =
        RAW_OVERRIDES.iter().map(|(c, f, t)| ((c.to_string(), f.to_string()), externs::parse_type(t))).collect();
    let known: BTreeSet<String> =
        classes.keys().cloned().chain(translated.iter().cloned()).filter(|c| !unmodelled.contains(c)).collect();
    let mut skipped = Vec::new();
    for (class, h, members) in &parsed {
        emit::collect_consts(class, members, h.is_interface);
    }
    for (class, h, members) in &parsed {
        let interfaces = hier::HIER.with(|x| x.borrow().places[class].interfaces.clone());
        let info = emit::class_info(class, members, interfaces, h.is_interface, &known, &overrides, &mut skipped);
        if info.has_statics {
            emit::CLASSES_WITH_STATICS.with(|s| s.borrow_mut().insert(class.clone()));
        }
        classes.insert(class.clone(), info);
    }
    emit::assign_slots(&mut classes);
    let mut code = String::from(
        "// @generated by `xtask jj2rs emit` from TLC's SANY parser sources. Do not edit.\n\n\
         #![allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused_mut, unused_variables, unused_assignments, unused_parens, unused_braces, unreachable_code, unreachable_patterns, unused_labels, unused_attributes, dead_code, clippy::all)]\n\n\
         use super::java::*;\n\
         use super::support::*;\n\n",
    );
    let mut errors = Vec::new();
    for (class, _, members) in &parsed {
        let hand: BTreeSet<String> = HAND_WRITTEN.iter().filter(|(c, _)| c == class).map(|(_, k)| k.to_string()).collect();
        emit::emit_class(&classes, class, members, &hand, &overrides, &mut code, &mut errors);
    }
    let roots: Vec<String> = hier::HIER.with(|h| h.borrow().families.keys().cloned().collect());
    for r in &roots {
        emit::emit_family(&classes, r, &mut code);
    }
    emit::emit_values(&classes, &parsed.iter().map(|p| p.0.clone()).collect::<Vec<_>>(), &mut code);
    // `Class.getName()`: the package comes from the source path.
    code.push_str("/// `Class.getName()` of a translated class, from its simple name.\npub(crate) fn java_class_name(simple: &str) -> &'static str {\n    match simple {\n");
    for (class, file) in TRANSLATED {
        let package = file.rsplit_once('/').map_or("", |(d, _)| d).replace('/', ".");
        code.push_str(&format!("        {class:?} => {:?},\n", format!("{package}.{class}")));
    }
    for (class, _, _) in parsed.iter().filter(|(c, ..)| !TRANSLATED.iter().any(|(t, _)| t == c)) {
        let (outer, file) = hier::HIER.with(|h| {
            let h = h.borrow();
            let mut top = class.clone();
            let mut path = vec![class.clone()];
            while let Some(o) = h.places.get(&top).and_then(|p| p.outer.clone()) {
                path.insert(0, o.clone());
                top = o;
            }
            (path.join("$"), TRANSLATED.iter().find(|(t, _)| *t == top).map(|(_, f)| *f).unwrap_or(""))
        });
        let package = file.rsplit_once('/').map_or("", |(d, _)| d).replace('/', ".");
        code.push_str(&format!("        {class:?} => {:?},\n", format!("{package}.{outer}")));
    }
    code.push_str("        _ => \"java.lang.Object\",\n    }\n}\n\n");
    emit::emit_jthrow(&classes, &mut code);
    let resets: String = parsed
        .iter()
        .filter(|(c, ..)| classes[c].has_statics)
        .map(|(c, ..)| format!("    {c}::reset_statics();\n"))
        .collect();
    code.push_str(&format!(
        "/// Forgets every translated class's statics, as a fresh JVM would (a new session).\npub(crate) fn reset_all_statics() {{\n{resets}}}\n"
    ));
    if check {
        let current = std::fs::read_to_string(out).map_err(|e| format!("{}: {e}", out.display()))?;
        return if current == code { Ok(()) } else { Err(format!("{} is stale; run `xtask jj2rs emit`", out.display())) };
    }
    std::fs::write(out, code).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("skipped (signature mentions an untranslated class):");
    for s in &skipped {
        println!("  {s}");
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &errors {
        let key = e.split(": ").nth(1).unwrap_or(e).to_string();
        *counts.entry(key).or_default() += 1;
    }
    println!("wrote {} ({} emission errors, {} distinct)", out.display(), errors.len(), counts.len());
    for (k, n) in &counts {
        println!("  {n:4}  {k}");
    }
    Ok(())
}
