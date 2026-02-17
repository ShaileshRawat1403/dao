use std::path::PathBuf;

use super::{AgentAdapter, ExecRequest};

pub struct ShellAdapter;

impl ShellAdapter {
    fn req(cwd: PathBuf, program: &str, args: &[&str]) -> ExecRequest {
        ExecRequest {
            cwd,
            program: program.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
        }
    }
}

impl AgentAdapter for ShellAdapter {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn scan(&self, cwd: PathBuf) -> ExecRequest {
        Self::req(cwd, "git", &["status", "--porcelain=v1"])
    }

    fn plan(&self, cwd: PathBuf, _system_context: Option<String>) -> ExecRequest {
        Self::req(cwd, "echo", &["PLAN: adapter wired"])
    }

    fn diff(&self, cwd: PathBuf, _plan_context: Option<String>) -> ExecRequest {
        Self::req(cwd, "git", &["diff"])
    }

    fn verify(&self, cwd: PathBuf) -> ExecRequest {
        Self::req(cwd, "cargo", &["test", "-q"])
    }
}
