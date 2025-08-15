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
    pub use build::handle_build;
    pub use init::handle_init;
    pub use link::handle_link;
    pub use remove::handle_remove;
    pub use script::handle_script;
    pub use testcmd::handle_test;
}

pub use commands::{
    handle_add, handle_build, handle_init, handle_link, handle_remove,
    handle_script, handle_test,
};
