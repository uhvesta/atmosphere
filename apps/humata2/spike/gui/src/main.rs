//! `humata2-gui-spike`: opens the spike's window, or runs one of its
//! benchmarks and prints the result as a JSON line. See README.md.

use std::process::ExitCode;

use humata2_gui_spike::args::{self, Parsed, USAGE};
use humata2_gui_spike::tree::TreeStore;
use humata2_gui_spike::{bench, shell};

fn main() -> ExitCode {
    bench::mark_start();
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("humata2-gui-spike: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    // Made before the app starts, so a failure exits cleanly with a message.
    let tree_store = match TreeStore::new() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("humata2-gui-spike: {error}");
            return ExitCode::FAILURE;
        }
    };
    gpui_kit::application().run(move |cx| {
        humata2_gui_spike::init(cx);
        if let Err(error) = shell::open(args, tree_store, cx) {
            eprintln!(
                "humata2-gui-spike: could not open the window: {error}. \
                 The spike needs a desktop session; on Linux check that DISPLAY or \
                 WAYLAND_DISPLAY is set and the GPU driver supports Vulkan"
            );
            std::process::exit(1);
        }
    });
    ExitCode::SUCCESS
}
