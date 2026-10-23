pub mod crawler;
pub mod judge;
pub mod pipeline;
pub mod planner;
// Shared module tree: progressive responses are used only by the MCP binary.
#[allow(dead_code)]
pub mod progressive;
pub mod publisher;
pub mod quality;
pub mod summarizer;
