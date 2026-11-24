// Reads trace files with TLC's own tlc2.value.ValueInputStream (gzip included, as
// tlc2.module._TLCTrace.ioDeserialize opens them) and prints each one's value, so a file
// tlars wrote and a file TLC wrote can be compared through TLC's reader.
// Usage (inside `nix develop`): see tools/golden/trace_interop.sh.

import tlc2.value.ValueInputStream;
import tlc2.value.impl.ModelValue;
import tlc2.value.impl.Value;

import java.io.File;

public class TraceInterop {
    public static void main(String[] args) throws Exception {
        // A MODELVALUE in the file is an index into `ModelValue.mvs`, which a cfg would have
        // filled: `-mv a,b` makes those model values in that order first.
        int first = 0;
        if (args.length > 1 && args[0].equals("-mv")) {
            for (String name : args[1].split(",")) {
                ModelValue.make(name);
            }
            first = 2;
        }
        ModelValue.setValues();
        for (String path : java.util.Arrays.copyOfRange(args, first, args.length)) {
            ValueInputStream vis = new ValueInputStream(new File(path), true);
            try {
                Value v = (Value) vis.readExternal();
                System.out.println(path + "\t" + v);
            } finally {
                vis.close();
            }
        }
    }
}
