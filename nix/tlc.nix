# TLC built from the pinned tlaplus source: the oracle tlars is compared against
# (golden files, corpus re-recording, benchmarks, instrumented intern dumps).
# Offline: every dependency is a jar vendored in the tlaplus repository.
{ stdenv, jdk21, makeWrapper, unzip, tlaplus }:

stdenv.mkDerivation {
  pname = "tla2tools";
  version = "bc1b506";
  src = tlaplus;
  nativeBuildInputs = [ jdk21 makeWrapper unzip ];

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
    # customBuild.xml "dist": the libraries the released tla2tools.jar bundles, and no
    # others (a jar with modules at its root, such as CommunityModules.jar, would change
    # how TLC resolves module names).
    unzip -qo lib/javax.mail/mailapi-1.6.8.jar '*.class' META-INF/LICENSE.md META-INF/mailcap -x 'javax/mail/search/*' -d class
    unzip -qo lib/javax.mail/smtp-1.6.8.jar '*.class' -d class
    unzip -qo lib/javax.mail/javax.activation_1.1.0.v201211130549.jar '*.class' -x 'org/*' -d class
    touch class/META-INF/javamail.default.address.map
    for j in lib/jline/jline-terminal-3.25.0.jar lib/jline/jline-console-3.25.0.jar \
             lib/jline/jline-builtins-3.25.0.jar lib/jline/jline-reader-3.25.0.jar lib/gson/gson-2.14.0.jar; do
      unzip -qo "$j" -x META-INF/MANIFEST.MF -d class
    done
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/share/tla2tools $out/bin
    jar cf $out/share/tla2tools/tla2tools.jar -C class .
    cp=$out/share/tla2tools/tla2tools.jar
    makeWrapper ${jdk21}/bin/java $out/bin/tlc --add-flags "-XX:+UseParallelGC -cp $cp tlc2.TLC"
    makeWrapper ${jdk21}/bin/java $out/bin/tla2tools-java --add-flags "-cp $cp"
    runHook postInstall
  '';
}
