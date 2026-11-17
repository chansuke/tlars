// Writes crates/tlars-tool/tests/golden/fp64.txt from TLC's own tlc2.util.FP64.
// Usage (inside `nix develop`):
//   javac -d target/golden $TLAPLUS_SRC/tlatools/org.lamport.tlatools/src/tlc2/util/FP64.java tools/golden/Fp64Golden.java
//   java -cp target/golden Fp64Golden crates/tlars-tool/tests/golden/fp64.txt
//
// Line format: "<poly index>\t<op>\t<argument>\t<fingerprint hex>". Ops are applied in
// sequence per poly index, starting from FP64.New(): "new", "byte", "int", "long", "str"
// (string argument as comma-separated UTF-16 units).

import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Random;
import tlc2.util.FP64;

public class Fp64Golden {
    public static void main(String[] args) throws IOException {
        int[] indices = {0, 1, 2, 17, 64, 129, 130};
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (int idx : indices) {
                FP64.Init(idx);
                long fp = FP64.New();
                w.println(idx + "\tnew\t-\t" + Long.toHexString(fp));
                Random r = new Random(idx);
                for (int i = 0; i < 200; i++) {
                    switch (r.nextInt(4)) {
                        case 0 -> {
                            byte b = (byte) r.nextInt();
                            fp = FP64.Extend(fp, b);
                            w.println(idx + "\tbyte\t" + b + "\t" + Long.toHexString(fp));
                        }
                        case 1 -> {
                            int x = r.nextInt();
                            fp = FP64.Extend(fp, x);
                            w.println(idx + "\tint\t" + x + "\t" + Long.toHexString(fp));
                        }
                        case 2 -> {
                            long x = r.nextLong();
                            fp = FP64.Extend(fp, x);
                            w.println(idx + "\tlong\t" + x + "\t" + Long.toHexString(fp));
                        }
                        default -> {
                            StringBuilder s = new StringBuilder();
                            int len = r.nextInt(12);
                            for (int j = 0; j < len; j++) {
                                s.append((char) (r.nextBoolean() ? 32 + r.nextInt(95) : r.nextInt(0xD800)));
                            }
                            fp = FP64.Extend(fp, s.toString());
                            StringBuilder units = new StringBuilder();
                            for (int j = 0; j < s.length(); j++) {
                                if (j > 0) units.append(',');
                                units.append((int) s.charAt(j));
                            }
                            w.println(idx + "\tstr\t" + units + "\t" + Long.toHexString(fp));
                        }
                    }
                }
            }
        }
    }
}
