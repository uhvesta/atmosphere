use std::process::ExitCode;

fn main() -> ExitCode {
    let code = lore_cli::run_from_env();
    ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1))
}
