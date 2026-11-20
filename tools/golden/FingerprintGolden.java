// Writes crates/tlars-tool/tests/golden/fingerprint.txt from TLC's own
// tlc2.value.impl.*.fingerPrint(long).
// Usage (inside `nix develop`):
//   javac -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/FingerprintGolden.java
//   java -cp "target/golden:$TLA2TOOLS_JAR" FingerprintGolden \
//       crates/tlars-tool/tests/golden/fingerprint.txt
//
// Line format: "tok\t<n>\t<model value name>" for the intern tokens the model values got,
// then "<poly index>\t<fingerprint hex>\t<value.toString()>" for each value under each poly.
// `matches_tlc_fingerprint` in crates/tlars-tool/src/value/fingerprint.rs builds the same
// list in the same order; the printed form is on the line so a drift in either list shows up
// as a label mismatch instead of a silent re-pairing.

import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import tlc2.util.FP64;
import tlc2.value.impl.BoolValue;
import tlc2.value.impl.FcnRcdValue;
import tlc2.value.impl.IntValue;
import tlc2.value.impl.IntervalValue;
import tlc2.value.impl.ModelValue;
import tlc2.value.impl.RecordValue;
import tlc2.value.impl.SetCapValue;
import tlc2.value.impl.SetCupValue;
import tlc2.value.impl.SetDiffValue;
import tlc2.value.impl.SetEnumValue;
import tlc2.value.impl.SetOfFcnsValue;
import tlc2.value.impl.SetOfRcdsValue;
import tlc2.value.impl.SetOfTuplesValue;
import tlc2.value.impl.StringValue;
import tlc2.value.impl.SubsetValue;
import tlc2.value.impl.TupleValue;
import tlc2.value.impl.UnionValue;
import tlc2.value.impl.Value;
import util.UniqueString;

public class FingerprintGolden {
    private static final int[] POLYS = {0, 17};
    private static final String[] MODEL_VALUES = {"a", "bb", "t_1", "zz"};

    public static void main(String[] args) throws IOException {
        final ModelValue[] mvs = new ModelValue[MODEL_VALUES.length];
        for (int i = 0; i < MODEL_VALUES.length; i++) {
            mvs[i] = (ModelValue) ModelValue.make(MODEL_VALUES[i]);
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (ModelValue mv : mvs) {
                w.println("tok\t" + mv.val.getTok() + "\t" + mv.val);
            }
            for (int poly : POLYS) {
                FP64.Init(poly);
                for (Value v : values(mvs)) {
                    // The printed form first: fingerPrint normalizes, and the Rust side
                    // prints in the same order so both labels describe the same value.
                    String shown = v.toString();
                    w.println(poly + "\t" + Long.toHexString(v.fingerPrint(FP64.New())) + "\t" + shown);
                }
            }
        }
    }

    private static UniqueString[] names(String... ns) {
        UniqueString[] us = new UniqueString[ns.length];
        for (int i = 0; i < ns.length; i++) {
            us[i] = UniqueString.uniqueStringOf(ns[i]);
        }
        return us;
    }

    private static Value[] vals(Value... vs) {
        return vs;
    }

    private static Value set(Value... vs) {
        return new SetEnumValue(vs, false);
    }

    private static List<Value> values(ModelValue[] mv) {
        List<Value> l = new ArrayList<>();
        l.add(BoolValue.ValTrue);
        l.add(BoolValue.ValFalse);
        l.add(IntValue.gen(0));
        l.add(IntValue.gen(1));
        l.add(IntValue.gen(-1));
        l.add(IntValue.gen(255));
        l.add(IntValue.gen(256));
        l.add(IntValue.gen(Integer.MAX_VALUE));
        l.add(IntValue.gen(Integer.MIN_VALUE));
        l.add(new StringValue(""));
        l.add(new StringValue("a"));
        l.add(new StringValue("abc"));
        l.add(new StringValue("aa"));
        // Only the low byte of each UTF-16 unit is fingerprinted, so these two differ in
        // length alone.
        l.add(new StringValue("ĀA"));
        l.add(new StringValue("\u0000A"));
        l.add(mv[0]);
        l.add(mv[2]);
        l.add(mv[3]);
        // Tuples are functions with the keys 1..n.
        l.add(new TupleValue(vals()));
        l.add(new TupleValue(vals(IntValue.gen(1))));
        l.add(new TupleValue(vals(IntValue.gen(1), new StringValue("a"), BoolValue.ValTrue)));
        l.add(new TupleValue(vals(new TupleValue(vals(IntValue.gen(1))), set())));
        // A record is a function with STRING keys, fingerprinted in token order, so the
        // unsorted one must agree with the sorted one.
        l.add(new RecordValue(names("bb", "a"), vals(IntValue.gen(1), IntValue.gen(2)), false));
        l.add(new RecordValue(names("a", "bb"), vals(IntValue.gen(2), IntValue.gen(1)), false));
        l.add(new RecordValue(names("zz", "a", "bb"), vals(set(IntValue.gen(3), IntValue.gen(1)), mv[1], new StringValue("x")), false));
        l.add(set());
        l.add(set(IntValue.gen(1)));
        l.add(set(IntValue.gen(3), IntValue.gen(1), IntValue.gen(2), IntValue.gen(1)));
        l.add(new SetEnumValue(vals(IntValue.gen(1), IntValue.gen(2), IntValue.gen(3)), true));
        l.add(set(new StringValue("b"), new StringValue("a")));
        l.add(set(mv[1], mv[0]));
        l.add(set(BoolValue.ValTrue, BoolValue.ValFalse));
        l.add(set(set(IntValue.gen(2), IntValue.gen(1)), set()));
        // An interval fingerprints as the set of its elements.
        l.add(new IntervalValue(1, 3));
        l.add(new IntervalValue(0, 0));
        l.add(new IntervalValue(5, 4));
        l.add(new IntervalValue(-2, 2));
        l.add(set(IntValue.gen(2), IntValue.gen(1), IntValue.gen(3)));
        // An explicit domain is fingerprinted sorted; an interval domain writes INTVALUE keys.
        l.add(new FcnRcdValue(vals(new StringValue("bb"), new StringValue("a")), vals(IntValue.gen(1), IntValue.gen(2)), false));
        l.add(new FcnRcdValue(vals(IntValue.gen(2), IntValue.gen(1)), vals(new StringValue("b"), new StringValue("a")), false));
        l.add(new FcnRcdValue(new IntervalValue(1, 3), vals(IntValue.gen(7), IntValue.gen(8), IntValue.gen(9))));
        l.add(new FcnRcdValue(new IntervalValue(-1, 1), vals(IntValue.gen(7), IntValue.gen(8), IntValue.gen(9))));
        l.add(new FcnRcdValue(vals(IntValue.gen(1), IntValue.gen(2), IntValue.gen(3)), vals(IntValue.gen(7), IntValue.gen(8), IntValue.gen(9)), true));
        // The lazy set classes convert first, so they agree with the enumerated set.
        l.add(new SubsetValue(set(IntValue.gen(1), IntValue.gen(2))));
        l.add(new SetCupValue(set(IntValue.gen(1)), set(IntValue.gen(2), IntValue.gen(1))));
        l.add(new SetCapValue(set(IntValue.gen(1), IntValue.gen(2)), set(IntValue.gen(2), IntValue.gen(3))));
        l.add(new SetDiffValue(set(IntValue.gen(1), IntValue.gen(2)), set(IntValue.gen(2))));
        l.add(new UnionValue(set(set(IntValue.gen(1)), set(IntValue.gen(2), IntValue.gen(1)))));
        l.add(new SetOfFcnsValue(set(IntValue.gen(1), IntValue.gen(2)), set(BoolValue.ValFalse, BoolValue.ValTrue)));
        l.add(new SetOfTuplesValue(vals(set(IntValue.gen(1), IntValue.gen(2)), set(new StringValue("a")))));
        l.add(new SetOfRcdsValue(names("a", "bb"), vals(set(IntValue.gen(1)), set(IntValue.gen(2), IntValue.gen(3))), false));
        return l;
    }
}
