use anyhow::Result;
use clap::Parser;

mod cli;
mod cmake;
mod commands;
mod models;
mod tools;
mod templates;
mod util;

use cli::{Cli, Commands, CmakeCommands};
use commands::{
    handle_add, handle_build, handle_generate, handle_init, handle_link, handle_remove,
    handle_run, handle_script, handle_test, handle_cmake_install
};

use std::borrow::Cow;

fn opt_str(opt: &Option<String>) -> Option<&str> {
    opt.as_deref()
}

fn parse_edge<'a>(edge: &'a str, to: &'a Option<String>) -> Result<(Cow<'a, str>, Cow<'a, str>)> {
    if let Some(t) = to.as_ref() {
        return Ok((Cow::from(edge), Cow::from(t.as_str())));
    }
    if let Some((a, b)) = edge.split_once(':') {
        let from = a.trim();
        let to = b.trim();
        if !from.is_empty() && !to.is_empty() {
            return Ok((Cow::from(from), Cow::from(to)));
        }
    }
    anyhow::bail!("Use `triton link A B` or `triton link A:B`")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { name, generator, cxx_std } =>
            handle_init(opt_str(&name), &generator, &cxx_std),

        Commands::Add { items, features, host } =>
            handle_add(&items, opt_str(&features), host),

        Commands::Remove { pkg, component, features, host } =>
            handle_remove(&pkg, opt_str(&component), opt_str(&features), host),

        Commands::Generate =>
            handle_generate(),

        Commands::Build { path, config, clean, cleanf } =>
            handle_build(&path, &config, clean, cleanf),

        Commands::Run { path, component, config, args } =>
            handle_run(&path, opt_str(&component), &config, &args),

        Commands::Link { edge, to } => {
            let (from, to) = parse_edge(&edge, &to)?;
            handle_link(&from, &to)
        }

        Commands::Test { path, config } =>
            handle_test(&path, &config),

        Commands::Cmake { cmd } => match cmd {
            CmakeCommands::Install { version } => handle_cmake_install(version),
        },

        Commands::Script(v) =>
            handle_script(&v),
    }
}
