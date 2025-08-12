use anyhow::Result;
use clap::Parser;

mod cli;
mod cmake;
mod commands; // re-exports handlers
mod models;
mod tools;
mod templates;
mod util;

use cli::{Cli, Commands};
use commands::{handle_add, handle_build, handle_generate, handle_init, handle_link, handle_run};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { name, triplet, generator, cxx_std } =>
            handle_init(name.as_deref(), &triplet, &generator, &cxx_std),

        Commands::Add { pkg, component, features, host } =>
            handle_add(&pkg, &component, features.as_deref(), host),

        Commands::Link { from, to } =>
            handle_link(&from, &to),

        Commands::Generate =>
            handle_generate(),

        Commands::Build { path, config } =>
            handle_build(&path, &config),

        Commands::Run { path, component, config, args } =>
            handle_run(&path, component.as_deref(), &config, &args),
    }
}
