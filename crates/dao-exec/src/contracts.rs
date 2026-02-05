use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInvocationStatus {
    Succeeded,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub run_id: u64,
    pub invocation_id: u64,
    pub tool_id: String,
    pub requested_tier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub run_id: u64,
    pub invocation_id: u64,
    pub tool_id: String,
    pub status: ToolInvocationStatus,
    pub artifacts_emitted: Vec<String>,
    pub logs: Vec<String>,
}
