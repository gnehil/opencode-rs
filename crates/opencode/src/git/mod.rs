pub mod branch;
pub mod diff;
pub mod status;

pub use branch::current_branch;
pub use diff::git_diff;
pub use status::git_status;
