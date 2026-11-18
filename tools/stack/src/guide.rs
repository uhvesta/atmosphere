//! `stack guide`: the agent guide, or the PR template in use.

use std::path::Path;

use crate::body::Template;
use crate::cli::GuideArgs;
use crate::config::Config;
use crate::error::Error;

const GUIDE: &str = include_str!("guide.md");

pub(crate) fn run(args: &GuideArgs, _dir: &Path) -> Result<(), Error> {
    if !args.template {
        print!("{GUIDE}");
        return Ok(());
    }
    let config = Config::load()?;
    let template = Template::load(&config)?;
    match &template.path {
        Some(path) => println!("PR template: {}\n", path.display()),
        None => println!(
            "PR template: built in (to change it, put a file in {} and name it with `template = \"<file>\"` in stack.toml)\n",
            config.dir.display()
        ),
    }
    print!("{}", template.text);
    Ok(())
}
