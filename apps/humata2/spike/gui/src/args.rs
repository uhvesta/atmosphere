//! The command line.

use std::num::ParseIntError;

use thiserror::Error;

pub const USAGE: &str = "\
usage: humata2-gui-spike [options]

  --screen tree|diff|terminal   the screen to open on (default tree)
  --bench scroll|idle|start     run a benchmark, print one JSON line, exit
  --exit-after-ms N             quit N milliseconds after starting
  --step-px N                   pixels scrolled per frame in --bench scroll (default 200)
  --wrap on|off                 wrap long diff lines (default on)
  --animate                     start the terminal's fake output running
  --bench-secs N                how long --bench scroll runs on the terminal (default 10)
  --idle-ms N                   how long --bench idle waits after first paint (default 10000)
  --help                        print this and exit
";

/// The screen the window opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Tree,
    Diff,
    Terminal,
}

impl Screen {
    pub const ALL: [Screen; 3] = [Screen::Tree, Screen::Diff, Screen::Terminal];

    /// The name used on the command line and in benchmark output.
    pub fn name(self) -> &'static str {
        match self {
            Screen::Tree => "tree",
            Screen::Diff => "diff",
            Screen::Terminal => "terminal",
        }
    }
}

/// Which benchmark to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bench {
    /// Scroll the screen top to bottom and report frame times.
    Scroll,
    /// Sit idle after the first paint and report CPU time and memory.
    Idle,
    /// Report the time from `main` to the first paint.
    Start,
}

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Args {
    pub screen: Screen,
    pub bench: Option<Bench>,
    pub exit_after_ms: Option<u64>,
    pub step_px: u32,
    pub wrap: bool,
    pub animate: bool,
    pub bench_secs: u64,
    pub idle_ms: u64,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            screen: Screen::Tree,
            bench: None,
            exit_after_ms: None,
            step_px: 200,
            wrap: true,
            animate: false,
            bench_secs: 10,
            idle_ms: 10_000,
        }
    }
}

/// The command line parsed: run, or print the usage.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Run(Args),
    Help,
}

/// Why the command line could not be read.
#[derive(Debug, Error, PartialEq)]
pub enum ArgsError {
    #[error("unknown option {flag}; run with --help to see the options")]
    UnknownFlag { flag: String },
    #[error("{flag} needs a value after it; run with --help to see what it takes")]
    MissingValue { flag: &'static str },
    #[error("{flag} does not take {value:?}; use one of {expected}")]
    InvalidValue {
        flag: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error("{flag} needs a whole number of zero or more, not {value:?}")]
    InvalidNumber {
        flag: &'static str,
        value: String,
        #[source]
        source: ParseIntError,
    },
}

/// Reads the arguments after the program name.
///
/// # Errors
///
/// When an option is unknown, lacks its value, or has one it doesn't take.
pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Parsed, ArgsError> {
    let mut args = Args::default();
    let mut arguments = arguments.into_iter();
    while let Some(flag) = arguments.next() {
        let mut value = |flag: &'static str| arguments.next().ok_or(ArgsError::MissingValue { flag });
        match flag.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--screen" => {
                let value = value("--screen")?;
                args.screen = Screen::ALL
                    .into_iter()
                    .find(|screen| screen.name() == value)
                    .ok_or(ArgsError::InvalidValue {
                        flag: "--screen",
                        value,
                        expected: "tree, diff or terminal",
                    })?;
            }
            "--bench" => {
                let value = value("--bench")?;
                args.bench = Some(match value.as_str() {
                    "scroll" => Bench::Scroll,
                    "idle" => Bench::Idle,
                    "start" => Bench::Start,
                    _ => {
                        return Err(ArgsError::InvalidValue {
                            flag: "--bench",
                            value,
                            expected: "scroll, idle or start",
                        });
                    }
                });
            }
            "--wrap" => {
                let value = value("--wrap")?;
                args.wrap = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => {
                        return Err(ArgsError::InvalidValue {
                            flag: "--wrap",
                            value,
                            expected: "on or off",
                        });
                    }
                };
            }
            "--animate" => args.animate = true,
            "--exit-after-ms" => {
                args.exit_after_ms = Some(number("--exit-after-ms", value("--exit-after-ms")?)?);
            }
            "--step-px" => args.step_px = number("--step-px", value("--step-px")?)?,
            "--bench-secs" => args.bench_secs = number("--bench-secs", value("--bench-secs")?)?,
            "--idle-ms" => args.idle_ms = number("--idle-ms", value("--idle-ms")?)?,
            _ => return Err(ArgsError::UnknownFlag { flag }),
        }
    }
    Ok(Parsed::Run(args))
}

fn number<N: std::str::FromStr<Err = ParseIntError>>(
    flag: &'static str,
    value: String,
) -> Result<N, ArgsError> {
    value
        .parse()
        .map_err(|source| ArgsError::InvalidNumber { flag, value, source })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(arguments: &[&str]) -> Result<Parsed, ArgsError> {
        parse(arguments.iter().map(|argument| argument.to_string()))
    }

    #[test]
    fn no_arguments_open_the_tree_without_a_benchmark() {
        assert_eq!(parse_all(&[]), Ok(Parsed::Run(Args::default())));
    }

    #[test]
    fn options_set_the_screen_benchmark_and_numbers() {
        let parsed = parse_all(&[
            "--screen", "diff", "--bench", "scroll", "--wrap", "off", "--step-px", "120",
            "--exit-after-ms", "500",
        ]);
        assert_eq!(
            parsed,
            Ok(Parsed::Run(Args {
                screen: Screen::Diff,
                bench: Some(Bench::Scroll),
                wrap: false,
                step_px: 120,
                exit_after_ms: Some(500),
                ..Args::default()
            }))
        );
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(matches!(
            parse_all(&["--screen", "chat"]),
            Err(ArgsError::InvalidValue { .. })
        ));
        assert!(matches!(
            parse_all(&["--bench"]),
            Err(ArgsError::MissingValue { .. })
        ));
        assert!(matches!(
            parse_all(&["--step-px", "-3"]),
            Err(ArgsError::InvalidNumber { .. })
        ));
        assert!(matches!(
            parse_all(&["--fast"]),
            Err(ArgsError::UnknownFlag { .. })
        ));
    }
}
