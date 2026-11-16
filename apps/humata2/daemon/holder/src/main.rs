//! `humata2_holder`: holds one terminal for humatad2. Prints `ready` on
//! stdout once it's listening; what goes wrong goes to stderr.

use std::error::Error;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use humata2_holder::{ServeOptions, saved_scrollback_path, serve};

#[derive(Parser)]
#[command(about = "Holds one terminal's program and scrollback for humatad2.")]
struct Args {
    /// The Unix socket to serve the terminal on.
    #[arg(long)]
    socket: PathBuf,
    /// The folder the program starts in.
    #[arg(long)]
    cwd: PathBuf,
    #[arg(long, default_value_t = 80)]
    cols: u16,
    #[arg(long, default_value_t = 24)]
    rows: u16,
    /// How much output to keep for clients that attach later.
    #[arg(long, default_value_t = 8 << 20)]
    scrollback_bytes: usize,
    /// Where output is saved for a holder started on the same socket
    /// after this one is killed. Beside the socket unless set.
    #[arg(long)]
    scrollback_file: Option<PathBuf>,
    /// The program and its arguments.
    #[arg(last = true, required = true)]
    command: Vec<OsString>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    // Leaves the starter's session, so a hang-up there doesn't reach the
    // holder. Already a session leader is fine.
    let _ = rustix::process::setsid();

    let mut command = args.command.into_iter();
    let Some(program) = command.next() else {
        eprintln!("humata2_holder: name the program to run after `--`");
        return ExitCode::FAILURE;
    };
    let saved = args
        .scrollback_file
        .unwrap_or_else(|| saved_scrollback_path(&args.socket));
    // The host may own bounded compressed checkpoints instead of a raw transcript.
    // Its holder still keeps the normal in-memory replay window.
    let memory_only=std::env::var_os("HUMATA2_HOLDER_MEMORY_ONLY").is_some_and(|value|value=="1");
    let options = ServeOptions {
        saved: (!memory_only).then_some(saved),
        socket: args.socket,
        program: PathBuf::from(program),
        args: command.collect(),
        cwd: args.cwd,
        cols: args.cols,
        rows: args.rows,
        scrollback_bytes: args.scrollback_bytes,
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("humata2_holder: couldn't start its runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let served = runtime.block_on(serve(options, || {
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "ready");
        let _ = stdout.flush();
    }));
    match served {
        // Leaves without waiting for a client's typing stuck on a program
        // that no longer reads.
        Ok(()) => std::process::exit(0),
        Err(error) => {
            let mut message = format!("humata2_holder: {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                message.push_str(&format!(": {cause}"));
                source = cause.source();
            }
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
