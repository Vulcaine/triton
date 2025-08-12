use anyhow::Result;
use clap::Parser;

mod cli;
mod cmake;
mod commands;
mod models;
mod tools;
mod templates;
mod util;

use cli::{Cli, Commands};
use commands::{handle_add, handle_build, handle_init, handle_link, handle_run, handle_remove};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { name, triplet, generator, cxx_std } =>
            handle_init(name.as_deref(), &triplet, &generator, &cxx_std),

        Commands::Add { pkg, component, features, host } =>
            handle_add(&pkg, component.as_deref(), features.as_deref(), host),

       Commands::Remove { pkg, component, features, host } => 
        handle_remove(&pkg, Some(component.as_str()), features.as_deref(), host),

        Commands::Link { from, to } =>
            handle_link(&from, &to),

        Commands::Generate => {
            let root: models::TritonRoot = util::read_json("triton.json")?;
            for (n, c) in &root.components { cmake::rewrite_component_cmake(n, &root, c)?; }
            cmake::regenerate_root_cmake(&root)
        }

        Commands::Build { path, config } =>
            handle_build(&path, &config),

        Commands::Run { path, component, config, args } =>
            handle_run(&path, component.as_deref(), &config, &args),
    }
}
