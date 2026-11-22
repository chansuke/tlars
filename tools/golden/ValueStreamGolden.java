// Writes crates/tlars-tool/tests/golden/value_stream.txt from TLC's own
// tlc2.value.ValueOutputStream and the write(IValueOutputStream) of every value class.
// Usage (inside `nix develop`):
//   javac -d target/golden -cp "$TLA2TOOLS_JAR" tools/golden/ValueStreamGolden.java
//   java -cp "target/golden:$TLA2TOOLS_JAR" ValueStreamGolden \
//       crates/tlars-tool/tests/golden/value_stream.txt
//
// Line formats, in this order:
//   "mv\t<tok>\t<name>"          the intern token of each model value, in creation order
//   "tok\t<tok>\t<text>"         the intern token of every other string the cases use
//   "nat\t<n>\t<hex>"            ValueOutputStream.writeNat of n
//   "val\t<hex>\t<toString>\t<hex again>\t<toString again>"
//   "raw\t<hex>\t<toString>\t<hex again>\t<toString again>"
// where a value case carries the bytes of the value (fingerprinted first as ioSerialize does,
// or for "raw" written as constructed so that it is not normalized), its printed form, and
// then the bytes and printed form of what ValueInputStream.readExternal reads back from those
// bytes and ValueOutputStream writes again. The second pass is not always the first: a record
// read externally is not normalized, and a string is only the bytes String.getBytes kept.
// `matches_tlc_value_stream` in crates/tlars-tool/src/value/stream.rs builds the same lists
// in the same order; each value's printed form is on its line, so a list that drifted out of
// step fails on the label instead of comparing bytes of two different values.
//
// Object identity decides the DUMMYVALUE back-references the stream writes, and tlars'
// Value::Str carries an intern token rather than an identity, so no two StringValues here
// share an object: a string is written out in full at every occurrence, which is what the
// port does. Containers are shared on purpose, because an Arc in tlars is an identity the
// port keys the handle table by, exactly as the Java stream keys it by the object.

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import tlc2.value.ValueOutputStream;
import tlc2.value.impl.BoolValue;
import tlc2.value.impl.FcnRcdValue;
import tlc2.value.impl.IntValue;
import tlc2.value.impl.IntervalValue;
import tlc2.value.impl.ModelValue;
import tlc2.value.impl.RecordValue;
import tlc2.value.impl.SetEnumValue;
import tlc2.value.impl.StringValue;
import tlc2.value.impl.SubsetValue;
import tlc2.value.impl.TupleValue;
import tlc2.value.impl.Value;
import util.UniqueString;

public class ValueStreamGolden {
    private static final String[] MODEL_VALUES = {"a", "bb", "t_1", "zz"};
    private static final int[] NATS = {0, 1, 0x7f, 0x80, 0x7fff, 0x8000, 0x10000, Integer.MAX_VALUE};

    // Every string the value cases use, interned here so the golden can hand their tokens to
    // the Rust side; a token depends on the order the whole process interns in.
    private static final String[] STRINGS = {
        // The bytes of "ĀA" and "\u0080ÿ" read back as another string ("\u0000A"
        // and "ﾀ￿"), because writeString keeps the low byte of each UTF-16 unit and
        // readString sign-extends it. Both of those are in this list too, so the token the
        // read assigns is one this golden recorded and not one that depends on whatever else
        // the process has interned.
        "", "x", "abc", "aa", "ĀA", "\u0000A", "\u0080ÿ", "ﾀ￿", "bb", "zz", "q", "field", "name",
        "other",
    };

    public static void main(String[] args) throws IOException {
        ModelValue[] mvs = new ModelValue[MODEL_VALUES.length];
        for (int i = 0; i < MODEL_VALUES.length; i++) {
            mvs[i] = (ModelValue) ModelValue.make(MODEL_VALUES[i]);
        }
        // `Spec` calls this once the cfg's model values are made; `ModelValue.mvs` is what
        // the stream reads a MODELVALUE index out of.
        ModelValue.setValues();
        Map<String, UniqueString> interned = new LinkedHashMap<>();
        for (String s : STRINGS) {
            interned.put(s, UniqueString.uniqueStringOf(s));
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (ModelValue mv : mvs) {
                w.println("mv\t" + mv.val.getTok() + "\t" + mv.val);
            }
            for (Map.Entry<String, UniqueString> e : interned.entrySet()) {
                w.println("tok\t" + e.getValue().getTok() + "\t" + escape(e.getKey()));
            }
            for (int n : NATS) {
                ByteArrayOutputStream out = new ByteArrayOutputStream();
                ValueOutputStream vos = new ValueOutputStream(out, false);
                vos.writeNat(n);
                vos.close();
                w.println("nat\t" + n + "\t" + hex(out.toByteArray()));
            }
            // The bytes are taken first and then reused, because toString normalizes a set in
            // place and would otherwise change what the second pass reads. The Rust side
            // writes, prints and reads back in this same order.
            for (Value v : fingerprinted(mvs)) {
                byte[] bytes = pickle(v, true);
                w.println("val\t" + hex(bytes) + "\t" + escape(v.toString()) + again(bytes));
            }
            for (Value v : raw(mvs)) {
                byte[] bytes = pickle(v, false);
                w.println("raw\t" + hex(bytes) + "\t" + escape(v.toString()) + again(bytes));
            }
        }
    }

    // tlc2.module._TLCTrace.ioSerialize without the gzip wrapper: fingerPrint(0) and then
    // write, each value into a stream of its own so it starts with an empty handle table.
    private static byte[] pickle(Value v, boolean fingerprint) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        ValueOutputStream vos = new ValueOutputStream(out, false);
        try {
            if (fingerprint) {
                v.fingerPrint(0L);
            }
            v.write(vos);
        } finally {
            vos.close();
        }
        return out.toByteArray();
    }

    // What tlc2.module._TLCTrace.ioDeserialize reads from these bytes, written out again.
    private static String again(byte[] bytes) throws IOException {
        tlc2.value.ValueInputStream vis = new tlc2.value.ValueInputStream(new java.io.ByteArrayInputStream(bytes));
        Value read;
        try {
            read = (Value) vis.readExternal();
        } finally {
            vis.close();
        }
        return "\t" + hex(pickle(read, false)) + "\t" + escape(read.toString());
    }

    private static List<Value> fingerprinted(ModelValue[] mv) {
        List<Value> l = new ArrayList<>();
        l.add(BoolValue.ValTrue);
        l.add(BoolValue.ValFalse);
        for (int n : new int[] {0, 1, -1, 127, 128, 32767, 32768, Integer.MAX_VALUE, Integer.MIN_VALUE}) {
            l.add(IntValue.gen(n));
        }
        for (String s : STRINGS) {
            l.add(new StringValue(s));
        }
        l.add(mv[0]);
        l.add(mv[1]);
        l.add(mv[3]);
        l.add(new IntervalValue(1, 3));
        l.add(new IntervalValue(5, 4));
        l.add(new IntervalValue(-2, 2));
        l.add(new IntervalValue(0, 0));
        l.add(new TupleValue(vals()));
        l.add(new TupleValue(vals(IntValue.gen(1))));
        // Two StringValues of the same text, each written in full: equal is not identical.
        l.add(new TupleValue(vals(new StringValue("x"), new StringValue("x"))));
        l.add(new TupleValue(vals(new TupleValue(vals(IntValue.gen(1))), set(IntValue.gen(2)))));
        // The same tuple twice: the second occurrence is a DUMMYVALUE back-reference.
        Value shared = new TupleValue(vals(IntValue.gen(7), new StringValue("q")));
        l.add(new TupleValue(vals(shared, IntValue.gen(0), shared)));
        l.add(new RecordValue(names("x"), vals(IntValue.gen(1)), false));
        l.add(new RecordValue(names("bb", "x"), vals(IntValue.gen(1), IntValue.gen(2)), false));
        // A field name is a UniqueString and a string value is a StringValue, so "x" here is
        // written out twice although both come from the same intern table entry.
        l.add(new RecordValue(names("x"), vals(new StringValue("x")), false));
        // Two records sharing their field names: the names are the same UniqueStrings, so the
        // second record's names are back-references while the records themselves are not.
        Value first = new RecordValue(names("field", "other"), vals(IntValue.gen(1), IntValue.gen(2)), false);
        Value second = new RecordValue(names("field", "other"), vals(IntValue.gen(3), IntValue.gen(4)), false);
        l.add(new TupleValue(vals(first, second)));
        Value sharedRecord = new RecordValue(names("name"), vals(new StringValue("abc")), false);
        l.add(new TupleValue(vals(sharedRecord, sharedRecord)));
        l.add(set());
        l.add(set(IntValue.gen(1)));
        l.add(set(IntValue.gen(3), IntValue.gen(1), IntValue.gen(2)));
        l.add(set(new StringValue("zz"), new StringValue("aa")));
        l.add(set(set(IntValue.gen(2), IntValue.gen(1)), set()));
        l.add(set(mv[1], mv[0]));
        Value sharedSet = set(IntValue.gen(5), IntValue.gen(6));
        l.add(new TupleValue(vals(sharedSet, sharedSet)));
        l.add(new FcnRcdValue(vals(new StringValue("bb"), new StringValue("aa")), vals(IntValue.gen(1), IntValue.gen(2)), false));
        l.add(new FcnRcdValue(vals(IntValue.gen(2), IntValue.gen(1)), vals(new StringValue("q"), new StringValue("x")), false));
        l.add(new FcnRcdValue(new IntervalValue(1, 3), vals(IntValue.gen(7), IntValue.gen(8), IntValue.gen(9))));
        l.add(new FcnRcdValue(new IntervalValue(-1, 1), vals(IntValue.gen(7), IntValue.gen(8), IntValue.gen(9))));
        // A lazy set converts to the SetEnumValue it caches, and that one object carries the
        // handle: the second occurrence of the subset is a back-reference to it.
        Value subset = new SubsetValue(set(IntValue.gen(1), IntValue.gen(2)));
        l.add(subset);
        l.add(new TupleValue(vals(subset, subset)));
        // Deep nesting, the shape a trace has: a record of a set of pairs of records.
        Value state = new RecordValue(names("x"), vals(IntValue.gen(0)), false);
        Value pair = new TupleValue(vals(IntValue.gen(1), state));
        l.add(new RecordValue(names("name", "other"), vals(set(pair), new StringValue("abc")), false));
        return l;
    }

    // Values written without fingerprinting them first, which is the only way the stream
    // writes an unnormalized record, set or function (a negative length, or the info byte 2).
    private static List<Value> raw(ModelValue[] mv) {
        List<Value> l = new ArrayList<>();
        l.add(new RecordValue(names("zz", "aa"), vals(IntValue.gen(1), IntValue.gen(2)), false));
        l.add(new RecordValue(names("aa", "zz"), vals(IntValue.gen(1), IntValue.gen(2)), true));
        l.add(set(IntValue.gen(3), IntValue.gen(1), IntValue.gen(3)));
        l.add(new SetEnumValue(vals(IntValue.gen(1), IntValue.gen(2)), true));
        l.add(new FcnRcdValue(vals(IntValue.gen(2), IntValue.gen(1)), vals(IntValue.gen(3), IntValue.gen(4)), false));
        l.add(new FcnRcdValue(vals(IntValue.gen(1), IntValue.gen(2)), vals(IntValue.gen(3), IntValue.gen(4)), true));
        l.add(new TupleValue(vals(set(IntValue.gen(2), IntValue.gen(1)), mv[2])));
        return l;
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

    private static String hex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            sb.append(String.format("%02x", b));
        }
        return sb.toString();
    }

    // Backslash, tab, newline and the code point of anything outside printable ASCII, so a
    // label stays one field on one line.
    private static String escape(String s) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\\') {
                sb.append("\\\\");
            } else if (c == '\t') {
                sb.append("\\t");
            } else if (c == '\n') {
                sb.append("\\n");
            } else if (c < 0x20 || c > 0x7e) {
                sb.append(String.format("\\u%04x", (int) c));
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }
}
