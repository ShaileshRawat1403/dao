use std::env;
use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use dao_core::actions::RuntimeAction;
use dao_core::actions::ShellAction;
use dao_core::persistence::PersistedExecutionMode;
use dao_core::persistence::PersistedPersonaPolicy;
use dao_core::persistence::PersistedShellEvent;
use dao_core::persistence::PersistedShellSnapshot;
use dao_core::persistence::PersistedWorkflowStatus;
use dao_core::persistence::ShellEventStore;
use dao_core::persistence::replay_latest_workflow;
use dao_core::policy_simulation::simulate_tool;
use dao_core::reducer::reduce;
use dao_core::state::ARTIFACT_SCHEMA_V1;
use dao_core::state::ApprovalAction;
use dao_core::state::ApprovalDecisionKind;
use dao_core::state::ApprovalDecisionRecord;
use dao_core::state::ApprovalRequestRecord;
use dao_core::state::ArtifactError;
use dao_core::state::DiffArtifact;
use dao_core::state::DiffFile;
use dao_core::state::DiffFileStatus;
use dao_core::state::DiffHunk;
use dao_core::state::DiffLine;
use dao_core::state::DiffLineKind;
use dao_core::state::ErrorKind;
use dao_core::state::LogEntry;
use dao_core::state::LogLevel;
use dao_core::state::LogSource;
use dao_core::state::Personality;
use dao_core::state::PlanArtifact;
use dao_core::state::PlanStep;
use dao_core::state::ShellState;
use dao_core::state::StepStatus;
use dao_core::state::SystemArtifact;
use dao_core::state::VerifyArtifact;
use dao_core::state::VerifyCheck;
use dao_core::state::VerifyCheckStatus;
use dao_core::state::VerifyOverall;
use dao_core::tool_registry::ToolId;
use dao_core::tool_registry::ToolRegistry;
use dao_core::workflow::WorkflowTemplateId;
use dao_core::workflow::workflow_template;
use dao_exec::contracts::ToolInvocation;
use dao_exec::contracts::ToolInvocationStatus;
use dao_exec::executor::SimulatedToolExecutor;
use dao_exec::executor::ToolExecutionContext;
use dao_exec::executor::ToolExecutionPayload;
use dao_exec::executor::ToolExecutor;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return Ok(());
    };

    match command.as_str() {
        "--help" | "-h" | "help" => {
            print_help();
            Ok(())
        }
        "--version" | "-V" | "version" => {
            println!("dao {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "run" => {
            let repo = parse_repo_arg(args.collect::<Vec<_>>())?;
            run_workflow(repo)
        }
        _ => {
            print_help();
            Err(format!("unknown command: {command}").into())
        }
    }
}

fn parse_repo_arg(args: Vec<String>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut repo = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--repo requires a path".into());
                };
                repo = Some(PathBuf::from(value));
                i += 2;
            }
            other => {
                return Err(format!("unsupported argument: {other}").into());
            }
        }
    }
    Ok(repo.unwrap_or_else(|| PathBuf::from(".")))
}

fn run_workflow(repo: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let repo = repo.canonicalize()?;
    let dao_dir = repo.join(".dao");
    fs::create_dir_all(&dao_dir)?;
    let events_path = dao_dir.join("workflow-events.jsonl");
    let snapshot_path = dao_dir.join("snapshot.json");
    let mut store = ShellEventStore::open(&events_path)?;

    let records = store.load()?;
    let prior_run_id = replay_latest_workflow(&records)
        .map(|run| run.run_id)
        .unwrap_or(0);
    let run_id = prior_run_id.saturating_add(1);

    let project_name = repo
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .to_string();
    let mut state = ShellState::new(project_name, Personality::Pragmatic);
    let policy_tier = state.approval.policy_tier;

    let mut last_seq = store.append(PersistedShellEvent::WorkflowRunStarted {
        run_id,
        template_id: "scan_plan_diff_verify".to_string(),
        execution_mode: PersistedExecutionMode::Simulated,
        policy_tier: policy_tier.label().to_string(),
        persona_policy: PersistedPersonaPolicy {
            tier_ceiling: state.sm.persona_policy.tier_ceiling.label().to_string(),
            explanation_depth: state.sm.persona_policy.explanation_depth.label().to_string(),
            output_format: state.sm.persona_policy.output_format.label().to_string(),
        },
    })?;
    persist_snapshot(&snapshot_path, last_seq, &store)?;

    let template = workflow_template(WorkflowTemplateId::ScanPlanDiffVerify);
    let executor = SimulatedToolExecutor;
    let context = ToolExecutionContext { cwd: &repo };

    for (step_index, step) in template.steps.iter().enumerate() {
        let spec = ToolRegistry::get(step.tool_id);
        let sim = simulate_tool(policy_tier, step.tool_id);
        reduce(
            &mut state,
            ShellAction::Runtime(RuntimeAction::AssessPolicyGate {
                run_id,
                action: ApprovalAction::Execute,
                risk: spec.risk_class,
                reason: sim.reason.to_string(),
            }),
        );

        if sim.blocked {
            last_seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
                run_id,
                status: PersistedWorkflowStatus::Blocked,
                step_index,
                reason: Some(sim.reason.to_string()),
            })?;
            persist_snapshot(&snapshot_path, last_seq, &store)?;
            println!("workflow blocked at {}: {}", step.step_id, sim.reason);
            return Ok(());
        }

        let invocation_id = (step_index as u64).saturating_add(1);
        let request_id = format!("req-{run_id}-{invocation_id}");

        if sim.requirement.label() == "require-approval" {
            let request = ApprovalRequestRecord {
                request_id: request_id.clone(),
                run_id,
                action: ApprovalAction::Execute,
                risk: spec.risk_class,
                reason: sim.reason.to_string().into(),
                preview: format!("workflow tool {}", step.tool_id.as_str()).into(),
                created_at_ms: None,
            };
            reduce(
                &mut state,
                ShellAction::Runtime(RuntimeAction::RequestApproval(request)),
            );
            last_seq = store.append(PersistedShellEvent::ApprovalRequested {
                request_id: request_id.clone(),
                run_id,
                invocation_id,
                tool_id: step.tool_id.as_str().to_string(),
                risk: spec.risk_class.label().to_string(),
                preview: format!("workflow tool {}", step.tool_id.as_str()),
            })?;

            if !prompt_approval(step.tool_id)? {
                let decision = ApprovalDecisionRecord {
                    request_id: request_id.clone(),
                    run_id,
                    action: ApprovalAction::Execute,
                    decision: ApprovalDecisionKind::Denied,
                    timestamp_ms: 0,
                };
                reduce(
                    &mut state,
                    ShellAction::Runtime(RuntimeAction::ResolveApproval(decision)),
                );
                last_seq = store.append(PersistedShellEvent::ApprovalResolved {
                    request_id,
                    run_id,
                    decision: "denied".to_string(),
                })?;
                persist_snapshot(&snapshot_path, last_seq, &store)?;
                last_seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
                    run_id,
                    status: PersistedWorkflowStatus::Blocked,
                    step_index,
                    reason: Some("approval denied".to_string()),
                })?;
                persist_snapshot(&snapshot_path, last_seq, &store)?;
                println!("workflow blocked: approval denied at {}", step.step_id);
                return Ok(());
            }

            let decision = ApprovalDecisionRecord {
                request_id: request_id.clone(),
                run_id,
                action: ApprovalAction::Execute,
                decision: ApprovalDecisionKind::Approved,
                timestamp_ms: 0,
            };
            reduce(
                &mut state,
                ShellAction::Runtime(RuntimeAction::ResolveApproval(decision)),
            );
            last_seq = store.append(PersistedShellEvent::ApprovalResolved {
                request_id,
                run_id,
                decision: "approved".to_string(),
            })?;
            persist_snapshot(&snapshot_path, last_seq, &store)?;
        }

        let invocation = ToolInvocation {
            run_id,
            invocation_id,
            tool_id: step.tool_id.as_str().to_string(),
            requested_tier: policy_tier.label().to_string(),
        };
        last_seq = store.append(PersistedShellEvent::ToolInvocationIssued {
            run_id,
            invocation_id,
            tool_id: step.tool_id.as_str().to_string(),
        })?;
        let outcome = executor.execute(invocation, &context);

        apply_execution_outcome(
            &mut state,
            run_id,
            invocation_id,
            payload_to_result(step.tool_id, outcome.payload),
            &outcome.result.logs,
        );

        last_seq = store.append(PersistedShellEvent::ToolResultRecorded {
            run_id,
            invocation_id,
            tool_id: step.tool_id.as_str().to_string(),
            status: status_label(outcome.result.status).to_string(),
        })?;

        let workflow_status = match outcome.result.status {
            ToolInvocationStatus::Succeeded => PersistedWorkflowStatus::Running,
            ToolInvocationStatus::Failed => PersistedWorkflowStatus::Failed,
            ToolInvocationStatus::Blocked => PersistedWorkflowStatus::Blocked,
        };
        last_seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
            run_id,
            status: workflow_status,
            step_index: step_index.saturating_add(1),
            reason: if outcome.result.status == ToolInvocationStatus::Succeeded {
                None
            } else {
                Some("tool execution did not succeed".to_string())
            },
        })?;
        persist_snapshot(&snapshot_path, last_seq, &store)?;

        if outcome.result.status != ToolInvocationStatus::Succeeded {
            println!(
                "workflow ended at {} with status {}",
                step.step_id,
                status_label(outcome.result.status)
            );
            return Ok(());
        }
    }

    last_seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
        run_id,
        status: PersistedWorkflowStatus::Completed,
        step_index: template.steps.len(),
        reason: None,
    })?;
    persist_snapshot(&snapshot_path, last_seq, &store)?;

    println!("workflow {run_id} completed");
    println!("events: {}", events_path.display());
    println!("snapshot: {}", snapshot_path.display());
    Ok(())
}

enum StepResult {
    System(SystemArtifact),
    Plan(PlanArtifact),
    Diff(DiffArtifact),
    Verify(VerifyArtifact),
}

fn payload_to_result(tool_id: ToolId, payload: ToolExecutionPayload) -> StepResult {
    match (tool_id, payload) {
        (
            ToolId::ScanRepo,
            ToolExecutionPayload::System {
                summary,
                detected_stack,
                entrypoints,
                risk_flags,
            },
        ) => StepResult::System(SystemArtifact {
            schema_version: ARTIFACT_SCHEMA_V1,
            run_id: 0,
            artifact_id: 0,
            repo_root: String::new(),
            detected_stack,
            entrypoints,
            risk_flags,
            summary,
            error: None,
        }),
        (ToolId::GeneratePlan, ToolExecutionPayload::Plan { steps }) => StepResult::Plan(PlanArtifact {
            schema_version: ARTIFACT_SCHEMA_V1,
            run_id: 0,
            artifact_id: 0,
            title: "Workflow plan".to_string(),
            steps: steps
                .into_iter()
                .enumerate()
                .map(|(idx, label)| PlanStep {
                    id: format!("step-{}", idx.saturating_add(1)),
                    label,
                    status: StepStatus::Pending,
                })
                .collect(),
            assumptions: Vec::new(),
            error: None,
        }),
        (ToolId::ComputeDiff, ToolExecutionPayload::Diff { unified_diff }) => StepResult::Diff(DiffArtifact {
            schema_version: ARTIFACT_SCHEMA_V1,
            run_id: 0,
            artifact_id: 0,
            files: vec![DiffFile {
                path: "workflow.diff".to_string(),
                status: DiffFileStatus::Modified,
                hunks: vec![DiffHunk {
                    header: "@@".to_string(),
                    lines: unified_diff
                        .lines()
                        .map(|line| DiffLine {
                            kind: if line.starts_with('+') {
                                DiffLineKind::Add
                            } else if line.starts_with('-') {
                                DiffLineKind::Remove
                            } else {
                                DiffLineKind::Context
                            },
                            text: line.to_string(),
                        })
                        .collect(),
                }],
            }],
            summary: "Diff preview".to_string(),
            error: None,
        }),
        (ToolId::Verify, ToolExecutionPayload::Verify { checks, passing }) => StepResult::Verify(VerifyArtifact {
            schema_version: ARTIFACT_SCHEMA_V1,
            run_id: 0,
            artifact_id: 0,
            checks: checks
                .into_iter()
                .map(|check| VerifyCheck {
                    name: check,
                    status: if passing {
                        VerifyCheckStatus::Pass
                    } else {
                        VerifyCheckStatus::Fail
                    },
                    details: None,
                })
                .collect(),
            overall: if passing {
                VerifyOverall::Passing
            } else {
                VerifyOverall::Failing
            },
            error: if passing {
                None
            } else {
                Some(ArtifactError {
                    kind: ErrorKind::Runtime,
                    message: "verification failed".into(),
                })
            },
        }),
        (_, _) => StepResult::Plan(PlanArtifact {
            schema_version: ARTIFACT_SCHEMA_V1,
            run_id: 0,
            artifact_id: 0,
            title: "Workflow plan".to_string(),
            steps: Vec::new(),
            assumptions: Vec::new(),
            error: Some(ArtifactError {
                kind: ErrorKind::Unknown,
                message: "payload mismatch".into(),
            }),
        }),
    }
}

fn apply_execution_outcome(
    state: &mut ShellState,
    run_id: u64,
    artifact_id: u64,
    step_result: StepResult,
    logs: &[String],
) {
    match step_result {
        StepResult::System(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(state, ShellAction::Runtime(RuntimeAction::SetSystemArtifact(artifact)));
        }
        StepResult::Plan(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(state, ShellAction::Runtime(RuntimeAction::SetPlanArtifact(artifact)));
        }
        StepResult::Diff(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(state, ShellAction::Runtime(RuntimeAction::SetDiffArtifact(artifact)));
        }
        StepResult::Verify(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(state, ShellAction::Runtime(RuntimeAction::SetVerifyArtifact(artifact)));
        }
    }

    for log in logs {
        reduce(
            state,
            ShellAction::Runtime(RuntimeAction::AppendStructuredLog(LogEntry {
                seq: 0,
                level: LogLevel::Info,
                ts_ms: None,
                source: LogSource::Runtime,
                context: Some("executor".to_string()),
                message: log.clone(),
                run_id,
            })),
        );
    }
}

fn persist_snapshot(
    snapshot_path: &Path,
    seq: u64,
    store: &ShellEventStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow = replay_latest_workflow(&store.load()?)
        .map(|mut run| {
            if run.status == PersistedWorkflowStatus::Running {
                run.status = PersistedWorkflowStatus::Blocked;
                run.blocked_reason = Some("interrupted".to_string());
            }
            run
        });
    let snapshot = PersistedShellSnapshot {
        version: 1,
        seq,
        workflow,
    };
    fs::write(snapshot_path, serde_json::to_vec_pretty(&snapshot)?)?;
    Ok(())
}

fn prompt_approval(tool_id: ToolId) -> io::Result<bool> {
    print!("approval required for {} [y/N]: ", tool_id.as_str());
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "YES"))
}

fn status_label(status: ToolInvocationStatus) -> &'static str {
    match status {
        ToolInvocationStatus::Succeeded => "succeeded",
        ToolInvocationStatus::Failed => "failed",
        ToolInvocationStatus::Blocked => "blocked",
    }
}

fn print_help() {
    println!("dao {}", env!("CARGO_PKG_VERSION"));
    println!("Usage:");
    println!("  dao run --repo PATH");
    println!("  dao --help");
    println!("  dao --version");
}
