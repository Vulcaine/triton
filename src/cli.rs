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
    /// Initialize a project
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
        from: String,
        to: String,
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
