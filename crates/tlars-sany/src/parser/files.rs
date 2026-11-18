//! How SANY finds and reads module files: `util.SimpleFilenameToStream` (the resolver),
//! `FilenameToStream.TLAFile`, `NamedInputStream`, `FileUtil.createNamedInputStream`,
//! `MonolithSpecExtractor.module` and the `ToolIO` streams they print to.
//!
//! The resolver searches the user directory, then the library paths, then the standard
//! modules. TLC reads the standard modules from its jar and copies each into a fresh
//! `tlc-*` temporary directory before parsing it; tlars embeds the same files
//! (`standard_modules.rs`) and copies them the same way, so the parser sees a real file. The
//! one difference: TLC's location of such a file is a `jar:` URL, printed after its path in
//! `Parsing file <path> (<url>)`; tlars has no jar, and prints the path alone, as TLC does
//! when its standard modules are on the file system.

#![allow(non_snake_case)]

use super::generated::TLAplusParser;
use super::java::*;
use super::support::SanyOutput;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    /// Lines TLC prints with `ToolIO.out.println` / `ToolIO.err.println` while resolving
    /// modules, in order; `(is_err, line)`.
    static TOOL_IO: RefCell<Vec<(bool, String)>> = const { RefCell::new(Vec::new()) };
}

fn tool_println(is_err: bool, line: String) {
    TOOL_IO.with(|t| t.borrow_mut().push((is_err, line)));
}

/// Takes the lines printed to `ToolIO.out` and `ToolIO.err` so far.
pub(crate) fn take_tool_io() -> Vec<(bool, String)> {
    TOOL_IO.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

const TLA_EXTENSION: &str = ".tla";

/// Strips one newline and what follows, as `createNamedInputStream` and `resolve` do.
fn strip_newline(name: &str) -> &str {
    match name.find('\n') {
        Some(n) => {
            tool_println(
                false,
                format!("*** Warning: module name '{name}' contained NEWLINE; Only the part before NEWLINE is considered."),
            );
            &name[..n]
        }
        None => name,
    }
}

/// `name` without a trailing `.tla` (matched case-insensitively), then with `.tla`.
fn with_tla_extension(name: &str) -> String {
    let stem = if name.to_lowercase().ends_with(TLA_EXTENSION) { &name[..name.len() - TLA_EXTENSION.len()] } else { name };
    format!("{stem}{TLA_EXTENSION}")
}

/// `util.SimpleFilenameToStream`.
pub(crate) struct FilenameToStream {
    /// `ToolIO.getUserDir()`, or the working directory when it is unset.
    user_dir: PathBuf,
    /// `-DTLA-Library` paths.
    library_paths: Vec<PathBuf>,
    /// `FilenameToStream.getTempDirectory()`, created on first use.
    tmp_dir: RefCell<Option<PathBuf>>,
}

/// `FilenameToStream.TLAFile`: a path, whether the standard library supplied it, and its
/// location (`libraryPath`, `File.toURI()` of an existing file).
struct TlaFile {
    path: PathBuf,
    is_library: bool,
    library_uri: Option<String>,
}

/// `File.toURI()` of an absolute file path.
fn file_uri(path: &Path) -> String {
    format!("file:{}", path.display())
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl FilenameToStream {
    /// `new SimpleFilenameToStream(libraryPaths)` with `ToolIO.setUserDir(user_dir)`.
    pub(crate) fn new(user_dir: Option<PathBuf>, library_paths: Vec<PathBuf>) -> J<FilenameToStream> {
        let user_dir = user_dir.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        J::new(FilenameToStream { user_dir, library_paths, tmp_dir: RefCell::new(None) })
    }

    /// `Files.createTempDirectory("tlc-")`.
    fn tmp_dir(&self) -> PathBuf {
        let mut slot = self.tmp_dir.borrow_mut();
        if let Some(d) = &*slot {
            return d.clone();
        }
        let base = std::env::temp_dir();
        let pid = std::process::id();
        let mut n = 0u32;
        let dir = loop {
            let candidate = base.join(format!("tlc-{pid}-{n}"));
            if std::fs::create_dir(&candidate).is_ok() {
                break candidate;
            }
            n += 1;
        };
        *slot = Some(dir.clone());
        dir
    }

    /// `locate`: the user directory, the library paths, then the standard modules; else a
    /// file in the user directory that does not exist.
    fn locate(&self, name: &str) -> TlaFile {
        for dir in std::iter::once(&self.user_dir).chain(&self.library_paths) {
            let p = absolute(&dir.join(name));
            if p.exists() {
                let library_uri = Some(file_uri(&p));
                return TlaFile { path: p, is_library: false, library_uri };
            }
        }
        if let Some((_, bytes)) = crate::standard_modules::STANDARD_MODULES.iter().find(|(n, _)| *n == name) {
            let p = self.tmp_dir().join(name);
            // `read` prints the stack trace of a failed copy and returns the file anyway.
            if let Err(e) = std::fs::write(&p, bytes) {
                tool_println(true, format!("java.io.IOException: {e}"));
            }
            let library_uri = Some(file_uri(&p));
            return TlaFile { path: p, is_library: true, library_uri };
        }
        let p = absolute(&self.user_dir.join(name));
        let library_uri = p.exists().then(|| file_uri(&p));
        TlaFile { path: p, is_library: false, library_uri }
    }

    /// `resolve(name, true)`.
    fn resolve_module(&self, name: &str) -> TlaFile {
        self.locate(&with_tla_extension(strip_newline(name)))
    }

    /// `getFullPath()`.
    fn full_path(&self) -> String {
        let mut parts: Vec<String> = vec![self.user_dir.display().to_string()];
        parts.extend(self.library_paths.iter().map(|p| p.display().to_string()));
        parts.push("classpath:/tla2sany/StandardModules/".into());
        parts.push("classpath:/".into());
        parts.join(", ")
    }
}

impl Drop for FilenameToStream {
    fn drop(&mut self) {
        // TLC deletes the copies on exit (`deleteOnExit`).
        if let Some(d) = self.tmp_dir.get_mut().take() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

impl J<FilenameToStream> {
    /// `isStandardModule(moduleName)`.
    pub(crate) fn isStandardModule(&self, module: JStr) -> bool {
        let f = self.jget().resolve_module(module.as_str());
        f.path.exists() && f.is_library
    }
}

/// `util.NamedInputStream`, with the file's contents read when it is opened.
pub(crate) struct NamedInputStream {
    file_name: JStr,
    module_name: JStr,
    path: PathBuf,
    is_library: bool,
    /// The resolver's location of the file; `None` for a module extracted from a monolith.
    library_uri: Option<String>,
    /// The monolith spec a module was extracted from.
    monolith: Option<PathBuf>,
    bytes: Vec<u8>,
}

impl J<NamedInputStream> {
    pub(crate) fn getName(&self) -> JStr {
        self.jread(|n| n.file_name.clone())
    }
    pub(crate) fn getFileName(&self) -> JStr {
        self.getName()
    }
    pub(crate) fn getModuleName(&self) -> JStr {
        self.jread(|n| n.module_name.clone())
    }
    /// `sourceFile().exists()`.
    pub(crate) fn exists(&self) -> bool {
        self.jread(|n| n.path.exists())
    }
    /// `sourceFile() instanceof TLAFile && ((TLAFile) sourceFile()).isLibraryModule()`.
    /// The file's contents as `FileInputStream` would read them.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        self.jread(|n| n.bytes.clone())
    }
    pub(crate) fn isLibraryModule(&self) -> bool {
        self.jread(|n| n.is_library)
    }
    /// `getAbsoluteResolvedPath()`, falling back to the path itself as `parseFile` does.
    pub(crate) fn absoluteResolvedPath(&self) -> JStr {
        self.jread(|n| JStr::of(&std::fs::canonicalize(&n.path).unwrap_or_else(|_| n.path.clone()).display().to_string()))
    }
    /// `parseFile`'s `originalFilePath`: the file's location when the resolved path differs
    /// from it (a symbolic link), or the monolith spec a module came from.
    pub(crate) fn originalFilePathSuffix(&self) -> JStr {
        let suffix = self.jread(|n| match (&n.library_uri, &n.monolith) {
            (Some(uri), _) => {
                let real = std::fs::canonicalize(&n.path).unwrap_or_else(|_| n.path.clone());
                if file_uri(&real) == *uri { String::new() } else { format!(" ({uri})") }
            }
            (None, Some(root)) => format!(" ({})", absolute(root).display()),
            (None, None) => String::new(),
        });
        JStr::of(&suffix)
    }
}

/// `util.MonolithSpecExtractor.module`: the lines of `root` from `---- MODULE name ----`
/// through the next `====` line, written to a temporary file.
fn monolith_module(root: &Path, module_name: &str) -> Option<J<NamedInputStream>> {
    let text = std::fs::read(root).ok()?;
    let text = String::from_utf8_lossy(&text);
    let mut out = String::new();
    let mut active = false;
    for line in java_lines(&text) {
        if active && line.starts_with("====") {
            out.push_str(line);
            out.push('\n');
            break;
        }
        if !active && is_module_start(line, module_name) {
            active = true;
        }
        if active {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !active {
        return None;
    }
    let safe = format!("{}{TLA_EXTENSION}", Path::new(module_name).file_name()?.to_string_lossy());
    let dir = tempdir_for_monolith()?;
    let path = dir.join(&safe);
    std::fs::write(&path, out.as_bytes()).ok()?;
    Some(J::new(NamedInputStream {
        file_name: JStr::of(&safe),
        module_name: JStr::of(module_name),
        path,
        is_library: false,
        library_uri: None,
        monolith: None,
        bytes: out.into_bytes(),
    }))
}

/// `Files.createTempDirectory(null)`.
fn tempdir_for_monolith() -> Option<PathBuf> {
    let base = std::env::temp_dir();
    let pid = std::process::id();
    (0..u32::MAX).map(|n| base.join(format!("tlars-{pid}-{n}"))).find(|d| std::fs::create_dir(d).is_ok())
}

/// `BufferedReader.readLine` over the whole text: lines end at `\n`, `\r` or `\r\n`.
fn java_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            Some(i) => {
                lines.push(&rest[..i]);
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + skip..];
            }
            None => {
                lines.push(rest);
                break;
            }
        }
    }
    lines
}

/// `line.matches("-----*\\s*MODULE\\s+" + Pattern.quote(name) + "\\s*----*")`.
fn is_module_start(line: &str, name: &str) -> bool {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r');
    let dashes = line.len() - line.trim_start_matches('-').len();
    if dashes < 4 {
        return false;
    }
    let rest = line[dashes..].trim_start_matches(is_ws);
    let Some(rest) = rest.strip_prefix("MODULE") else { return false };
    let after = rest.trim_start_matches(is_ws);
    if after.len() == rest.len() {
        return false;
    }
    let Some(rest) = after.strip_prefix(name) else { return false };
    let rest = rest.trim_start_matches(is_ws);
    rest.len() >= 3 && rest.chars().all(|c| c == '-')
}

pub(crate) struct FileUtil;

impl FileUtil {
    pub(crate) fn createNamedInputStream_2(name: JStr, resolver: J<FilenameToStream>) -> J<NamedInputStream> {
        Self::createNamedInputStream_3(name, resolver, J::null())
    }

    /// `FileUtil.createNamedInputStream(name, resolver, rootFileNis)`.
    pub(crate) fn createNamedInputStream_3(
        name: JStr,
        resolver: J<FilenameToStream>,
        root: J<NamedInputStream>,
    ) -> J<NamedInputStream> {
        let name = strip_newline(name.as_str()).to_string();
        let stem = if name.to_lowercase().ends_with(TLA_EXTENSION) { &name[..name.len() - TLA_EXTENSION.len()] } else { &name };
        let source_file_name = format!("{stem}{TLA_EXTENSION}");
        let module_name = stem.rsplit('/').next().unwrap_or(stem);
        let r = resolver.jget();
        let file = r.locate(&source_file_name);
        if file.path.exists() {
            match std::fs::read(&file.path) {
                Ok(bytes) => {
                    return J::new(NamedInputStream {
                        file_name: JStr::of(&source_file_name),
                        module_name: JStr::of(module_name),
                        path: file.path,
                        is_library: file.is_library,
                        library_uri: file.library_uri,
                        monolith: None,
                        bytes,
                    });
                }
                Err(_) => tool_println(false, "***Internal error: Unable to create NamedInputStream in toIStream method".into()),
            }
        }
        if !root.is_null() {
            let root_path = root.jread(|n| n.path.clone());
            return match monolith_module(&root_path, stem) {
                Some(nis) => {
                    nis.jget_mut().monolith = Some(root_path);
                    nis
                }
                None => J::null(),
            };
        }
        tool_println(
            true,
            format!("File does not exist: {} while looking in these directories: {}", file.path.display(), r.full_path()),
        );
        J::null()
    }

    /// `new TLAplusParser(out, nis)`.
    pub(crate) fn newParser(out: J<SanyOutput>, nis: J<NamedInputStream>) -> JResult<J<TLAplusParser>> {
        let bytes = nis.jread(|n| n.bytes.clone());
        TLAplusParser::new_from_bytes(out, &bytes)
    }
}

pub(crate) struct ToolIO;

impl ToolIO {
    /// `ToolIO.getDefaultResolver()`: a `SimpleFilenameToStream` over the working directory
    /// and the `TLA-Library` paths (unset: tlars has no system properties).
    pub(crate) fn getDefaultResolver() -> J<FilenameToStream> {
        FilenameToStream::new(None, Vec::new())
    }
}
