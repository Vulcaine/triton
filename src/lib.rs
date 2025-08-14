pub mod cmake;
pub mod models;
pub mod templates;
pub mod util;
pub mod tools;

pub mod commands {
    pub mod link;
    pub mod build;
    pub mod init;
    pub mod remove;
}

pub use crate::commands::link::handle_link;
