// Writes crates/tlars-msg/tests/golden/mp.txt from TLC's own tlc2.output.MP.
// Usage (inside `nix develop`):
//   javac -cp "$(dirname $(which tlc))/../share/tla2tools/tla2tools.jar" -d target/golden tools/golden/MpGolden.java
//   tla2tools-java -Dtlc2.output.MP.noTimestamps=true -cp target/golden:<tla2tools.jar> MpGolden crates/tlars-msg/tests/golden/mp.txt
//
// Line format: "<code>\t<class>\t<tool 0|1>\t<param count>\t<message>", the message with
// backslash and newline escaped as \\ and \n. Parameters are "P1", "P2", ...; 16 parameters
// cover the startup banners. TLC's warning history is empty, so non-tool warnings carry the
// first-warning hint. Combinations where TLC itself throws are omitted.

import java.io.IOException;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;
import tlc2.TLCGlobals;
import tlc2.output.EC;
import tlc2.output.MP;

public class MpGolden {
    public static void main(String[] args) throws Exception {
        int[] paramCounts = {0, 1, 2, 3, 4, 6, 16};
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (Field f : EC.class.getDeclaredFields()) {
                int m = f.getModifiers();
                if (!Modifier.isStatic(m) || f.getType() != int.class) continue;
                int code = f.getInt(null);
                for (int tool = 0; tool <= 1; tool++) {
                    TLCGlobals.tool = tool == 1;
                    for (int cls = 0; cls <= 4; cls++) {
                        for (int n : paramCounts) {
                            String[] params = new String[n];
                            for (int i = 0; i < n; i++) params[i] = "P" + (i + 1);
                            String msg;
                            try {
                                msg = MP.getMessage(cls, code, params);
                            } catch (RuntimeException e) {
                                // The startup banners index parameters directly; TLC always passes all.
                                continue;
                            }
                            w.println(code + "\t" + cls + "\t" + tool + "\t" + n + "\t"
                                    + msg.replace("\\", "\\\\").replace("\n", "\\n"));
                        }
                    }
                }
            }
        }
    }
}
