use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use crate::contracts::ToolInvocation;
use crate::contracts::ToolInvocationStatus;
use crate::contracts::ToolResult;

#[derive(Debug, Clone)]
pub enum ToolExecutionPayload {
    System {
        summary: String,
        detected_stack: Vec<String>,
        entrypoints: Vec<String>,
        risk_flags: Vec<String>,
    },
    Plan {
        steps: Vec<String>,
    },
    Diff {
        unified_diff: String,
    },
    Verify {
        checks: Vec<String>,
        passing: bool,
    },
}

#[derive(Debug, Clone)]
pub struct ToolExecutionOutcome {
    pub result: ToolResult,
    pub payload: ToolExecutionPayload,
}

pub struct ToolExecutionContext<'a> {
    pub cwd: &'a Path,
}

pub trait ToolExecutor {
    fn execute(
        &self,
        invocation: ToolInvocation,
        context: &ToolExecutionContext<'_>,
    ) -> ToolExecutionOutcome;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SimulatedToolExecutor;

#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeToolExecutor;

impl ToolExecutor for SimulatedToolExecutor {
    fn execute(
        &self,
        invocation: ToolInvocation,
        _context: &ToolExecutionContext<'_>,
    ) -> ToolExecutionOutcome {
        let payload = match invocation.tool_id.as_str() {
            "scan_repo" => ToolExecutionPayload::System {
                summary: format!("Workflow scan completed for run {}", invocation.run_id),
                detected_stack: Vec::new(),
                entrypoints: Vec::new(),
                risk_flags: Vec::new(),
            },
            "generate_plan" => ToolExecutionPayload::Plan {
                steps: vec![
                    "Review context".to_string(),
                    "Draft changes".to_string(),
                    "Validate outcomes".to_string(),
                ],
            },
            "compute_diff" => ToolExecutionPayload::Diff {
                unified_diff: format!(
                    "+++ b/workflow-run-{}.txt\n@@\n+Simulated diff for invocation {}",
                    invocation.run_id, invocation.invocation_id
                ),
            },
            "verify" => ToolExecutionPayload::Verify {
                checks: vec!["Simulated check".to_string()],
                passing: true,
            },
            _ => ToolExecutionPayload::Plan { steps: Vec::new() },
        };

        ToolExecutionOutcome {
            result: build_result(
                invocation,
                ToolInvocationStatus::Succeeded,
                vec!["simulated execution completed".to_string()],
            ),
            payload,
        }
    }
}

impl ToolExecutor for RuntimeToolExecutor {
    fn execute(
        &self,
        invocation: ToolInvocation,
        context: &ToolExecutionContext<'_>,
    ) -> ToolExecutionOutcome {
        match invocation.tool_id.as_str() {
            "scan_repo" => execute_scan(invocation, context.cwd),
            "generate_plan" => execute_plan(invocation, context.cwd),
            "compute_diff" => execute_diff(invocation, context.cwd),
            "verify" => execute_verify(invocation, context.cwd),
            _ => ToolExecutionOutcome {
                result: build_result(
                    invocation,
                    ToolInvocationStatus::Failed,
                    vec!["unknown tool id".to_string()],
                ),
                payload: ToolExecutionPayload::Plan { steps: Vec::new() },
            },
        }
    }
}

fn execute_scan(invocation: ToolInvocation, cwd: &Path) -> ToolExecutionOutcome {
    let mut detected_stack = Vec::new();
    if cwd.join("Cargo.toml").exists() {
        detected_stack.push("rust".to_string());
    }
    if cwd.join("package.json").exists() {
        detected_stack.push("node".to_string());
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        detected_stack.push("python".to_string());
    }
    if cwd.join("go.mod").exists() {
        detected_stack.push("go".to_string());
    }

    let mut entrypoints = Vec::new();
    for entrypoint in [
        "README.md",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "Makefile",
        "justfile",
    ] {
        if cwd.join(entrypoint).exists() {
            entrypoints.push(entrypoint.to_string());
        }
    }

    let mut risk_flags = Vec::new();
    if let Ok(output) = run_git(cwd, ["status", "--porcelain"]) {
        if !stdout_text(&output).trim().is_empty() {
            risk_flags.push("dirty_worktree".to_string());
        }
    }

    let stack_label = if detected_stack.is_empty() {
        "unknown".to_string()
    } else {
        detected_stack.join(", ")
    };
    let summary = format!(
        "Scanned {} (stack: {stack_label}, entrypoints: {})",
        cwd.display(),
        entrypoints.len()
    );

    ToolExecutionOutcome {
        result: build_result(
            invocation,
            ToolInvocationStatus::Succeeded,
            vec![format!("scan completed for {}", cwd.display())],
        ),
        payload: ToolExecutionPayload::System {
            summary,
            detected_stack,
            entrypoints,
            risk_flags,
        },
    }
}

fn execute_plan(invocation: ToolInvocation, cwd: &Path) -> ToolExecutionOutcome {
    let mut steps = Vec::new();
    if cwd.join("Cargo.toml").exists() {
        steps.push("Inspect Rust workspace and affected crates".to_string());
        steps.push("Implement targeted changes and keep clippy clean".to_string());
        steps.push("Run crate tests and validate behavior".to_string());
    } else if cwd.join("package.json").exists() {
        steps.push("Inspect JavaScript/TypeScript project layout".to_string());
        steps.push("Implement scoped code changes".to_string());
        steps.push("Run lint/tests and validate outputs".to_string());
    } else {
        steps.push("Inspect project layout and key files".to_string());
        steps.push("Propose minimal implementation changes".to_string());
        steps.push("Validate behavior with available checks".to_string());
    }

    ToolExecutionOutcome {
        result: build_result(
            invocation,
            ToolInvocationStatus::Succeeded,
            vec![format!("plan generated for {}", cwd.display())],
        ),
        payload: ToolExecutionPayload::Plan { steps },
    }
}

fn execute_diff(invocation: ToolInvocation, cwd: &Path) -> ToolExecutionOutcome {
    let diff_output = run_git_allow_diff_exit(cwd, ["diff", "--no-color"]);
    let untracked_output = run_git(cwd, ["ls-files", "--others", "--exclude-standard"]);

    match (diff_output, untracked_output) {
        (Ok(diff), Ok(untracked)) => {
            let mut unified_diff = stdout_text(&diff);
            let untracked_files = stdout_text(&untracked);
            for file in untracked_files
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                unified_diff.push_str(&format!("\n+++ b/{file}\n@@\n+<untracked file>\n"));
            }

            ToolExecutionOutcome {
                result: build_result(
                    invocation,
                    ToolInvocationStatus::Succeeded,
                    vec!["diff computed".to_string()],
                ),
                payload: ToolExecutionPayload::Diff { unified_diff },
            }
        }
        (Err(err), _) | (_, Err(err)) => ToolExecutionOutcome {
            result: build_result(
                invocation,
                ToolInvocationStatus::Failed,
                vec![format!("diff execution failed: {err}")],
            ),
            payload: ToolExecutionPayload::Diff {
                unified_diff: String::new(),
            },
        },
    }
}

fn execute_verify(invocation: ToolInvocation, cwd: &Path) -> ToolExecutionOutcome {
    match run_git_allow_diff_exit(cwd, ["diff", "--check"]) {
        Ok(output) => {
            let passing = output.status.success();
            let mut checks = vec!["git diff --check".to_string()];
            let details = stdout_text(&output);
            if !details.trim().is_empty() {
                checks.push(details);
            }
            let log = if passing {
                "verify checks passed".to_string()
            } else {
                "verify checks failed".to_string()
            };
            ToolExecutionOutcome {
                result: build_result(invocation, ToolInvocationStatus::Succeeded, vec![log]),
                payload: ToolExecutionPayload::Verify { checks, passing },
            }
        }
        Err(err) => ToolExecutionOutcome {
            result: build_result(
                invocation,
                ToolInvocationStatus::Failed,
                vec![format!("verify execution failed: {err}")],
            ),
            payload: ToolExecutionPayload::Verify {
                checks: vec!["git diff --check".to_string()],
                passing: false,
            },
        },
    }
}

fn emitted_artifacts(tool_id: &str) -> Vec<String> {
    match tool_id {
        "scan_repo" => vec!["system".to_string(), "logs".to_string()],
        "generate_plan" => vec!["plan".to_string(), "logs".to_string()],
        "compute_diff" => vec!["diff".to_string(), "logs".to_string()],
        "verify" => vec!["verify".to_string(), "logs".to_string()],
        _ => Vec::new(),
    }
}

fn build_result(
    invocation: ToolInvocation,
    status: ToolInvocationStatus,
    logs: Vec<String>,
) -> ToolResult {
    ToolResult {
        run_id: invocation.run_id,
        invocation_id: invocation.invocation_id,
        tool_id: invocation.tool_id.clone(),
        status,
        artifacts_emitted: emitted_artifacts(invocation.tool_id.as_str()),
        logs,
    }
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_git<I, S>(cwd: &Path, args: I) -> std::io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git").current_dir(cwd).args(args).output()
}

fn run_git_allow_diff_exit<I, S>(cwd: &Path, args: I) -> std::io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_git(cwd, args)?;
    if output.status.success() || output.status.code() == Some(1) {
        Ok(output)
    } else {
        Err(std::io::Error::other(format!(
            "git exited with status {}",
            output.status
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Stdio;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    fn invocation(tool_id: &str) -> ToolInvocation {
        ToolInvocation {
            run_id: 7,
            invocation_id: 3,
            tool_id: tool_id.to_string(),
            requested_tier: "balanced".to_string(),
        }
    }

    fn run_git_ok(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git command should execute");
        assert!(status.success(), "git {args:?} failed with {status}");
    }

    fn make_repo_fixture() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        run_git_ok(dir.path(), &["init"]);
        run_git_ok(dir.path(), &["config", "user.name", "Test User"]);
        run_git_ok(dir.path(), &["config", "user.email", "test@example.com"]);

        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .expect("write Cargo.toml");
        fs::write(dir.path().join("README.md"), "fixture\n").expect("write README");
        run_git_ok(dir.path(), &["add", "."]);
        run_git_ok(dir.path(), &["commit", "-m", "init"]);

        fs::write(dir.path().join("README.md"), "fixture\nchanged\n").expect("modify README");
        fs::write(dir.path().join("untracked.txt"), "hello\n").expect("write untracked");
        dir
    }

    fn assert_execution_contract(outcome: &ToolExecutionOutcome, invocation: ToolInvocation) {
        assert_eq!(outcome.result.run_id, invocation.run_id);
        assert_eq!(outcome.result.invocation_id, invocation.invocation_id);
        assert_eq!(outcome.result.tool_id, invocation.tool_id);
        assert!(!outcome.result.logs.is_empty());
    }

    #[test]
    fn simulated_executor_is_deterministic_for_diff() {
        let invocation = invocation("compute_diff");
        let context = ToolExecutionContext {
            cwd: Path::new("."),
        };
        let executor = SimulatedToolExecutor;
        let first = executor.execute(invocation.clone(), &context);
        let second = executor.execute(invocation, &context);
        assert_eq!(first.result, second.result);
        match (&first.payload, &second.payload) {
            (
                ToolExecutionPayload::Diff { unified_diff: left },
                ToolExecutionPayload::Diff {
                    unified_diff: right,
                },
            ) => assert_eq!(left, right),
            _ => panic!("expected diff payload"),
        }
    }

    #[test]
    fn executors_preserve_contract_shape_for_all_workflow_tools() {
        let fixture = make_repo_fixture();
        let context = ToolExecutionContext {
            cwd: fixture.path(),
        };
        let simulated = SimulatedToolExecutor;
        let runtime = RuntimeToolExecutor;

        for tool_id in ["scan_repo", "generate_plan", "compute_diff", "verify"] {
            let invocation = invocation(tool_id);
            let sim_outcome = simulated.execute(invocation.clone(), &context);
            let runtime_outcome = runtime.execute(invocation.clone(), &context);

            assert_execution_contract(&sim_outcome, invocation.clone());
            assert_execution_contract(&runtime_outcome, invocation.clone());
            assert_eq!(sim_outcome.result.status, runtime_outcome.result.status);

            match (&sim_outcome.payload, &runtime_outcome.payload) {
                (
                    ToolExecutionPayload::System { .. },
                    ToolExecutionPayload::System {
                        summary,
                        detected_stack,
                        ..
                    },
                ) => {
                    assert!(!summary.is_empty());
                    assert!(detected_stack.iter().any(|stack| stack == "rust"));
                }
                (ToolExecutionPayload::Plan { .. }, ToolExecutionPayload::Plan { steps }) => {
                    assert!(!steps.is_empty())
                }
                (
                    ToolExecutionPayload::Diff { .. },
                    ToolExecutionPayload::Diff { unified_diff },
                ) => {
                    assert!(unified_diff.contains("+++ b/"));
                }
                (
                    ToolExecutionPayload::Verify { .. },
                    ToolExecutionPayload::Verify { checks, .. },
                ) => assert!(!checks.is_empty()),
                _ => panic!("payload variant mismatch for {tool_id}"),
            }
        }
    }

    #[test]
    fn runtime_diff_fails_outside_git_repo() {
        let temp = tempfile::tempdir().expect("tempdir");
        let context = ToolExecutionContext { cwd: temp.path() };
        let executor = RuntimeToolExecutor;
        let invocation = invocation("compute_diff");

        let outcome = executor.execute(invocation, &context);
        assert_eq!(outcome.result.status, ToolInvocationStatus::Failed);
        assert_eq!(
            outcome.result.artifacts_emitted,
            vec!["diff".to_string(), "logs".to_string()]
        );
        match outcome.payload {
            ToolExecutionPayload::Diff { unified_diff } => assert!(unified_diff.is_empty()),
            _ => panic!("expected diff payload"),
        }
    }
}
