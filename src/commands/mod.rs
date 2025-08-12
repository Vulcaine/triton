// Keep submodules private…
mod init;
mod add;
mod build;
mod run;
mod generate;
mod link;
mod remove;

// …and re-export only the handlers for main.rs
pub use init::handle_init;
pub use add::handle_add;
pub use build::handle_build;
pub use run::handle_run;
pub use generate::handle_generate;
pub use link::handle_link;
pub use remove::handle_remove;