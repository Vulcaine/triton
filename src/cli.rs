// cli.rs

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
        items: Vec<String>,
        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },

    /// Link component A to component B (target_link_libraries(A PRIVATE B))
    Link {
        edge: String,
        to: Option<String>,
    },

    /// Unlink A from B (remove the dependency edge).
    ///
    /// Examples:
    ///   triton unlink sdl2:Game     — Game no longer depends on sdl2
    ///   triton unlink sdl2          — remove sdl2 from ALL components' link lists
    Unlink {
        edge: String,
        to: Option<String>,
    },

    /// Remove a package or unlink it from a specific component
    Remove {
        pkg: String,
        #[arg(long)]
        component: Option<String>,
        #[arg(long)]
        features: Option<String>,
        #[arg(long)]
        host: bool,
    },

    /// Re-generate managed CMake blocks
    Generate,

    /// Build the project
    Build {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long, default_value = "debug")]
        config: String,
        #[arg(long, conflicts_with = "cleanf")]
        clean: bool,
        #[arg(long)]
        cleanf: bool,
    },

    /// Run a component (usually an executable target)
    Run {
        path: String,
        #[arg(long)]
        component: Option<String>,
        #[arg(long, default_value = "debug")]
        config: String,
        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Run tests via CTest
    Test {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long, default_value = "debug")]
        config: String,
    },

    /// Manage cmake (installation helpers)
    Cmake {
        #[command(subcommand)]
        cmd: CmakeCommands,
    },

    /// Remove a component entirely (deletes from triton.json, unlinks from
    /// all dependents, removes the on-disk directory, and regenerates CMake).
    RemoveComponent {
        /// Component name to remove
        name: String,
    },

    /// Search for the CMake package name of an installed vcpkg dependency.
    ///
    /// Scans vcpkg_installed/<triplet>/share/ for matching Config.cmake files.
    FindTarget {
        /// The dependency name to search for (e.g. "openal-soft", "directxtex")
        dep: String,
    },

    /// Any unknown subcommand is treated as a script name + args.
    #[command(external_subcommand)]
    Script(Vec<String>),
}

#[derive(Subcommand)]
pub enum CmakeCommands {
    /// Ensure cmake >= <version> is installed (tries package managers cross-platform)
    ///
    /// Usage:
    ///   triton cmake install --version 3.30.1
    ///
    /// If --version is omitted, Triton uses the project's required version.
    Install {
        /// Minimum CMake version to ensure (e.g. "3.30.1").
        /// If omitted, uses the version from `effective_cmake_version()`.
        #[arg(long)]
        version: Option<String>,
    },
}
