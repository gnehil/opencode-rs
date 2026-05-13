mod default;
mod info;
mod mode;
mod model;
pub mod prompts;

pub use default::*;
pub use info::*;
pub use mode::*;
pub use model::*;
pub use prompts::*;

pub const DEFAULT_AGENT_NAME: &str = "build";

pub fn get_default_agent() -> AgentInfo {
    build_agent()
}

pub fn get_agent(name: &str) -> Option<AgentInfo> {
    match name {
        "build" => Some(build_agent()),
        "plan" => Some(plan_agent()),
        "general" => Some(general_agent()),
        "explore" => Some(explore_agent()),
        "scout" => Some(scout_agent()),
        "compaction" => Some(compaction_agent()),
        "title" => Some(title_agent()),
        "summary" => Some(summary_agent()),
        _ => None,
    }
}
