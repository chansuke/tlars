// Writes golden files for crates/tlars-java from the running JDK and from TLC's pprint.
// Usage (inside `nix develop`):
//   java -cp "$TLA2TOOLS_JAR" tools/golden/JavaGolden.java crates/tlars-java/tests/golden
// The oracle JDK is pinned to 21 (docs/DESIGN.md): Double.toString changed in JDK 19.

import java.io.IOException;
import java.io.PrintWriter;
import java.math.BigDecimal;
import java.math.MathContext;
import java.nio.file.Files;
import java.nio.file.Path;
import java.text.DecimalFormat;
import java.text.SimpleDateFormat;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.Hashtable;
import java.util.List;
import java.util.Random;
import java.util.TimeZone;
import java.util.TreeSet;
import tlc2.pprint.PrettyPrint;

public class JavaGolden {
    public static void main(String[] args) throws IOException {
        Path dir = Path.of(args[0]);
        Files.createDirectories(dir);
        stringHash(dir.resolve("string_hash.txt"));
        hashtable(dir.resolve("hashtable.txt"));
        random(dir.resolve("random.txt"));
        doubles(dir.resolve("double_to_string.txt"));
        reciprocal(dir.resolve("reciprocal.txt"));
        decimalFormat(dir.resolve("decimal_format.txt"));
        dates(dir.resolve("date_format.txt"));
        intHashSet(dir.resolve("int_hashset.txt"));
        hashMap(dir.resolve("hashmap.txt"));
        pprint(dir.resolve("pprint.txt"));
    }

    // "<width>\t<flat value>\t<PrettyPrint.mypp(value, width)>", with backslash, tab, newline
    // and carriage return written as \\, \t, \n and \r. The values cover every Parse and
    // Format path; each one is printed at a fixed set of widths plus its own length minus
    // one, its length and its length plus one, which is where flat turns into broken.
    static void pprint(Path p) throws IOException {
        String[] values = {
            // constants, including the ones Parse has a branch for
            "", "TRUE", "FALSE", "42", "-7", "-", "1..5", "0..2147483647", "1..", "mv1",
            "\"\"", "\"abc\"", "aLongModelValueName", "123456789012345678901234567890",
            "\"a string constant that is much wider than any of the widths below\"",
            // string escapes StringValue.PrintVersion writes: \t \n \f \r \\ and \"
            "{\"a\\tb\", \"c\\nd\", \"e\\fg\", \"h\\ri\", 1}",
            "{\"a\\\\b\", \"cccccccccc\", \"dddddddddd\", \"eeeeeeeeee\", \"ffffffffff\"}",
            "{\"a\\\"b\", \"cccccccccc\", \"dddddddddd\", \"eeeeeeeeee\", \"ffffffffff\"}",
            // sets
            "{}", "{1}", "{1, 2, 3}", "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15}",
            "{\"aaaaaaaaaa\", \"bbbbbbbbbb\", \"cccccccccc\", \"dddddddddd\", \"eeeeeeeeee\"}",
            "{{1, 2, 3}, {4, 5, 6}, {7, 8, 9}}",
            "{{{1, 2}, {3, 4}}, {{5, 6}, {7, 8}}, {{9, 10}, {11, 12}}, {{13, 14}, {15, 16}}}",
            "{{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12}, 13, {14, 15, 16, 17, 18, 19, 20, 21}}",
            "{-1, -22, -333, -4444, -55555, -666666, -7777777, -88888888, -999999999, 0}",
            "{1..5, 6..10, 11..15, 16..20, 21..25, 26..30, 31..35, 36..40, 41..45, 46..50}",
            // sequences
            "<<>>", "<<1>>", "<<1, 2, 3>>", "<<1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13>>",
            "<<<<1, 2, 3>>, <<4, 5, 6>>, <<7, 8, 9>>, <<10, 11, 12>>, <<13, 14, 15>>>>",
            "<<{1, 2, 3, 4, 5, 6, 7, 8}, [a |-> 1, b |-> 2], (1 :> 2 @@ 3 :> 4), \"xy\">>",
            // records: value on its own line, and a field name too wide for the line
            "[a |-> 1]", "[a |-> 1, b |-> 2]",
            "[aaaaaaaaaa |-> 1, bbbbbbbbbb |-> 2, cccccccccc |-> 3, dddddddddd |-> 4]",
            "[a |-> {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12}, b |-> <<13, 14, 15, 16, 17>>]",
            "[aLongFieldNameThatDoesNotFitOnOneLineAtAll |-> {1, 2, 3, 4, 5, 6, 7, 8, 9}]",
            "[a |-> [b |-> [c |-> [d |-> {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13}]]]]",
            // functions: both halves plain, a wide range, and a domain that must break too
            "(1 :> 2)", "(1 :> 2 @@ 3 :> 4)",
            "(1 :> {1, 2, 3, 4, 5, 6, 7, 8, 9, 10} @@ 2 :> {11, 12, 13, 14, 15, 16, 17})",
            "({1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11} :> {12, 13, 14, 15, 16, 17, 18, 19, 20})",
            "(\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" :> <<1, 2, 3, 4, 5, 6, 7, 8>>)",
            "({1, 2} :> (3 :> 4 @@ 5 :> 6) @@ [a |-> 7] :> <<8, 9, 10, 11, 12, 13, 14>>)",
            // SUBSET, whose pad is one space while its open delimiter is seven characters
            "SUBSET {1, 2}", "SUBSET {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16}",
            "SUBSET SUBSET {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18}",
            "{SUBSET {1, 2, 3, 4, 5}, SUBSET {6, 7, 8, 9, 10}, SUBSET {11, 12, 13, 14}}",
            // the terse texts of the lazy set classes, none of which Parse accepts
            "SUBSET (1..5)", "UNION({{1, 2, 3, 4, 5, 6, 7, 8}, {9, 10, 11, 12, 13, 14, 15}})",
            "[{1, 2, 3, 4, 5, 6, 7, 8, 9} -> {10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}]",
            "[a: {1, 2, 3, 4, 5, 6, 7, 8, 9, 10}, b: {11, 12, 13, 14, 15, 16, 17, 18, 19}]",
            "({1, 2, 3, 4, 5, 6, 7, 8, 9, 10} \\X {11, 12, 13, 14, 15, 16, 17, 18, 19, 20})",
            "({1, 2, 3, 4, 5, 6, 7, 8, 9, 10} \\cup {11, 12, 13, 14, 15, 16, 17, 18, 19})",
            "{x \\in {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12} : <expression x > 2 /\\ x < 9>}",
            "[x \\in 1..10 |-> <expression x + 1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10>]",
            // identifiers Parse cannot read, and text that stops short or runs off the end
            "{a_b, cccccccccc, dddddddddd, eeeeeeeeee, ffffffffff, gggggggggg, hhhhhhhh}",
            "[a_b |-> 1, cccccccccc |-> 2, dddddddddd |-> 3, eeeeeeeeee |-> 4, ff |-> 5]",
            "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,",
            "{", "}", "{1,,2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19}",
            "{1 @@ 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21}",
            "(1 :> 2, 3 :> 4, 5 :> 6, 7 :> 8, 9 :> 10, 11 :> 12, 13 :> 14, 15 :> 16, 17)",
            "<<1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21>",
            "\"an unterminated string constant that is wider than most of the widths here",
            // whitespace: extra spaces, a tab and a newline between elements, and a trailing
            // space, which stops the parse one character short of the end
            "{ 1 ,  2 ,  3 ,  4 ,  5 ,  6 ,  7 ,  8 ,  9 ,  10 ,  11 ,  12 ,  13 ,  14 }",
            "{1,\t2,\t3,\t4,\t5,\t6,\t7,\t8,\t9,\t10,\t11,\t12,\t13,\t14,\t15,\t16,\t17,\t18}",
            "{1,\n2,\n3,\n4,\n5,\n6,\n7,\n8,\n9,\n10,\n11,\n12,\n13,\n14,\n15,\n16,\n17,\n18}",
            " {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21}",
            "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21} ",
            // whitespace that is not a plain space: Character.isWhitespace takes the file
            // separator, the en quad and the ideographic space but not the next-line
            // character, the no-break space, the figure space or the narrow no-break space
            "{1,\u001c2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            "{1,　2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            "{1,\u00852, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            "{1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20}",
            // a character outside the BMP counts twice in a length, and neither of its two
            // UTF-16 code units is a letter, so a bare one cannot start a constant
            "{\"café\", \"naïve\"}", "[café |-> 1, naïve |-> 2]",
            "{\"𝐀\", \"b\"}", "[𝐀 |-> 1, b |-> 2]",
            // a field name may hold any letter or digit, and isDigit covers every decimal
            // digit, not just the ASCII ones
            "[١a |-> 1, bbbbbbbbbb |-> 2, cccccccccc |-> 3, dddddddddd |-> 4444444]",
            "[café |-> {1, 2, 3, 4, 5, 6, 7, 8}, naïve |-> <<9, 10, 11, 12, 13>>]",
        };
        try (PrintWriter w = open(p)) {
            for (String v : values) {
                TreeSet<Integer> widths = new TreeSet<>(List.of(0, 1, 2, 3, 7, 8, 10, 20, 40, 79, 80, 81));
                for (int d = -1; d <= 1; d++) widths.add(Math.max(0, v.length() + d));
                for (int width : widths) {
                    w.println(width + "\t" + esc(v) + "\t" + esc(PrettyPrint.mypp(v, width)));
                }
            }
        }
    }

    static String esc(String s) {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '\\': b.append("\\\\"); break;
                case '\n': b.append("\\n"); break;
                case '\t': b.append("\\t"); break;
                case '\r': b.append("\\r"); break;
                default: b.append(c);
            }
        }
        return b.toString();
    }

    // "<start>\t<ops>\t<keys in iteration order>". start: "new", "cap N", or "set k;k;..."
    // (new HashSet<>(list)). ops, comma separated: "p<k>" put, "r<k>" remove, "a<k;k;...>"
    // putAll of a new HashMap filled with those puts. Integer keys hash to themselves; some
    // collide (multiples of 16 and 64) so bins overflow and small tables grow in treeifyBin;
    // at most 8 distinct multiples of 64 keep tables of 64+ from treeifying (not modelled).
    static void hashMap(Path p) throws IOException {
        Random r = new Random(29);
        try (PrintWriter w = open(p)) {
            for (int c = 0; c < 400; c++) {
                int kind = r.nextInt(3);
                java.util.HashMap<Integer, Integer> m;
                String start;
                if (kind == 0) {
                    m = new java.util.HashMap<>();
                    start = "new";
                } else if (kind == 1) {
                    int cap = r.nextInt(40);
                    m = new java.util.HashMap<>(cap);
                    start = "cap " + cap;
                } else {
                    List<Integer> init = keys(r, r.nextInt(30));
                    java.util.HashSet<Integer> set = new java.util.HashSet<>(init);
                    // Read the set's map order back through a LinkedHashMap-free copy.
                    m = new java.util.HashMap<>();
                    StringBuilder b = new StringBuilder("set ");
                    for (int i = 0; i < init.size(); i++) b.append(i == 0 ? "" : ";").append(init.get(i));
                    start = b.toString();
                    // Replay into m below by recording the set's own order instead.
                    StringBuilder order = new StringBuilder();
                    for (int v : set) order.append(order.length() == 0 ? "" : ",").append(v);
                    w.println(start + "\t\t" + order);
                    continue;
                }
                StringBuilder ops = new StringBuilder();
                int nops = r.nextInt(c < 200 ? 20 : 120);
                for (int i = 0; i < nops; i++) {
                    if (ops.length() > 0) ops.append(',');
                    int op = r.nextInt(10);
                    if (op < 7) {
                        int k = key(r);
                        m.put(k, i);
                        ops.append('p').append(k);
                    } else if (op < 9) {
                        int k = m.isEmpty() || r.nextBoolean() ? key(r) : m.keySet().iterator().next();
                        m.remove(k);
                        ops.append('r').append(k);
                    } else {
                        java.util.HashMap<Integer, Integer> src = new java.util.HashMap<>();
                        List<Integer> ks = keys(r, r.nextInt(25));
                        StringBuilder b = new StringBuilder();
                        for (int k : ks) {
                            src.put(k, 0);
                            b.append(b.length() == 0 ? "" : ";").append(k);
                        }
                        m.putAll(src);
                        ops.append('a').append(b);
                    }
                }
                StringBuilder order = new StringBuilder();
                for (int v : m.keySet()) order.append(order.length() == 0 ? "" : ",").append(v);
                w.println(start + "\t" + ops + "\t" + order);
            }
        }
    }

    static int key(Random r) {
        switch (r.nextInt(4)) {
            case 0: return 16 * r.nextInt(12);
            case 1: return 64 * r.nextInt(8);
            case 2: return 2 * r.nextInt(30000) + 1;
            default: return r.nextInt(200);
        }
    }

    static List<Integer> keys(Random r, int n) {
        List<Integer> out = new ArrayList<>();
        for (int i = 0; i < n; i++) out.add(key(r));
        return out;
    }

    static PrintWriter open(Path p) throws IOException {
        return new PrintWriter(Files.newBufferedWriter(p));
    }

    // "<hash>\t<code points, comma separated>"
    static void stringHash(Path p) throws IOException {
        List<String> inputs = new ArrayList<>(List.of("", "a", "x", "Init", "Next", "TypeOK", "vars",
                "***Idonotexist***", "--TLA+BUILTINS--", "\\in", "ℕ", "∧", "日本語", "😀", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        Random r = new Random(1);
        for (int i = 0; i < 200; i++) {
            StringBuilder b = new StringBuilder();
            int len = r.nextInt(20);
            for (int j = 0; j < len; j++) {
                int kind = r.nextInt(10);
                int cp = kind < 7 ? 32 + r.nextInt(95) : kind < 9 ? 0xA0 + r.nextInt(0xD000) : 0x10000 + r.nextInt(0x1000);
                b.appendCodePoint(cp);
            }
            inputs.add(b.toString());
        }
        try (PrintWriter w = open(p)) {
            for (String s : inputs) {
                w.println(s.hashCode() + "\t" + codePoints(s));
            }
        }
    }

    static String codePoints(String s) {
        StringBuilder b = new StringBuilder();
        s.codePoints().forEach(cp -> {
            if (b.length() > 0) b.append(',');
            b.append(cp);
        });
        return b.toString();
    }

    // "case" line, then "put <key>" lines, then "order <k1>,<k2>,..." (elements() order).
    static void hashtable(Path p) throws IOException {
        Random r = new Random(7);
        try (PrintWriter w = open(p)) {
            for (int c = 0; c < 60; c++) {
                Hashtable<String, Integer> t = new Hashtable<>();
                w.println("case");
                int n = c < 10 ? c : r.nextInt(120);
                for (int i = 0; i < n; i++) {
                    String k = r.nextInt(4) == 0 && i > 0 ? "k" + r.nextInt(i) : "k" + i + "_" + r.nextInt(1000);
                    t.put(k, i);
                    w.println("put " + k);
                }
                StringBuilder b = new StringBuilder();
                for (Enumeration<String> e = t.keys(); e.hasMoreElements();) {
                    if (b.length() > 0) b.append(',');
                    b.append(e.nextElement());
                }
                w.println("order " + b);
            }
            // Cases with removals ("remove <k>") between puts.
            Random q = new Random(8);
            for (int c = 0; c < 40; c++) {
                Hashtable<String, Integer> t = new Hashtable<>();
                List<String> live = new ArrayList<>();
                w.println("case");
                int n = q.nextInt(150);
                for (int i = 0; i < n; i++) {
                    if (!live.isEmpty() && q.nextInt(3) == 0) {
                        String k = live.remove(q.nextInt(live.size()));
                        t.remove(k);
                        w.println("remove " + k);
                    } else {
                        String k = "r" + i + "_" + q.nextInt(1000);
                        t.put(k, i);
                        live.add(k);
                        w.println("put " + k);
                    }
                }
                StringBuilder b = new StringBuilder();
                for (Enumeration<String> e = t.keys(); e.hasMoreElements();) {
                    if (b.length() > 0) b.append(',');
                    b.append(e.nextElement());
                }
                w.println("order " + b);
            }
            // Cases with an initial capacity ("case <capacity>"), as new Hashtable<>(n).
            Random u = new Random(9);
            int[] capacities = {0, 1, 2, 7, 100, 200, 700};
            for (int c = 0; c < 35; c++) {
                int capacity = capacities[c % capacities.length];
                Hashtable<String, Integer> t = new Hashtable<>(capacity);
                w.println("case " + capacity);
                int n = u.nextInt(300);
                for (int i = 0; i < n; i++) {
                    String k = "c" + i + "_" + u.nextInt(1000);
                    t.put(k, i);
                    w.println("put " + k);
                }
                StringBuilder b = new StringBuilder();
                for (Enumeration<String> e = t.keys(); e.hasMoreElements();) {
                    if (b.length() > 0) b.append(',');
                    b.append(e.nextElement());
                }
                w.println("order " + b);
            }
        }
    }

    // "seed <s>" then "int <v>", "bounded <bound> <v>", "long <v>", "double <hex bits>".
    static void random(Path p) throws IOException {
        long[] seeds = {0, 1, 4, -1, 42, Long.MAX_VALUE, Long.MIN_VALUE, 0x5DEECE66DL, 123456789012345L};
        int[] bounds = {1, 2, 3, 7, 10, 16, 100, 1 << 30, (1 << 30) + 1, Integer.MAX_VALUE};
        try (PrintWriter w = open(p)) {
            for (long s : seeds) {
                Random r = new Random(s);
                w.println("seed " + s);
                for (int i = 0; i < 20; i++) {
                    w.println("int " + r.nextInt());
                    int b = bounds[i % bounds.length];
                    w.println("bounded " + b + " " + r.nextInt(b));
                    w.println("long " + r.nextLong());
                    w.println("double " + Long.toHexString(Double.doubleToRawLongBits(r.nextDouble())));
                }
                r.setSeed(s ^ 0x1234);
                w.println("reseed " + (s ^ 0x1234));
                w.println("int " + r.nextInt());
            }
        }
    }

    // "<hex bits>\t<Double.toString>"
    static void doubles(Path p) throws IOException {
        List<Double> vs = new ArrayList<>(List.of(0.0, -0.0, 1.0, -1.0, 0.1, 0.001, 0.0001, 1e7, 9999999.0, 1e-3,
                123456789.0, 1e21, 1e22, 1e23, 2e23, 5e-324, Double.MIN_NORMAL, Double.MAX_VALUE,
                Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, 1.0 / 3, 2.0 / 3, 100.0, 1e-10,
                5.421010862427522E-20, 1.6E-17, 2.7105054312137611E-19));
        Random r = new Random(3);
        for (int i = 0; i < 2000; i++) {
            vs.add(Double.longBitsToDouble(r.nextLong()));
        }
        for (int i = 0; i < 2000; i++) {
            vs.add((double) r.nextInt(1_000_000) * (1L << r.nextInt(20)) / Math.pow(2, 64));
        }
        for (int i = 0; i < 500; i++) {
            vs.add(r.nextDouble() * Math.pow(10, r.nextInt(40) - 20));
        }
        try (PrintWriter w = open(p)) {
            for (double v : vs) {
                w.println(Long.toHexString(Double.doubleToRawLongBits(v)) + "\t" + Double.toString(v));
            }
        }
    }

    // "<n>\t<hex bits of BigDecimal(1).divide(n, MathContext(2)).doubleValue()>"
    static void reciprocal(Path p) throws IOException {
        List<Long> ns = new ArrayList<>(List.of(1L, 2L, 3L, 6L, 7L, 9L, 10L, 11L, 15L, 95L, 96L, 99L, 100L, 101L, 995L, 996L,
                999L, 1000L, Long.MAX_VALUE, 1L << 40, 3L << 50));
        Random r = new Random(5);
        for (int i = 0; i < 500; i++) {
            ns.add(1 + (r.nextLong() >>> (1 + r.nextInt(62))));
        }
        try (PrintWriter w = open(p)) {
            for (long n : ns) {
                double d = BigDecimal.valueOf(1d).divide(BigDecimal.valueOf(n), new MathContext(2)).doubleValue();
                w.println(n + "\t" + Long.toHexString(Double.doubleToRawLongBits(d)));
            }
        }
    }

    // "<n>\t<DecimalFormat("###,###.###").format(n)>"
    static void decimalFormat(Path p) throws IOException {
        DecimalFormat df = new DecimalFormat("###,###.###");
        List<Long> ns = new ArrayList<>(List.of(0L, 1L, 12L, 123L, 999L, 1000L, 1234L, 999999L, 1000000L, -1L, -1234L,
                Long.MAX_VALUE, Long.MIN_VALUE));
        Random r = new Random(9);
        for (int i = 0; i < 300; i++) {
            ns.add(r.nextLong() >> r.nextInt(64));
        }
        try (PrintWriter w = open(p)) {
            for (long n : ns) {
                w.println(n + "\t" + df.format(n));
            }
        }
    }

    // "<a, comma separated>\t<b>\t<a.toString()>\t<(new HashSet<>(a)).retainAll(b).toString()>"
    // with a and b built by add() in the listed order.
    static void intHashSet(Path p) throws IOException {
        Random r = new Random(13);
        try (PrintWriter w = open(p)) {
            for (int c = 0; c < 300; c++) {
                int n = r.nextInt(c < 100 ? 8 : 60);
                List<Integer> a = new ArrayList<>();
                List<Integer> b = new ArrayList<>();
                for (int i = 0; i < n; i++) a.add(r.nextBoolean() ? 1000 + r.nextInt(1600) : 4000 + r.nextInt(900));
                for (int i = 0; i < n; i++) b.add(r.nextBoolean() && !a.isEmpty() ? a.get(r.nextInt(a.size())) : 1000 + r.nextInt(4000));
                java.util.HashSet<Integer> sa = new java.util.HashSet<>();
                for (int v : a) sa.add(v);
                java.util.HashSet<Integer> sb = new java.util.HashSet<>();
                for (int v : b) sb.add(v);
                java.util.HashSet<Integer> inter = new java.util.HashSet<>(sa);
                inter.retainAll(sb);
                w.println(join(a) + "\t" + join(b) + "\t" + sa + "\t" + inter);
            }
        }
    }

    static String join(List<Integer> xs) {
        StringBuilder s = new StringBuilder();
        for (int x : xs) {
            if (s.length() > 0) s.append(',');
            s.append(x);
        }
        return s.toString();
    }

    // "<pattern>\t<offset minutes>\t<epoch millis>\t<formatted>"
    static void dates(Path p) throws IOException {
        String[] patterns = {"yyyy-MM-dd HH:mm:ss", "D'd' HH'h'", "HH'h' mm'min'", "mm'min' ss's'", "ss's'",
                "yy-MM-dd-HH-mm-ss.SSS", "yy-MM-dd-HH-mm-ss"};
        int[] offsets = {0, 540, -300, 330};
        List<Long> ms = new ArrayList<>(List.of(0L, 999L, 59_999L, 3_599_999L, 86_399_999L, 86_400_000L,
                31L * 86_400_000L, 1_763_380_000_000L, 951_782_400_000L, 4_102_444_800_000L, -1L));
        Random r = new Random(11);
        for (int i = 0; i < 200; i++) {
            ms.add((r.nextLong() >>> 1) % 4_102_444_800_000L);
        }
        try (PrintWriter w = open(p)) {
            for (String pat : patterns) {
                for (int off : offsets) {
                    SimpleDateFormat f = new SimpleDateFormat(pat);
                    f.setTimeZone(TimeZone.getTimeZone(java.time.ZoneOffset.ofTotalSeconds(off * 60)));
                    for (long m : ms) {
                        w.println(pat + "\t" + off + "\t" + m + "\t" + f.format(m));
                    }
                }
            }
        }
    }
}
