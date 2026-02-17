use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub cwd: PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub trait AgentAdapter {
    fn name(&self) -> &'static str;

    fn scan(&self, cwd: PathBuf) -> ExecRequest;
    fn plan(&self, cwd: PathBuf, system_context: Option<String>) -> ExecRequest;
    fn diff(&self, cwd: PathBuf, plan_context: Option<String>) -> ExecRequest;
    fn verify(&self, cwd: PathBuf) -> ExecRequest;
}

pub mod shell;
pub mod codex;
