use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "triton")]
#[command(about = "A minimal C++ project manager on top of vcpkg + CMake")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new project. If --name is given, create a new folder; otherwise use cwd.
    Init {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "x64-windows")]
        triplet: String,
        #[arg(long, default_value = "Ninja")]
        generator: String,
        #[arg(long, default_value = "20")]
        cxx_std: String,
    },

    /// Add a package to vcpkg.json and wire it into a component
    Add {
        pkg: String,
        #[arg(long, default_value = "app")]
        component: String,
        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },

    /// Rescan components, rewrite CMake managed blocks
    Generate,

    /// Configure+build a project at PATH (default "."); choose --config debug|release
    Build {
        path: Option<String>,
        #[arg(long, default_value = "debug")]
        config: String,
    },

    /// Build then run an executable component (default: app)
    Run {
        path: Option<String>,
        #[arg(long)]
        component: Option<String>,
        #[arg(long, default_value = "debug")]
        config: String,
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}
