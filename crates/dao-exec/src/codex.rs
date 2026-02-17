use std::path::PathBuf;

use super::{AgentAdapter, ExecRequest};

pub struct CodexAdapter;

impl CodexAdapter {
    fn req(cwd: PathBuf, args: &[&str]) -> ExecRequest {
        ExecRequest {
            cwd,
            program: "codex".to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Vec::new(),
        }
    }
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn scan(&self, cwd: PathBuf) -> ExecRequest {
        // Assuming `codex scan` exists and outputs JSON or text summary
        Self::req(cwd, &["scan"])
    }

    fn plan(&self, cwd: PathBuf, system_context: Option<String>) -> ExecRequest {
        let mut args = vec!["plan"];
        if let Some(ctx) = system_context {
            // In a real implementation, we might pass context via a file or stdin
            // For CLI args, we'd need to be careful about length limits
            // Here we just use a flag as an example
            // args.push("--context");
            // args.push(&ctx);
        }
        Self::req(cwd, &args)
    }

    fn diff(&self, cwd: PathBuf, _plan_context: Option<String>) -> ExecRequest {
        Self::req(cwd, &["diff"])
    }

    fn verify(&self, cwd: PathBuf) -> ExecRequest {
        Self::req(cwd, &["verify"])
    }
}
