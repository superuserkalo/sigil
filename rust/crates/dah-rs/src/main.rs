//! `dah`: the standalone CLI binary. Sigil runs the same entry as `Sigil cli ...`.

fn main() {
    std::process::exit(dah_rs::run_from(std::env::args_os()));
}
