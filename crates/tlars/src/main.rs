//! `tlars [-D<prop>=<v> ...] [TLC options] Spec.tla`: `java tlc2.TLC`, in Rust.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let status = tlars_check::run(tlars_check::Invocation::from_process(args));
    std::process::exit(status.code());
}
