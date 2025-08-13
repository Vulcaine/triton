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
    /// `triton init .` minimally initializes the current folder (no component scaffold).
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

    /// Add one or more packages.
    ///
    /// Examples:
    ///   triton add lua sol2
    ///   triton add lua->demo sol2->demo
    ///   triton add lua sol2 demo   # if 'demo' is an existing component, link both to it
    Add {
        /// One or more items. Each item may be:
        /// - "pkg" (vcpkg), "org/repo[@branch]" (git), or "pkg->component" (link sugar).
        items: Vec<String>,

        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },

    /// Link component A to component B (target_link_libraries(A PRIVATE B))
    Link {
        /// Either `A B` or `A:B`. If two args are provided, both are used.
        edge: String,
        to: Option<String>,
    },

    /// Remove a package or unlink it from a specific component
    ///
    /// - `triton remove <pkg>`: remove from project deps and unlink from all components.
    /// - `triton remove <pkg> --component X`: only unlink from component X (keep dep).
    Remove {
        pkg: String,
        /// If provided, only unlink the pkg from this component.
        #[arg(long)]
        component: Option<String>,
        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },

    /// Re-generate managed CMake blocks
    Generate,

     /// Build
    Build {
        /// Project root path (defaults to current dir)
        #[arg(default_value = ".")]
        path: String,
        /// Configuration preset (debug|release)
        #[arg(long, default_value = "debug")]
        config: String,
        /// Interactively confirm cleaning build/<config> before building
        #[arg(long, conflicts_with = "cleanf")]
        clean: bool,
        /// Force clean build/<config> without prompting
        #[arg(long)]
        cleanf: bool,
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
