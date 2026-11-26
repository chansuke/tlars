//! `tla2sany.drivers.SANY`: the front end's entry point. `parse` loads every module of a
//! spec (`SpecObj.loadSpec`, translated), runs the translated `Generator` and level checker
//! on each in dependency order, and reports warnings and errors to the `SanyOutput` with
//! SANY's text. Written by hand: the class is glue between translated classes, and most of
//! it is the command line (`SANYmain`), the explorer and deprecated overloads.
//!
//! Not modelled: the stack trace `AbortException` prints during semantic analysis (a JVM
//! stack trace; only its first line, the exception, is printed).

use super::generated::{Context, ErrorDetails, Generator, Linter, ParseUnit, SpecObj};
use super::java::*;
use super::support::{ArrayList, ErrorCode, LogLevel, SanyOutput, UniqueString};

/// `tla2sany.drivers.SanySettings`.
#[derive(Clone, Debug)]
pub(crate) struct SanySettings {
    pub(crate) strict_error_codes: bool,
    pub(crate) semantic_analysis: bool,
    pub(crate) level_checking: bool,
    pub(crate) linting: bool,
    pub(crate) validate_pcal_translation: bool,
    pub(crate) suppressed_codes: Vec<ErrorCode>,
    pub(crate) messages_as_error_codes: Vec<ErrorCode>,
}

/// `tla2sany.drivers.SanyExitCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SanyExitCode {
    Ok,
    SyntaxParsingFailure,
    SemanticAnalysisOrLevelCheckingFailure,
    Error,
}

impl SanyExitCode {
    pub(crate) fn code(self) -> i32 {
        match self {
            SanyExitCode::Ok => 0,
            SanyExitCode::SyntaxParsingFailure => 2,
            SanyExitCode::SemanticAnalysisOrLevelCheckingFailure => 4,
            SanyExitCode::Error => -1,
        }
    }

    /// `fromCode`.
    pub(crate) fn from_code(code: i32) -> SanyExitCode {
        match code {
            0 => SanyExitCode::Ok,
            2 => SanyExitCode::SyntaxParsingFailure,
            4 => SanyExitCode::SemanticAnalysisOrLevelCheckingFailure,
            _ => SanyExitCode::Error,
        }
    }
}

/// Why `parse` stopped: what SANY's `ParseException`/`SemanticException` (exit code
/// `ERROR`) and `FrontEndException` (thrown to the caller) carry.
#[derive(Debug)]
pub(crate) enum FrontEndFailure {
    /// `FrontEndException`: an unexpected exception, already logged.
    FrontEnd(#[expect(dead_code, reason = "M5: TLC_PARSING_FAILED2 prints the exception")] JThrow),
}

/// `Throwable.toString()` of what the front end catches.
fn throwable_string(e: &JThrow) -> String {
    match e {
        // `AbortException.toString()` is its details'.
        JThrow::AbortException(a) => {
            a.jread(|a| a.details.clone()).toString().map(|s| s.java_string()).unwrap_or_else(|_| "tla2sany.semantic.AbortException".into())
        }
        other => {
            let class = match other {
                JThrow::Extern(x) => extern_class_name(x.class),
                // The family root's name: the front end only reaches here with exceptions
                // that have no translated subclasses.
                translated => super::generated::java_class_name(&format!("{translated:?}")).to_string(),
            };
            match other.getMessage() {
                Ok(m) if !m.is_null() => format!("{class}: {}", m.java_string()),
                _ => class,
            }
        }
    }
}

/// `Class.getName()` of a library exception.
fn extern_class_name(simple: &str) -> String {
    let package = match simple {
        "IOException" | "UnsupportedEncodingException" | "FileNotFoundException" => "java.io",
        "NoSuchElementException" => "java.util",
        "WrongInvocationException" => "util",
        _ => "java.lang",
    };
    format!("{package}.{simple}")
}

const SYNTAX_PARSING_FAILURE: i32 = 2;
const SEMANTIC_FAILURE: i32 = 4;

/// `SANY.parse(spec, fileName, out, settings)`.
pub(crate) fn parse(spec: &J<SpecObj>, out: &J<SanyOutput>, settings: &SanySettings) -> Result<SanyExitCode, FrontEndFailure> {
    let run = || -> JResult<Option<SanyExitCode>> {
        // **** Initialize the global environment
        Context::reInit()?;
        if !front_end_parse(spec, out, settings)? {
            return Ok(Some(SanyExitCode::Error));
        }
        if settings.semantic_analysis {
            if !front_end_semantic_analysis(spec, out, settings)? {
                return Ok(Some(SanyExitCode::Error));
            }
            let error_level = spec.jread(|s| s.errorLevel);
            if settings.level_checking && SanyExitCode::from_code(error_level) == SanyExitCode::Ok && settings.linting {
                let semantic_errors = spec.jread(|s| s.semanticErrors.clone());
                let warns_before = semantic_errors.getWarningDetails()?.size();
                front_end_linting(spec, out)?;
                // Print linting warnings, respecting suppressed/elevated codes.
                let all_warnings = semantic_errors.getWarningDetails()?;
                let linting: Vec<J<ErrorDetails>> = all_warnings.jget().0[warns_before as usize..].to_vec();
                let linting = ArrayList::from_vec(linting);
                if report_messages(out, &filter_visible_messages(&linting, settings)?, settings)? {
                    set_error_level(spec, SEMANTIC_FAILURE);
                }
            }
        }
        Ok(None)
    };
    match run() {
        Ok(Some(code)) => Ok(code),
        Ok(None) => Ok(if settings.strict_error_codes { SanyExitCode::from_code(spec.jread(|s| s.errorLevel)) } else { SanyExitCode::Ok }),
        // `catch (Exception e)`: logged and rethrown as a `FrontEndException`.
        Err(e) => {
            out.log_2(LogLevel::Error, JStr::of(&throwable_string(&e)));
            Err(FrontEndFailure::FrontEnd(e))
        }
    }
}

fn is_message_elevated(message: &J<ErrorDetails>, settings: &SanySettings) -> JResult<bool> {
    let code = message.getCode()?;
    Ok(settings.messages_as_error_codes.contains(&code))
}

fn filter_visible_messages(messages: &J<ArrayList<J<ErrorDetails>>>, settings: &SanySettings) -> JResult<Vec<J<ErrorDetails>>> {
    let mut visible = Vec::new();
    for m in messages.jget().0.iter() {
        if !settings.suppressed_codes.contains(&m.getCode()?) {
            visible.push(m.clone());
        }
    }
    Ok(visible)
}

/// `reportMessages`: non-elevated messages as warnings, then elevated ones as errors;
/// whether any was elevated.
fn report_messages(out: &J<SanyOutput>, messages: &[J<ErrorDetails>], settings: &SanySettings) -> JResult<bool> {
    let mut elevated = false;
    for m in messages {
        if !is_message_elevated(m, settings)? {
            out.log_3(LogLevel::Warning, JStr::of("%s\n\n\n"), &m.toString()?);
        }
    }
    for m in messages {
        if is_message_elevated(m, settings)? {
            out.log_3(LogLevel::Error, JStr::of("Warning treated as error: %s\n\n\n"), &m.toString()?);
            elevated = true;
        }
    }
    Ok(elevated)
}

fn set_error_level(spec: &J<SpecObj>, level: i32) {
    spec.jget_mut().errorLevel = level;
}

/// `frontEndParse(spec, out, settings)`; `false` where SANY throws `ParseException`.
fn front_end_parse(spec: &J<SpecObj>, out: &J<SanyOutput>, settings: &SanySettings) -> JResult<bool> {
    let parse_errors = spec.jread(|s| s.parseErrors.clone());
    let loaded = spec.loadSpec_4(spec.getFileName()?, parse_errors.clone(), settings.validate_pcal_translation, out.clone());
    let failure = match loaded {
        Ok(_) => {
            let visible = filter_visible_messages(&parse_errors.getWarningDetails()?, settings)?;
            if !visible.is_empty() {
                out.log_4(
                    LogLevel::Warning,
                    JStr::of("Warnings (%d) during syntax parsing of %s:\n\n"),
                    &(visible.len() as i32),
                    &spec.getFileName()?,
                );
                if report_messages(out, &visible, settings)? {
                    set_error_level(spec, SYNTAX_PARSING_FAILURE);
                }
            }
            if parse_errors.isSuccess()? {
                return Ok(true);
            }
            out.log_2(LogLevel::Error, parse_errors.toString()?);
            None
        }
        // `catch (ParseException e)`.
        Err(JThrow::ParseException(_)) => None,
        // `catch (Exception e)`.
        Err(e) if e.is_a("Exception") => Some(e),
        Err(e) => return Err(e),
    };
    set_error_level(spec, SYNTAX_PARSING_FAILURE);
    if let Some(e) = failure {
        out.log_3(LogLevel::Error, JStr::of("\nFatal errors while parsing TLA+ spec in file %s\n"), &spec.getFileName()?);
        out.log_2(LogLevel::Error, JStr::of(&throwable_string(&e)));
        out.log_2(LogLevel::Error, parse_errors.toString()?);
    }
    Ok(false)
}

/// `frontEndSemanticAnalysis(spec, out, settings)`; `false` where SANY throws
/// `SemanticException`.
fn front_end_semantic_analysis(spec: &J<SpecObj>, out: &J<SanyOutput>, settings: &SanySettings) -> JResult<bool> {
    let table = spec.getExternalModuleTable()?;
    let semantic_errors = spec.jread(|s| s.semanticErrors.clone());
    let order = spec.jread(|s| s.semanticAnalysisVector.clone());
    let body = || -> JResult<()> {
        for i in 0..order.size() {
            let module_name = order.elementAt(i);
            if !table.getContext(UniqueString::uniqueStringOf(module_name.clone()))?.is_null() {
                continue;
            }
            let parse_unit: J<ParseUnit> = spec.jread(|s| s.parseUnitContext.clone()).get(module_name.clone());
            let syntax_tree_root = parse_unit.getParseTree()?;
            out.log_3(LogLevel::Info, JStr::of("Semantic processing of module %s"), &module_name);
            let generator = Generator::new_2(table.clone(), semantic_errors.clone())?;
            let module_node = generator.generate(syntax_tree_root)?;
            module_node.setStandard(spec.getResolver()?.isStandardModule(module_name.clone()))?;
            table.put(
                UniqueString::uniqueStringOf(module_name.clone()),
                generator.getSymbolTable()?.getExternalContext()?,
                module_node.clone(),
            )?;
            if !module_node.is_null() && semantic_errors.isSuccess()? && settings.level_checking {
                module_node.levelCheck_1(semantic_errors.clone())?;
            }
            if i == order.size() - 1 {
                table.setRootModule(module_node.clone())?;
            }
            if semantic_errors.getNumMessages()? > 0 {
                let visible = filter_visible_messages(&semantic_errors.getWarningDetails()?, settings)?;
                if !visible.is_empty() {
                    out.log_3(LogLevel::Warning, JStr::of("*** Warnings: %d\n"), &(visible.len() as i32));
                    if report_messages(out, &visible, settings)? {
                        set_error_level(spec, SEMANTIC_FAILURE);
                    }
                }
                let errors = semantic_errors.getErrorDetails()?;
                if !errors.isEmpty() {
                    out.log_3(LogLevel::Error, JStr::of("Semantic errors:\n\n*** Errors: %d\n"), &errors.size());
                    for e in errors.jget().0.iter() {
                        out.log_3(LogLevel::Error, JStr::of("%s\n\n"), &e.toString()?);
                    }
                }
                if semantic_errors.getNumErrors()? > 0 {
                    set_error_level(spec, SEMANTIC_FAILURE);
                }
            }
        }
        Ok(())
    };
    match body() {
        Ok(()) => Ok(true),
        Err(JThrow::AbortException(a)) => {
            let e = JThrow::AbortException(a);
            out.log_4(
                LogLevel::Error,
                JStr::of("Fatal errors in semantic processing of TLA spec %s\n%s\nStack trace for exception:\n"),
                &spec.getFileName()?,
                &e.getMessage()?,
            );
            // `e.printStackTrace(out.getStream(LogLevel.ERROR))`, without the JVM frames.
            out.log_2(LogLevel::Error, JStr::of(&throwable_string(&e)));
            if semantic_errors.getNumMessages()? > 0 {
                out.log_3(
                    LogLevel::Error,
                    JStr::of("Semantic errors detected before the unexpected exception:\n\n%s"),
                    &semantic_errors.toString()?,
                );
                if semantic_errors.getNumErrors()? > 0 {
                    set_error_level(spec, SEMANTIC_FAILURE);
                }
            }
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// `frontEndLinting`: lints every module that is not a standard module.
fn front_end_linting(spec: &J<SpecObj>, out: &J<SanyOutput>) -> JResult<()> {
    let table = spec.getExternalModuleTable()?;
    let semantic_errors = spec.jread(|s| s.semanticErrors.clone());
    let linter = Linter::new_0()?;
    let modules = table.getModuleNodes()?;
    for i in 0..modules.length() {
        let module = modules.at(i);
        if module.isStandardModule()? {
            continue;
        }
        out.log_3(LogLevel::Info, JStr::of("Linting of module %s"), &module.getName_0_via_SymbolNode()?);
        linter.lint(module, table.clone(), semantic_errors.clone())?;
    }
    Ok(())
}
