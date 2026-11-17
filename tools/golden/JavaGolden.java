// Writes golden files for crates/tlars-java from the running JDK.
// Usage: java tools/golden/JavaGolden.java crates/tlars-java/tests/golden
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
