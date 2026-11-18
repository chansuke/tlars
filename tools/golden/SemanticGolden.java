// Writes what SANY reports for each listed spec: parsing, semantic analysis and level
// checking through SANY.parse. tools/golden/semantic_golden.sh runs one JVM per spec, as TLC
// does, and concatenates the outputs into the checked-in goldens.
// Usage (inside `nix develop`):
//   javac -d target/golden -cp $TLA2TOOLS_JAR tools/golden/SemanticGolden.java
//   java -cp target/golden:$TLA2TOOLS_JAR tla2sany.drivers.SemanticGolden OUT ROOT DEPTH FILE...
//
// Each FILE (relative to ROOT) is a root module, resolved as TLC does: the user directory is
// the file's directory. Settings: no strict error codes; semantic analysis, level checking,
// linting and PlusCal validation on. The first line is "depth DEPTH". Per file: "== <FILE>", "exit <code>" (or "exit
// FrontEndException"), "errorLevel <n>", "log <LEVEL> <message>" for each message above
// TRACE ("Parsing file <path>[ (<url>)]" reduced to the file's name), "toolio <out|err>
// <line>" for lines printed to ToolIO, "intern <tok> <string>" for each string interned
// (tokens continue across the files of one run), then, when SANY succeeded, the ExternalModuleTable's
// toString(DEPTH) as "dump <line>" lines (or "dump-exception <class>" when it throws) and "level <module> <definition> <level>" for each
// module's operator definitions. Strings escape `\`, newline, CR and tab.

package tla2sany.drivers;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;
import tla2sany.modanalyzer.SpecObj;
import tla2sany.output.LogLevel;
import tla2sany.output.SanyOutput;
import tla2sany.semantic.ModuleNode;
import tla2sany.semantic.OpDefNode;
import util.SimpleFilenameToStream;
import util.ToolIO;
import util.UniqueString;

public class SemanticGolden {
    static String esc(String s) {
        return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t");
    }

    /** "Parsing file /abs/dir/M.tla (url)" -> "Parsing file M.tla". */
    static String normalize(String message) {
        final String prefix = "Parsing file ";
        if (!message.startsWith(prefix)) return message;
        String path = message.substring(prefix.length());
        int paren = path.indexOf(" (");
        if (paren >= 0) path = path.substring(0, paren);
        return prefix + path.substring(path.lastIndexOf('/') + 1);
    }

    static void lines(PrintWriter w, String tag, ByteArrayOutputStream bytes) {
        String text = bytes.toString(StandardCharsets.UTF_8);
        bytes.reset();
        if (text.isEmpty()) return;
        for (String l : text.split("\n", -1)) {
            if (!l.isEmpty()) w.println("toolio " + tag + " " + esc(l));
        }
    }

    public static void main(String[] args) throws IOException {
        int interned = 0;
        final ByteArrayOutputStream toolOut = new ByteArrayOutputStream();
        final ByteArrayOutputStream toolErr = new ByteArrayOutputStream();
        ToolIO.out = new PrintStream(toolOut, true, StandardCharsets.UTF_8);
        ToolIO.err = new PrintStream(toolErr, true, StandardCharsets.UTF_8);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            Path root = Path.of(args[1]).toAbsolutePath();
            int depth = Integer.parseInt(args[2]);
            w.println("depth " + depth);
            for (int i = 3; i < args.length; i++) {
                Path file = root.resolve(args[i]);
                final long started = System.nanoTime();
                List<String> log = new ArrayList<>();
                SanyOutput out = new SanyOutput() {
                    public void log(LogLevel level, String format, Object... a) {
                        // As TLC's SimpleSanyOutput: no arguments, no String.format.
                        if (level != LogLevel.TRACE) log.add(level + " " + normalize(a.length == 0 ? format : String.format(format, a)));
                    }
                    public PrintStream getStream(LogLevel level) { return System.out; }
                };
                ToolIO.setUserDir(file.getParent().toString());
                String name = file.getFileName().toString();
                SpecObj spec = new SpecObj(name, new SimpleFilenameToStream());
                SanySettings settings = new SanySettings(false, true, true, true, true, Set.of(), Set.of());
                String exit;
                try {
                    exit = Integer.toString(SANY.parse(spec, name, out, settings).code());
                } catch (FrontEndException e) {
                    exit = "FrontEndException";
                }
                w.println("== " + args[i]);
                w.println("exit " + exit);
                w.println("errorLevel " + spec.getErrorLevel());
                for (String l : log) w.println("log " + esc(l));
                lines(w, "out", toolOut);
                lines(w, "err", toolErr);
                for (UniqueString u; (u = UniqueString.uidToUniqueString(interned + 1)) != null; interned++) {
                    w.println("intern " + (interned + 1) + " " + esc(u.toString()));
                }
                if (exit.equals("0") && spec.getErrorLevel() == 0) {
                    try {
                        String dump = spec.getExternalModuleTable().toString(depth, spec.getSemanticErrors());
                        for (String l : dump.split("\n", -1)) w.println("dump " + esc(l));
                    } catch (RuntimeException e) {
                        // The explorer's toString has bugs of its own (null context entries).
                        w.println("dump-exception " + e.getClass().getName());
                    }
                    for (ModuleNode m : spec.getExternalModuleTable().getModuleNodes()) {
                        for (OpDefNode d : m.getOpDefs()) {
                            w.println("level " + m.getName() + " " + d.getName() + " " + d.getLevel());
                        }
                    }
                }
                w.flush();
                System.err.printf("%6d ms  %s%n", (System.nanoTime() - started) / 1_000_000, args[i]);
            }
        }
    }
}
