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
    pub mod script;
    pub mod add;
    pub mod testcmd;

    pub use add::handle_add;
    pub use link::handle_link;
}

pub use crate::commands::link::handle_link;
