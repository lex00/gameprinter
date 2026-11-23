//! `gp-kit SRC OUT`: the module kit made from the tool's source in SRC, in
//! OUT, as `gp game kit --out OUT` makes it from a source build. The release
//! build (scripts/release/build.sh) runs it so a fresh runner needn't build
//! a whole gp first. Exit codes: 0 made, 2 couldn't make it.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [src, out] = args.as_slice() else {
        eprintln!("usage: gp-kit SRC OUT");
        return ExitCode::from(2);
    };
    match gp_kit::make(src, out) {
        Ok(m) => {
            println!(
                "module kit in {} from {}: {} crates, hash {}",
                m.kit.display(),
                src.display(),
                m.crates,
                m.hash
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gp-kit: {e:#}");
            ExitCode::from(2)
        }
    }
}
