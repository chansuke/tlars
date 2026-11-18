// Writes TLC's parser output for each listed module, parsed in sequence in one JVM (as TLC
// parses a spec's modules). See tools/golden/parse_golden.sh for the checked-in goldens.
// Usage (inside `nix develop`):
//   javac -d target/golden -cp $TLA2TOOLS_JAR tools/golden/ParseGolden.java
//   java -cp target/golden:$TLA2TOOLS_JAR tla2sany.parser.ParseGolden OUT ROOT FILE...
//
// FILEs are relative to ROOT. Per file: "== <FILE>", "parse <true|false>", "log <LEVEL> <message>" for each message the
// parser logs above TRACE, "intern <tok> <string>" for each string interned while parsing
// (tokens continue across files), then the tree in preorder, one node per line:
// "<depth> <kind> <image|-> <l0>,<l1>,<l2>,<l3> <zero count|-> <one count|->". Strings
// escape `\`, newline, CR and tab.

package tla2sany.parser;

import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import tla2sany.output.LogLevel;
import tla2sany.output.SanyOutput;
import util.UniqueString;

public class ParseGolden {
    static String esc(String s) {
        return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t");
    }

    static void dump(PrintWriter w, SyntaxTreeNode n, int depth) {
        String image = n.image == null ? "-" : esc(n.image.toString());
        w.printf("%d %d %s %d,%d,%d,%d %s %s%n", depth, n.kind, image, n.location[0], n.location[1], n.location[2],
                n.location[3], n.zero == null ? "-" : n.zero.length, n.one == null ? "-" : n.one.length);
        if (n.zero != null) for (SyntaxTreeNode c : n.zero) dumpChild(w, c, depth + 1);
        if (n.one != null) for (SyntaxTreeNode c : n.one) dumpChild(w, c, depth + 1);
    }

    static void dumpChild(PrintWriter w, SyntaxTreeNode c, int depth) {
        if (c == null) w.printf("%d null%n", depth);
        else dump(w, c, depth);
    }

    public static void main(String[] args) throws IOException {
        int interned = 0;
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            Path root = Path.of(args[1]);
            for (int i = 2; i < args.length; i++) {
                Path file = root.resolve(args[i]);
                List<String> log = new ArrayList<>();
                SanyOutput out = new SanyOutput() {
                    public void log(LogLevel level, String format, Object... a) {
                        // As TLC's SimpleSanyOutput: no arguments, no String.format.
                        if (level != LogLevel.TRACE) log.add(level + " " + (a.length == 0 ? format : String.format(format, a)));
                    }
                    public java.io.PrintStream getStream(LogLevel level) { return System.out; }
                };
                TLAplusParser p = new TLAplusParser(out, Files.readAllBytes(file));
                boolean ok = p.parse();
                w.println("== " + args[i]);
                w.println("parse " + ok);
                for (String l : log) w.println("log " + esc(l));
                for (UniqueString u; (u = UniqueString.uidToUniqueString(interned + 1)) != null; interned++) {
                    w.println("intern " + (interned + 1) + " " + esc(u.toString()));
                }
                if (ok) dump(w, p.ParseTree, 0);
            }
        }
    }
}
