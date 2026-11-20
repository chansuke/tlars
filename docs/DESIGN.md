# tlars 設計 (統合版): TLC の pure Rust 完全再実装

対象: tlaplus `bc1b506` (2026-11-14) の `java tlc2.TLC`。参照実装として tla-ts `a81d456` を比較した。
スケッチは `sketch/` にあり、std のみの 8 crate で約 4.2k 行。`cargo check --offline --workspace --all-targets` は警告 0、`cargo clippy -- -D clippy::disallowed_types` はエラー 0。

## Problem

tlars は、tla-ts の適合性コーパス (MIT、61,542 件、`-tool -workers 1 -fp 0 -seed 4 -lncheck final` で記録) から見て `java tlc2.TLC` と区別できないことを目標にする。CLI、`.cfg`、意味論、状態数、反例 trace、`-tool` の枠付き出力、終了コードがすべて一致しなければならない。

TLA+ の意味論そのものは難所ではない。出力のバイト列を決めているのは Java の偶発的な挙動で、Phase A で次を一次情報から確認した。

- **文字列と model value の順序は intern 順。** 比較は `UniqueString.compareTo` = `tok - tok` で、intern の順序は cfg → SANY の startup 文字列 → 字句順 → クラス初期化のタイミング、で決まる。
- **変数・定数・`I!Op` の順序は宣言順ではない。** `java.util.Hashtable` を列挙した順の逆で、`Context.java:320` と `ModuleNode.java:400` が該当する。この順序は fingerprint、symmetry、状態の印字すべてに効く。
- **FP64 には種別ごとの符号化規則がある。** 例として、文字列は UTF-16 単位の下位 1 byte だけを使い、tuple は FCNRCD として扱う。FPSet は 63 bit マスクで管理する。
- **値クラスごとに列挙順が違う。** 例えば SetCup は重複を出し、SetOfFcns は最後の要素が最速で変わる。`toString` の結果も値の表現に依存し、`1..3` と `{1, 2, 3}` は印字が異なる。
- **`Worker.addElement` での検査の順番と計数方法が出力に効く。**
  - generated の数は呼び出しのたびに増える。
  - モデル外の successor は毎回 invariant を検査する。
  - implied action は常に検査する。
  - deadlock は「呼び出しが 0 回」のときを指す。
- **trace 再構成は fingerprint が一致した最初の action を採る。** 表示されるラベルはこれで決まる。
- **RandomElement は親状態ごとに再シードする。** シードは `fp(parent) ^ seed` で 1 回だけ設定する (`RandomEnumerableValues.java:136-160`)。trace 再生も `IdThread.setCurrentState` を通るので同じ規則になる (`Tool.java:3786`)。
- **liveness の `SetOfStates` はスロット順に辺を並べる。** 重複判定は状態の等値で行い、比較中の例外は握りつぶす。親状態は最後に stutter として加える。容量の multiplier は worker ごとに持つ。
- **pprint は値を一度文字列にしてから再パースし、幅 80 で折り返す。**
- **MP/EC には固有の規則がある。**
  - メッセージの class は呼び出し側が決める。
  - 最初の warning にだけ案内文を付ける。
  - `%i%` の置換は null で止まる。
- **SANY の構文解析には固有の挙動がある。**
  - 定義の区切りは DEFBREAK の先読みで見つける。
  - junction list は列位置で判定し、tab は 8 桁単位で数える。
  - 演算子の優先度は範囲で比較する。
  - JavaCC のエラー文を出す。
  - `matchFcnConst` のバグもそのまま持っている。

tla-ts は、パーサーを 2 本 (通常用とエラー文用の 2,083 行の JavaCC エミュレーター) 持ち、型付き compiler も別に持ち、並列実行で問題が出たら逐次でやり直す、という形でこれらを再現している。tlars はそれを繰り返さない。

## Usage (caller's view)

```
$ tlars [-D<prop>=<v> ...] [TLC options] Spec.tla
$ tlars -tool -workers 1 -fp 0 -seed 4 -lncheck final -noGenerateSpecTE -config MC.cfg MC.tla
$ echo $status   # 0/10/11/12/13/14/75/76/77/150/151/255 (EC.ExitStatus と同一)
```

```rust
// crates/tlars/src/main.rs (バイナリ全体)
let status = tlars_check::run(tlars_check::Invocation::from_process(args));
std::process::exit(status.code());

// crates/tlars-conformance: 1 ケースをプロセス内で hermetic に実行
let status = tlars_check::run(Invocation {
    args: case.tlc_args(ctx),
    system_properties: vec![("tlc2.output.MP.noTimestamps".into(), "true".into())],
    cwd, stdout: Box::new(&mut out), stderr: Box::new(std::io::sink()),
    env: Env::hermetic(),               // 時計、pid、乱数源を固定。プロセス global は持たない
});
let verdict = compare(&expected, status, &out, ctx.is_some());   // transcript.ts の移植

// tlars-check/src/bfs/worker.rs (Worker.addElement)。行の順序そのものが TLC の観測される挙動
let in_model = ev.is_in_model(succ)? && ev.is_in_actions(s0, succ)?;
let mut unseen = true;                                   // モデル外は毎回 invariant を検査する
if in_model {
    let fp = ev.fingerprint(succ)?;
    unseen = self.seen.seen(fp) == Seen::New;            // SeenOracle: v1 は共有 FPSet の put
    ...
}
if unseen { mc.check_invariants(ev, self.cur, succ)?; }
mc.check_implied_actions(ev, self.cur, succ)?;           // 既出の状態でも必ず検査する
```

## Shape

### Crate 構成 (依存は下向き)

```
tlars (bin)                     tlars-conformance (corpus runner)
      \                          /
       tlars-check   run(): TLC.java, ModelChecker/Worker, FPSet, queue, trace, liveness graph, DFID, simulate
            |
       tlars-tool    SpecProcessor, Tool (eval/init/next/ENABLED/replay), Value, FP64, LiveSpec, natives, Effects
            |
   tlars-sany   tlars-msg    SANY + cfg + Interner  |  EC/MP catalog, framing, exit map
            \     /
          tlars-java        Java/TLC utility の再現 (Hashtable, String.hashCode, Random, Double.toString, pprint ...)
   xtask: gen-fidelity / jj2rs / intern-dump-diff (いずれも --check 付き)
```

crate の境界は、TLC 自身が既にインターフェースとして使っている継ぎ目に置く。evaluator と checker の間は `ITool`、SANY と TLC の間は `ModuleNode` の読み取り API である。

### 要になる判断

1. **意味グラフは 1 つ、パーサーも 1 つ (base: 候補 1)。**
   - SANY の意味グラフを型付き arena に置き、SANY が終わった時点で凍結する。
   - パーサーは `xtask jj2rs` が `TLAplusParser.java` (10,790 行) を機械的に転写して作る。
   - 統合版では TokenManager (5,334 行) も転写対象に加えた。judge が「全案とも lexer を手書きしていた」と指摘した穴を塞ぐためである。
   - これで構文エラーと字句エラーの文言、`bpa` のスタック、`matchFcnConst` のバグ、`\X` の n 項化が、作り方の時点で TLC と一致する。
   - semantic action も、それが呼ぶ SANY のクラス (SyntaxTreeNode、Operator(s)、OperatorStack、ParseError(s) など) も同じ変換器で転写する。手で書くのは Java ライブラリの部分集合と、変換しないクラスだけである (下の「パーサーの転写」)。
2. **順序は事後に再生せず、TLC と同じ動作から自然に出す (base)。**
   - intern は TLC と同じ地点で行う。クラス初期化は明示的な `JavaClass` イベントとして扱う。
   - SANY の `Context` は `tlars_java::Hashtable` そのものにする。そのため宣言順を所有するのは 1 か所だけになる。
3. **`Value` は TLC の値クラスと 1 対 1 で写す (base)。**
   - 21 の variant を持ち、16 byte で、`Send + Sync`。
   - `PartialEq`/`Ord`/`Hash` は実装しない。比較は fallible な `tlc_equals`/`tlc_compare` を受け手側の型から先に dispatch する。
   - 遅延値 (thunk) は値の中に入れられない。
   - TLC がその場で行う正規化は、書き込み 1 回限りの `OnceLock` で表す。
   - 統合版では、候補 2 の `Canon` を graft した。eq/cmp/fp は `Canon` に対してだけ書き、表現ごとの fast path は debug build で `fast_checked` が `Canon` の結果と照合する。
4. **evaluator は Tool.java を写す (base)。**
   - Init/Next は stack 上で借用する cons list の継続渡しで生成し、部分状態は bind/unbind で組み立てる。
   - lookup は Ctx の連鎖 → side table の順の動的探索で、TLC と同じである。judge が一次情報で確認したとおり、引数付き LET 演算子と INSTANCE 置換は呼び出し側の context で解決される。候補 2 のようにレキシカルに解決すると TLC とずれる。
   - 名前渡しの thunk のキャッシュ条件は `LazyValue.java:164-204` を写す。
5. **effect の窓口は 1 つにする (graft: 候補 3)。**
   - RNG、TLCSet/TLCGet、`TLCGet("distinct")` などのカウンタ読み取り、実行時の intern、Print はすべて `tlars-tool::effects::Effects` を通す。
   - `StateRandom` は親状態ごとに `fp ^ seed` で 1 回だけ再シードする。trace 再生も同じ経路を通るので、RNG のスナップショットや復元は要らない。
   - これで A と B の設計にあった parity バグが直る。同時に、将来の順序どおりの commit で、投機的な effect を記録する場所がここ 1 か所に決まる。
6. **エンジンは ModelChecker/Worker を写す (base) が、`SeenOracle` を引数にする (graft: 候補 3)。**
   - `addElement` の方針 (計数、検査の順番、enqueue の規則) は `examine` の 1 か所にしか書かない。
   - v1 の oracle は `SharedPut` で、TLC と同じく共有 FPSet に put する。
   - M6 で候補 3 の reorder buffer に差し替える。内容は、`StateIx`/`Rank`、shard ごとに書き手が 1 つの FPSet、committer による `SetOfStates` の構築、multiplier の prefix-scan である。差し替えても `examine` は変わらない。
7. **liveness は式の側と graph の側で分ける (graft: 候補 2)。**
   - `Liveness.processLiveness` (DNF、PEM、tableau) は `tlars-tool::live::LiveSpec` が持つ。
   - checker が見るのは check の index とビット列だけにし、`Ctx` や式を見ないようにした。sketch でも `pub use Ctx` を削除してある。
   - `SetOfStates` は、等値で重複判定し、比較中の例外は握りつぶし、親を最後に stutter として加える。いずれも一次情報どおりである。
8. **そのほかの graft (候補 2 から)。**
   - FPSet の鍵は `FpKey` だけにした。`From<Fp>` でしか作れないので、63 bit マスクの適用漏れが型で防げる。
   - `tlars-java` の所属規則を 1 文で書いた。「JDK/TLC の utility を bit 単位で再現し、TLA+ の値に触れないものだけを置く」。pprint もこの規則に従ってここへ移した。
9. **出力はすべて `tlars-msg` の 1 つの `catalog!` 表から作る。** code、名前、テンプレート、終了コードをここから導出する。class は呼び出し側が渡し、表には持たせない。stdout に書くのは `Report` だけである。
10. **Java の static は 1 回の実行を表す `Session` に集める。** 状態を共有しないので、61k 件をプロセス内で並列に実行できる。同じ spec を同じプロセスで 2 回実行して差分を取るテストで、状態の漏れを検出する。
11. **出力に効く crate では std の hash コンテナを禁止する。** clippy の `disallowed-types` で `HashMap`/`HashSet` を禁じ、CI では `-D` で落とす。

### 不変条件の所有者表

| 不変条件 | 唯一の所有者 |
|---|---|
| intern 順とクラスの static 文字列 | `sany::intern` と生成表 `sany/fidelity/class_statics.rs` (`xtask gen-fidelity --check`) |
| Hashtable による var/const/`I!Op` の順序 | `tlars_java::Hashtable`。これを `sany::semantic::Context` としてそのまま使う |
| FP64 の多項式表 | `tool::fp64` (`POLYS` は生成)。種別ごとの符号化は `value/fingerprint.rs` |
| 63 bit マスク | `FpKey::from` |
| 全順序、サイズ優先の集合比較、model value の規則、エラー文 | `value/order.rs` (`Canon` と `fast_checked`) |
| ネイティブ列挙順と正規化後の列挙順 | `value/enumerate.rs` |
| `toString` | `value/print.rs`。pprint は `tlars_java::pretty_print` |
| action の分割順とラベル、「最初に一致した action」の再生 | `tool::actions`、`Ev::successor_from_fp` |
| RNG の再シード、registers、カウンタ読み取り | `tool::effects` |
| `addElement` の方針 | `check::bfs::worker::examine` (`SeenOracle` で汎用化) |
| `SetOfStates` のスロット順 | `check::liveness::set_of_states` |
| 時相式の DNF、PEM、tableau | `tool::live` |
| MP/EC と終了コード | `tlars-msg::catalog!` |
| JavaCC の構文・字句エラー文 | `xtask jj2rs` の生成物 |
| `Double.toString` (JDK 別)、`DecimalFormat`、`SimpleDateFormat`、`Random` | `tlars-java` |

### あえてやらないこと

- **値の hash-consing はしない。** 印字の異なる表現が 1 つに統合されてしまう。
- **静的な名前解決や INSTANCE のインライン展開はしない。** thunk の同一性、キャッシュ、RandomElement の値、エラーの出るタイミング、`I!Next` の分割が TLC と変わる。
- **評価器を 2 つ持たない。** typed tier や closure compiler を別途足すことはしない。速度は M3.5 で計測してから、同じ evaluator の内部で稼ぐ。
- **ユーザーの `.class` override は v1 では扱わない。** 警告を出して TLA+ の定義を使う。差し込み口として `OverrideProvider` を private に用意しておく。
- **スコープ外:** TLAPS、PlusCal の翻訳、tla2tex、Toolbox、debugger、分散 TLC。

## Synthesis decision

- **base: 候補 1 (Faithful mirror)。**
  - cross-judge (opus xhigh) の採点は A 24 / B 20 / C 24 で、A を base に推奨した。私の採点でも、忠実性、値と evaluator、作りやすさの 3 項目で A が最も強かった。
  - コーパスは全件 `-workers 1` で記録されているので、TLC のアルゴリズムを TLC のデータ形のまま動かす A が parity に最も近い。
  - 決め手は 2 つある。動的な lookup を残していること (B はここが TLC とずれる)、そしてパーサーの転写でエラー文が作り方の時点で一致することである。
- **候補 3 から graft したもの:**
  - `Effects` の窓口と、親状態ごとの RNG 再シード。A と B にあった parity バグの修正にあたる。
  - `SeenOracle` による `addElement` の汎用化。M6 で reorder buffer を入れる前提になる。
  - 決定性 sweep (1 worker と 2・8 worker、`ScheduleFuzz`)。
  - `SetOfStates` の正確な仕様。
- **候補 2 から graft したもの:**
  - `Canon` と `fast_checked`。
  - liveness の式の側を tool に寄せ、checker から `Ctx` を外したこと。
  - `FpKey`。
  - `tlars-java` の 1 文規則と pprint の移設。
- **捨てたもの:**
  - 候補 2 の lowering とレキシカル frame。引数付き LET で TLC とずれる。
  - 候補 2 のレベル同期 BFS と taint 時の全面やり直し。B 自身が捨てた方式を結局採用していた。
  - 候補 2 の「code ごとに class を 1 つ」の表。class は呼び出し側が決める。
  - 候補 3 の `-lncheck default` を作業量ベースで置き換える案。製品として TLC とずれる。
  - 候補 3 で並列化を M3 に置いていた順序。M6 に後ろ倒しした。
  - 候補 3 の M8 closure IR。評価器が 2 つになる。
- **3 案の一致点:** 値の表現は保持し、hash-consing はしない。パーサーは 1 本。不変条件ごとに所有者を 1 つ置く。MP の表は 1 つ。tla-ts のコーパスをプロセス内で実行する。
- **検証:**
  - 統合版スケッチで `cargo check` (警告 0) と clippy (`disallowed_types` 違反 0) を通した。
  - RNG 再シード、trace 再生時の再シード、LET のスコープ、`SetOfStates` の catch を TLC のソースで再確認した。

## Tradeoffs accepted

- 約 10k 行 (TokenManager を含めて約 15k 行) の生成コードを受け入れる。引き換えに、構文と字句のエラー文が作り方の時点で一致する。
- `Arc` の cons list と動的 lookup のコストを受け入れる。引き換えに、遅延評価とエラーの出るタイミングが TLC と同じになる。速度は M3.5 で計測し、許容できなければ同じ evaluator 内で arena 化する。
- 約 21 の値 variant と受け手側の型からの dispatch を受け入れる。引き換えに、印字、列挙順、エラー文が正確になる。
- v1 では `-workers N` で違反が出たとき、TLC と同程度に非決定的になることを受け入れる。成功時の数は一致する。M6 で `-workers 1` と完全に一致させる。
- FPSet、queue、trace、liveness graph は M6 まで in-memory で持つ。disk 版は同じ型の背後に後から差し込む。
- 版の表示 (2262) と起動時の表示 (2187) のうち JVM を表す欄は、TLC の値をまねず tlars 自身の値を出す。2262 は `Version <tlars の版> of tlars (rev: bc1b506)`、2187 の JVM のベンダー、版、アーキテクチャは `tlars`、tlars の版、`64bit`、ヒープとヒープ外のメモリは `0` である。どの実装の出力かを利用者が取り違えないためで、どちらのメッセージも照合の正規化で落ちる。FPSet とキューの名前は、TLC と同じシステムプロパティ (`tlc2.tool.fp.FPSet.impl`、`tlc2.tool.queue.IStateQueue`) とその既定値に従う。

## Alternatives considered

- **core IR に lowering し、Java の挙動を kernel に閉じ込める案 (候補 2 の形)。** 下流が構文を見なくて済む点は深い設計だった。ただ TLC の動的スコープはレキシカルな frame では表せず、parity が崩れる。順序も「Java 世界の表を凍結して再生する」方式になり、TLC の動作から自然に出る形にならない。
- **決定性を優先したイベント契約の案 (候補 3 の形)。** 並列でも出力が完全に一致し、組み込み用途にも良い。しかし土台にすると M3 で投機実行の機構を抱え込み、parity 前に複雑さが前倒しになる。`SeenOracle` と `Effects` の部分だけを取り込んで、M6 で導入できる形にした。
- **tla-ts 型の 2 パーサー + 型付き tier + 並列の後に逐次でやり直す案。** 同じことを 2 通りで実装することになり、手で同期するリストも生まれる。

## 決定事項 (2026-11-17 チェックポイント)

1. `-workers N`: v1 は TLC 同様に非決定 (成功時の数は一致)、M6 で in-order commit により `-workers 1` と完全一致。
2. TLC のバグは既定で再現 (`Fidelity::Tlc`)、tla-ts の修正は切替 (`Fidelity::Corrected`)。
3. 固定版は tlaplus `bc1b506`。tla-ts コーパスの記録 (jar 2026.10.24) と差がある箇所は、bc1b506 の TLC で再録して期待値を更新する。
4. intern 順は計測用 TLC の InternTable dump で一度照合する (`xtask intern-dump-diff`)。
5. `-fp`/`-seed` の既定は TLC 同様に乱数。

### パーサーの転写 (`xtask jj2rs`)

- 入力は固定版 TLC のソースツリーにある Java ファイル。JavaCC の生成物 (`TLAplusParser`、`TLAplusParserTokenManager`、`SimpleCharStream`、`Token`、`ParseException`、`TokenMgrError`、2 つの定数 interface) はソースツリーに置かれており、JavaCC を回し直した出力と同一である。これに SANY のクラス (`SyntaxTreeNode`、`Operator`、`Operators`、`OSelement`、`OperatorStack`、`ParseError`、`ParseErrors`、`ParseExceptionExtended`、`Strings`) を加え、`crates/tlars-sany/src/parser/generated.rs` (約 2.3 万行) を出力する。
- Java の意味は `parser/java.rs` が受け持つ。
  - 参照は `J<T>` (null を取りうる `Rc<RefCell<T>>`、`==` は参照の同一性)、文字列は `JStr` (UTF-16 単位で添字を数える) で表す。
  - 例外は `JResult` で伝える。`Error` と実行時例外は panic にする。TLC でもパースが中断するからである。
  - `Object` は downcast できる箱にする。
  - フィールドは `jread` で読み、借用はその読み取りの間で終わらせる。こうしないと、呼び出し先が同じオブジェクトを書き換えたときに `RefCell` が panic する。
- オーバーロードは引数の型で解決する。static フィールドは遅延初期化して、Java のクラス初期化順 (つまり intern 順) を再現する。
- 手書きするものは `parser/support.rs` にまとめる。
  - Java ライブラリの部分集合: `Vector`、`Hashtable` (列挙順は `tlars_java::Hashtable` で再現)、`Deque`、`StringBuffer`、`Reader` など。
  - `UniqueString` (セッションの `Interner` に委ねる)、`Location`、`SanyOutput`、`JunctionListContext`。最後のものは enum と内部クラスを使うので手で移植する。
  - `SimpleCharStream.ExpandBuff`: `catch (Throwable)` を含むため。
  - `TLAplusParser(SanyOutput, byte[])`: UTF-8 で decode してから `Reader` 版のコンストラクタに渡す。
- 意味解析 (`tla2sany/semantic` の全クラス) も同じ変換器で転写する。そのために変換器に足した規則は次のとおり。
  - 継承の階層は、根のクラスごとに `enum RootObj` を 1 つ作り、具体クラス 1 つにつき variant を 1 つ置く。各 variant は、根からそのクラスまでのフィールドをすべて持つ。参照は `J<RootObj>` で共通にするので、型を変えるキャストは何もしない。仮想呼び出しと `instanceof` は、variant に対する `match` で表す。
  - オーバーライドの判定は「同じシグネチャを最初に宣言したクラスまたは interface」(origin) を単位にする。両方の origin を実装するクラスがあれば、2 つは同じ分岐関数にまとめる。シグネチャが同じでも別系統なら別の関数にする (`DecimalNode.bigVal()` と `NumeralNode.bigVal()` がその例)。
  - 内部クラスは、外側のインスタンスを `this__0` に持つ。名前の解決では外側を順にたどる。
  - 例外は生成される `JThrow` に入れる。これは翻訳したクラスの例外にも、`NumberFormatException` のようなライブラリの例外にも使う。`catch` は「その型の下位クラスか」を判定して捕まえる。
  - `HashMap`/`HashSet` は JDK 21 と同じ列挙順を再現する (`tlars_java::JavaHashMap`、JDK の golden で照合済み)。`SemanticNode.hashCode` は `kind` と連番の `myUID` から決まるので、TLC の列挙順も再現できる。キーには `JValue` (`hashCode`/`equals`/`toString`) を生成する。オーバーライドがなく参照の同一性で比べるクラスは、Java でも列挙順が実行ごとに変わるので一致させられない。
  - `Integer` は null を取りうる `JInteger` で表す (SANY がレベル表の `null` を検査するため)。`Boolean`、`Long`、`Character` はプリミティブのまま扱う。
  - stream とラムダ、raw 型の使い回し、モデル化しない interface を引数に取るメソッドは、変換の前に Java ソースへパッチを当てる (`jj2rs/mod.rs` の `PATCHES`)。どのパッチも、ファイル中にちょうど 1 回だけ現れる完全一致の置換で、理由を添えてある。
  - 無名クラスを使う `walkChildren`、`isDefinedWith`、`substituteFor` は、リンターと TLC が使うだけなので、今は `todo!()` にしてある。
  - 多次元配列の生成 (`new boolean[a][b][]`) は、長さを与えた次元ごとに別の配列を作り、残りの次元は null にする。
- 仕様の読み込み (`tla2sany/modanalyzer` の `SpecObj`、`ParseUnit`、`ModulePointer` など) も転写する。ファイルの扱いは shim (`parser/files.rs`) にまとめ、`PATCHES` で Java 側をそれに合わせる。範囲を置き換えるパッチは `開始⋯終了` と書く。
  - 解決順は `SimpleFilenameToStream` と同じで、ユーザーディレクトリ、ライブラリパス、標準モジュールの順に探す。標準モジュールは `xtask gen-fidelity` が固定版ソースから `crates/tlars-sany/StandardModules/` に写し、バイナリに埋め込む。TLC と同じく、読む前に `tlc-*` の一時ディレクトリへ書き出す。
  - TLC は jar から読んだ標準モジュールについて `Parsing file <path> (jar:file:...)` と jar の URL を添える。tlars には jar がないので、パスだけを出す (TLC の標準モジュールがファイルシステム上にあるときと同じ形)。
  - 1 ファイルに複数のモジュールを書いた仕様 (`MonolithSpecExtractor`) は Rust で書き直した。
- PlusCal の翻訳検証 (`SpecObj.validateParseUnit`) のために、PlusCal のパーサーと翻訳器 (`pcal` パッケージ) と、それが使う `tla2tex` の `Symbol`、`Misc`、`Debug` も転写する。`pcal.Validator` は内部 enum、正規表現、CRC32 を使う短いつなぎなので `parser/pcal_validator.rs` に手で書く。検査和の正規表現は固定の形なので、Java の `\s` と ASCII の `(?i)` に合わせた一致判定を手で書いた。そのために変換器に足した規則は次のとおり。
  - ファイル単位の識別子の言い換え (`jj2rs/mod.rs` の `RENAMES`)。PlusCal の `Vector` は `java.util.Vector` で、`contains` と `indexOf` が `equals` で比べる。SANY の `tla2sany.utilities.Vector` (`==` で比べる) と区別するため `JavaVector` と読み替える。`Tokenize` の `Token` は `tla2tex.Token` なので `TexToken` と読み替える。
  - 型消去する外部クラス (`ERASED_LIBRARY_GENERICS`)。PlusCal は raw な `Vector` と `Vector<T>` を自由に混ぜるので、`JavaVector` は Java と同じく要素を `Object` で持つ 1 つの型にする。要素を返すメソッドは、呼び出し側が期待する型を turbofish で受け取ってキャストする。
  - 外部クラスと同名の内部クラス (`AST.Assert` と `util.Assert`) は `AST_Assert` と改名し、修飾名から引く。
  - `Object` は保持する値のクラス名を持ち、`getClass()` を動的に返す。PlusCal は AST の種類を `getClass().equals(...)` で判定するためである。
  - ファミリーの根が `toString` を持たず、下位クラスだけが持つ場合 (`AST.If` など) は、`toString` を下位クラスごとに振り分ける。
  - 複数例外の `catch (A | B e)`、`catch (final E e)`、多次元配列、`split` のリテラルの選択肢だけの正規表現。
  - TLC と一致させられない点: `Character.isLetter`/`isDigit` は、ASCII の外では Rust の `Alphabetic`/`Numeric` で代用している (Java は一般カテゴリ Lu/Ll/Lt/Lm/Lo と Nd)。
  - PlusCal 翻訳器をプログラムとして使う部分 (`trans.main`、ファイルの読み書き、`-help`) は、検証からは呼ばれないので `todo!()` のままにしてある。
- リンター (`tla2sany.linter.Linter`) も転写する。ストリームはループに、無名の `ChildrenVisitor` は名前付きのクラスにするパッチを当てる (同じ順序、同じ短絡評価)。
- SANY のドライバ (`SANY.parse`、`frontEndParse`、`frontEndSemanticAnalysis`、`frontEndLinting`) は、転写したクラスをつなぐだけなので `parser/driver.rs` に手で書く。意味解析中の `AbortException` について TLC が出す JVM のスタックトレースは、先頭行 (例外の文字列) だけを出す。引数のないメッセージは、TLC の `SimpleSanyOutput` と同じく `String.format` を通さずにそのまま出す。
- 公開 API `tlars_sany::load` は `SpecProcessor.processSpec` の呼び方 (`SanySettings.forExternalCaller`: 厳格な終了コード、全段階あり) で SANY を動かし、成功すれば `graph::SemanticGraph` を返す。
  - 大きいスタックを持つ専用スレッドで動かす。翻訳したコードの static はスレッドごとなので、1 回の読み込みが新しい JVM と同じ状態から始まる。SANY の出力と `ToolIO` の出力は、TLC と同じ順でチャネル経由で `SanyOut` へ流す。
  - ルートの名前とユーザーディレクトリは TLC と同じく呼び出し側が決める (TLC は絶対パスのときだけユーザーディレクトリを仕様のディレクトリにする)。
  - 失敗は `SanyFailed` で返す。`FrontEnd` は `TLC_PARSING_FAILED2`、`Parse`/`Semantic` は記録されたエラー文字列付きの `TLC_PARSING_FAILED`、`Exit` はエラーに昇格した警告の場合にあたる。
  - 翻訳したオブジェクトのグラフは `parser/freeze.rs` が識別子ごとに 1 つの id を振って arena に写す。`@` (`AtNode`) は自分を含む EXCEPT を指すので、式の id は子をたどる前に予約する。
  - M0 の設計から次を変えた。組み込み演算子の `OpDefKind::BuiltIn` は値を持たない (演算子は名前で分かり、TLC の演算子コード `BuiltinOp` への対応は評価器が受け持つ)。定理と証明は TLC が評価しないのでグラフに含めない。`Loc` のモジュール名は null を取りうる。
  - 未実装: `ParameterizedSpecObj` (`-inv` などが根の EXTENDS に加えるモジュール) と `resolve_root_name` (M3)。
- 外部 API のシグネチャは `xtask` の `jj2rs/externs.rs` に表として書く。表にない呼び出しは出力時のエラーになり、推測で補うことはしない。`jj2rs emit --check` で生成物が最新かを確かめる。

## Risks

- jj2rs の snippet 表 (約 300) が扱える量か。M1 冒頭で spike。
  - spike 結果 (2026-11-17): JavaCC 4.0 で生成した 6 ファイル (17,562 行) の全 618 メソッドを Java 部分集合パーサーで AST 化できた (`xtask jj2rs survey`)。手書きは `.jj` 冒頭のヘルパー 19 メソッド (847 行) と semantic action が呼ぶ約 45 の API (SyntaxTreeNode、OperatorStack、JunctionListContext、proof level 等) に限られる。snippet 表方式はやめ、semantic action も同じ変換器で Rust に落とす方針に変更。
  - 実装結果 (2026-11-18): 転写したパーサーは tlaplus/Examples の全 433 モジュールで TLC と一致した (構文木、パースの成否、エラーログ、パース中の intern 列)。比較対象は `tools/golden/ParseGolden.java` の出力で、一致した文字列は構文エラーと字句エラーの文言も含む。
  - 実装結果 (2026-11-18): 転写した SANY の前段 (読み込み、意味解析、レベル検査) は、tlaplus/Examples の全 433 モジュールをそれぞれ根の仕様として動かしたとき TLC と一致した (終了コード、エラーと警告の文言を含むログ、ToolIO の出力、intern 列、`ExternalModuleTable` の dump、全演算子定義のレベル)。433 件のうち 151 件は TLC でもエラーで終わる仕様である。比較対象は `tools/golden/SemanticGolden.java` の出力で、仕様ごとに JVM を起動する。リンターと PlusCal の翻訳検証も有効にして一致した (PlusCal を含む仕様では翻訳器が実際に動き、その出力も一致する)。`tlars_sany::load` が返すグラフの全モジュールの演算子定義と名前、レベル、コンソール出力も、同じ golden と一致する。
- 転写したコードは Java のオブジェクトモデル (`Rc<RefCell<_>>` と `JResult`) のまま動くので遅い可能性がある。パースは全 433 モジュールで約 2 秒 (release、TLC は JVM 起動込みで約 2 秒) なので、今は問題にしていない。`SyntaxTreeNode.parent` は `Rc` の循環参照になり、`setParent` を呼ぶと木がリークする。Generator に渡すときに読み替えるか、`Weak` にする。
- `Arc` トラフィックが JIT 済み TLC に対し許容できるか。M3.5 で計測。
- stack overflow の深さは `-Xss2m` の frame サイズ依存で一致させられない見込み。

## Phasing

| マイルストーン | 内容 | 合格条件 |
|---|---|---|
| M0 | `tlars-java` (`String.hashCode`、Hashtable、HashSet 順、Random、Double.toString、BigDecimal、DecimalFormat、SimpleDateFormat、数値パース)、FP64、`tlars-msg` (全 251 コード)、CLI、`Env` (cfg と pprint はそれぞれ M1 後と M2 へ移動) | JDK 21 と bc1b506 の TLC から生成した golden (完了) |
| M1 | jj2rs (parser + TokenManager)、Generator、level、SANY の出力文 | syntax/modules/errors 約 1.8k 件。Hashtable 順と intern 順をここで検証する。パーサー、読み込み、意味解析、レベル検査、リンター、PlusCal の翻訳検証、`load` は完了 (Examples 全 433 モジュールで TLC と一致) |
| M2 | 全値クラス (order/fp/print/enumerate)、定数評価 | expressions 984 件 × 5 context |
| M3 | 1 worker の BFS、SpecProcessor、action 分割、trace 再生、ALIAS/VIEW/SYMMETRY | checker と standard |
| M3.5 | **性能ゲート** (新設、judge の指摘による) | 大きめの spec (Paxos 等) で TLC との速度比を計測。許容できなければ表現を見直す |
| M4 | liveness (`LiveSpec`、graph、`SetOfStates`、SCC、lasso) | temporal 873 件 (`-lncheck final`) |
| M5 | CLI parity (simulate/dfid/continue/difftrace/dump/dumpTrace/TE spec/coverage) | cli 194 件と生成された 54k 件 |
| M6 | in-order commit の並列化 (`SeenOracle` を差し替え)、決定性 sweep、disk FPSet/queue/trace、checkpoint | 全コーパスを 1/2/8 worker で一致させる |

## 開発環境

- `nix develop`: Rust、JDK 21、samply/perf、bc1b506 から組み立てた `tlc`/`tla2tools-java`、`TLAPLUS_SRC`。
- `cargo run -p xtask -- gen-fidelity [--check]`: FP64.Polys、EC/MP カタログ、SANY ErrorCode、埋め込む標準モジュールを固定版ソースから生成。
- golden の再生成手順は `tools/golden/*` の先頭コメント。
- ベンチマーク: `cargo run --release -p xtask -- bench run --engine tlc|tlars [--workers N] [--repeat N] [--filter TEXT]`。対象は `bench/suite.txt` (tlaplus/Examples の cfg パス)、期待値は Examples の manifest から読み、結果は `bench/results/*.jsonl` (git 管理外)。`bench compare <base> <cand>` で比較。基準値は `bench/baselines/` に置く。
- プロファイル: `cargo build --profile profiling -p tlars` の後に `samply record` か `perf record`。

## Next implementation step

`tlars-java` の `string_hash_code`、`Hashtable`、`JavaRandom`、`Fp64` を golden test 付きで実装する。続けて cfg の字句解析と構文解析を通し、「cfg 系 423 件で TLC と同じ 151 系エラーがバイト単位で一致する」を最初の到達点にする。
