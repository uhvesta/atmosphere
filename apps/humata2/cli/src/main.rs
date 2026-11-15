//! `humata`: see the library for what each command does.

fn main() {
    let code = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(humata2_cli::main(std::env::args_os().collect())),
        Err(error) => {
            eprintln!("humata: couldn't start: {error}");
            eprintln!("The system may be out of threads or file descriptors; close some programs and run it again.");
            humata2_cli::exit::INTERNAL
        }
    };
    std::process::exit(code);
}
