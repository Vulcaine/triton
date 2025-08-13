pub mod cmake;
pub mod models;
pub mod templates;
pub mod util;
pub mod tools;

pub mod commands {
    pub mod link;
    pub mod build;
}

pub use crate::commands::link::handle_link;
