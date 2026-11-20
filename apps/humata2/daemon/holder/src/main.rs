//! `humata2_holder`: holds one terminal for humatad2. Prints `ready` on
//! stdout once it's listening; what goes wrong goes to stderr.

use std::error::Error;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use humata2_holder::{
    ServeOptions, check_take_over, handoff_versions, saved_scrollback_path, serve, take_over,
};

#[derive(Parser)]
#[command(about = "Holds one terminal's program and scrollback for humatad2.")]
struct Args {
    /// The Unix socket to serve the terminal on.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// The folder the program starts in.
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Prints the handoff versions this build reads, and exits.
    #[arg(long)]
    handoff_versions: bool,
    /// Reads the handoff at this path as a take-over would, and exits.
    #[arg(long)]
    check_take_over: Option<PathBuf>,
    /// Runs as the holder this handoff describes (used by a move).
    #[arg(long)]
    take_over: Option<PathBuf>,
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
    /// Keep replay in memory without writing terminal output to disk.
    #[arg(long)]
    memory_only:bool,
    /// A line shown before the program's first output.
    #[arg(long)]
    banner: Option<String>,
    /// The program and its arguments.
    #[arg(last = true)]
    command: Vec<OsString>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if args.handoff_versions {
        let versions: Vec<String> = handoff_versions().iter().map(u32::to_string).collect();
        println!("{}", versions.join(" "));
        return ExitCode::SUCCESS;
    }
    if let Some(path) = &args.check_take_over {
        return match check_take_over(path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("humata2_holder: {}", chain(&error));
                ExitCode::FAILURE
            }
        };
    }
    let Some(socket) = args.socket.clone() else {
        eprintln!("humata2_holder: name the socket to serve with --socket");
        return ExitCode::FAILURE;
    };
    if let Some(handoff) = &args.take_over {
        // Nothing on stdout and no new session: this is the same holder.
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("humata2_holder: couldn't start its runtime: {error}");
                return ExitCode::FAILURE;
            }
        };
        return match runtime.block_on(take_over(socket, handoff)) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("humata2_holder: {}", chain(&error));
                ExitCode::FAILURE
            }
        };
    }
    let Some(cwd) = args.cwd.clone() else {
        eprintln!("humata2_holder: name the folder to start in with --cwd");
        return ExitCode::FAILURE;
    };
    if args.command.is_empty() {
        eprintln!("humata2_holder: name the program to run after `--`");
        return ExitCode::FAILURE;
    }
    // Says it is running before anything slow, so the starter can tell a
    // launch the OS is slow to make from a start-up of ours that is stuck.
    {
        let mut stdout = std::io::stdout();
        let _ = writeln!(stdout, "started");
        let _ = stdout.flush();
    }
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
        .unwrap_or_else(|| saved_scrollback_path(&socket));
    // The host may own bounded compressed checkpoints instead of a raw transcript.
    // Its holder still keeps the normal in-memory replay window.
    let memory_only=args.memory_only;
    let options = ServeOptions {
        saved: (!memory_only).then_some(saved),
        socket,
        program: PathBuf::from(program),
        args: command.collect(),
        cwd,
        cols: args.cols,
        rows: args.rows,
        scrollback_bytes: args.scrollback_bytes,
        banner: args.banner,
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
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
            eprintln!("humata2_holder: {}", chain(&error));
            ExitCode::FAILURE
        }
    }
}

/// `error` and its causes on one line.
fn chain(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    message
}
