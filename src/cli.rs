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
    /// Initialize a project. `triton init demo` creates a new project.
    /// `triton init .` minimally initializes the current repo (no component scaffold).
    Init {
        /// Project name (or '.' for minimal init in current folder)
        name: Option<String>,
        /// vcpkg triplet
        #[arg(long, default_value = "x64-windows")]
        triplet: String,
        /// CMake generator
        #[arg(long, default_value = "Ninja")]
        generator: String,
        /// C++ standard
        #[arg(long, default_value = "20")]
        cxx_std: String,
    },
    /// Add a vcpkg package to a component (creates the component if missing)
    Add {
        pkg: String,
        #[arg(long, default_value = "app")]
        component: Option<String>,
        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },
    /// Link component A to component B (target_link_libraries(A PRIVATE B))
    Link {
        /// Either `A B` or `A->B`. If two args are provided, both are used.
        edge: String,
        to: Option<String>,
    },
     /// Remove a package (or only some features) and unlink it from a component
    Remove {
        pkg: String,
        #[arg(long, default_value = "app")]
        component: String,
        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },
    /// Re-generate managed CMake blocks
    Generate,
    /// Build
    Build {
        path: String,
        #[arg(long, default_value = "debug")]
        config: String,
    },
    /// Run
    Run {
        path: String,
        #[arg(long)]
        component: Option<String>,
        #[arg(long, default_value = "debug")]
        config: String,
        #[arg(last = true)]
        args: Vec<String>,
    },
}
