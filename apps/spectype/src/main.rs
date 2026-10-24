use std::process::ExitCode;

#[cfg(not(target_family = "wasm"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    let mut args = std::env::args();
    let argv0 = args.next().unwrap_or_default();
    let name = std::path::Path::new(&argv0).file_name().map(|n| n.to_string_lossy().into_owned());
    let program = name.as_deref().map_or("spectype", |n| n.strip_suffix(".exe").unwrap_or(n));
    if spectype::run_cli_then_exit(program, args.collect()) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
