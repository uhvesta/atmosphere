//! Test helper for the real-CLI fork-and-resume fixtures: forks one
//! conversation through Humata's adapter, as the daemon does, so the fixture
//! can resume the fork in the installed CLI.
//!
//! Usage: `fork_helper <claude|codex|copilot> <home> <cli-home> <from-folder>
//! <id> <to-folder> <new-id>`. `<cli-home>` is the CLI's own home
//! (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME`) as the fixture set it.
//! Prints one JSON object with the new conversation's id and files.

use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use humata2_conversation::{Format, Placement, adapter};

#[derive(Debug, thiserror::Error)]
enum HelperError {
    #[error(
        "fork_helper needs seven arguments: <claude|codex|copilot> <home> <cli-home> <from-folder> <id> <to-folder> <new-id>, but got {given}. Run it from the fixture script, which passes them"
    )]
    Usage { given: usize },

    #[error("{name:?} is not a conversation format fork_helper knows. Use claude, codex or copilot")]
    UnknownFormat { name: String },

    #[error(
        "Humata's {format} adapter could not fork conversation {id}. The adapter's own refusal is the source of this error; the CLI's files were not changed"
    )]
    Fork {
        format: Format,
        id: String,
        #[source]
        source: humata2_conversation::ConversationError,
    },
}

fn run(args: &[String]) -> Result<String, HelperError> {
    let [name, home, cli_home, from_folder, id, to_folder, new_id] = args else {
        return Err(HelperError::Usage { given: args.len() });
    };
    let format = match name.as_str() {
        "claude" => Format::ClaudeJsonl,
        "codex" => Format::CodexRollout,
        "copilot" => Format::CopilotSessionState,
        _ => return Err(HelperError::UnknownFormat { name: name.clone() }),
    };
    let placement = |folder: &String| Placement {
        home: PathBuf::from(home),
        cli_home: Some(PathBuf::from(cli_home)),
        folder: PathBuf::from(folder),
    };
    let fork_error = |source| HelperError::Fork { format, id: id.clone(), source };
    let adapter = adapter(format);
    let parent = adapter.locate(&placement(from_folder), id).map_err(fork_error)?;
    let forked = adapter.fork(&parent, None, new_id, &placement(to_folder)).map_err(fork_error)?;
    let files: Vec<String> = forked.files.iter().map(|file| file.path.to_string_lossy().into_owned()).collect();
    Ok(serde_json::json!({ "id": forked.id, "files": files }).to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            let mut cause = error.source();
            while let Some(inner) = cause {
                eprintln!("  caused by: {inner}");
                cause = inner.source();
            }
            ExitCode::FAILURE
        }
    }
}
