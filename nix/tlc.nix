# TLC built from the pinned tlaplus source: the oracle tlars is compared against
# (golden files, corpus re-recording, benchmarks, instrumented intern dumps).
# Offline: every dependency is a jar vendored in the tlaplus repository.
{ stdenv, jdk21, makeWrapper, tlaplus }:

stdenv.mkDerivation {
  pname = "tla2tools";
  version = "bc1b506";
  src = tlaplus;
  nativeBuildInputs = [ jdk21 makeWrapper ];

  buildPhase = ''
    runHook preBuild
    cd tlatools/org.lamport.tlatools
    # customBuild.xml "generate" (regenerates the checked-in JavaCC parser; output is identical).
    java -Dfile.encoding=UTF-8 -cp lib/javacc-4.0.jar javacc -OUTPUT_DIRECTORY=src/tla2sany/parser javacc/tla+.jj
    libs=$(find lib -name '*.jar' | tr '\n' ':')
    find src -name '*.java' > sources.txt
    mkdir class
    # customBuild.xml "compile": release 11, UTF-8.
    javac -nowarn -encoding UTF-8 --release 11 -cp "$libs" -d class @sources.txt
    (cd src && find . -type f ! -name '*.java' -exec install -D {} ../class/{} \;)
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/share/tla2tools $out/bin
    jar cf $out/share/tla2tools/tla2tools.jar -C class .
    cp -r lib $out/share/tla2tools/lib
    cp=$out/share/tla2tools/tla2tools.jar:$(find $out/share/tla2tools/lib -name '*.jar' | tr '\n' ':')
    makeWrapper ${jdk21}/bin/java $out/bin/tlc --add-flags "-XX:+UseParallelGC -cp $cp tlc2.TLC"
    makeWrapper ${jdk21}/bin/java $out/bin/tla2tools-java --add-flags "-cp $cp"
    runHook postInstall
  '';
}
