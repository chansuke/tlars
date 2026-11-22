// Prints TLC's `UniqueString` intern table after a run, in token order, so that tlars'
// intern sequence can be compared with it (tools/golden/intern_dump.sh).
// The table is the one `_TLCTrace.ioDeserialize` reads through `UniqueString.internTbl`.

import java.util.*;
public class InternDump {
  public static void main(String[] args) throws Exception {
    tlc2.TLC tlc = new tlc2.TLC();
    tlc.handleParameters(args);
    tlc.process();
    Map<String, util.UniqueString> m = util.UniqueString.internTbl.toMap();
    List<Map.Entry<String, util.UniqueString>> l = new ArrayList<>(m.entrySet());
    l.sort(Comparator.comparingInt(e -> e.getValue().getTok()));
    for (Map.Entry<String, util.UniqueString> e : l) {
      System.err.println("INTERN " + e.getValue().getTok() + " " + quote(e.getKey()));
    }
  }
  static String quote(String s) {
    StringBuilder sb = new StringBuilder("\"");
    for (char c : s.toCharArray()) {
      if (c == '"' || c == '\\') sb.append('\\').append(c);
      else if (c == '\n') sb.append("\\n");
      else if (c < 0x20 || c > 0x7e) sb.append(String.format("\\u{%x}", (int) c));
      else sb.append(c);
    }
    return sb.append('"').toString();
  }
}
