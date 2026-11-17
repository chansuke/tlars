{
  description = "tlars: TLC (the TLA+ model checker) reimplemented in pure Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # The TLC version tlars reproduces (docs/DESIGN.md). Read by `xtask gen-fidelity`.
    tlaplus = {
      url = "github:tlaplus/tlaplus/bc1b506bdcad80bb518bf3776a9161c34ff0fc4c";
      flake = false;
    };
    # Benchmark and smoke-test models with recorded state counts (MIT). Read by `xtask bench`.
    tlaplus-examples = {
      url = "github:tlaplus/Examples/334c3983a2f44ac78fce89d1d20aa79ceb3e6987";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, flake-utils, tlaplus, tlaplus-examples }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        tlc = pkgs.callPackage ./nix/tlc.nix { inherit tlaplus; };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
            # Oracle JDK for golden files and the instrumented TLC (docs/DESIGN.md pins JDK 21).
            pkgs.jdk21
            # `tlc` and `tla2tools-java` (java with tla2tools.jar on the classpath), built from bc1b506.
            tlc
            # Profiling: `cargo build --profile profiling`, then `samply record` or `perf record`.
            pkgs.samply
            # GNU time: `xtask bench` records peak RSS with it.
            pkgs.time
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.perf ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
          TLAPLUS_SRC = "${tlaplus}";
          TLAPLUS_EXAMPLES = "${tlaplus-examples}";
        };

        packages.tlc = tlc;

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "tlars";
          version = "0.0.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "tlars" ];
          doCheck = false;
        };
      });
}
