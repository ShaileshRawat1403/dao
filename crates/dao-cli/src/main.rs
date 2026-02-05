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
use dao_core::persistence::PersistedShellEventRecord;
use dao_core::persistence::PersistedShellSnapshot;
use dao_core::persistence::PersistedWorkflowStatus;
use dao_core::persistence::ReplayedWorkflowRun;
use dao_core::persistence::ShellEventStore;
use dao_core::persistence::replay_latest_workflow;
use dao_core::persistence::replay_workflow_from;
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
use dao_core::state::PolicyTier;
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
        if err.to_string().starts_with("malformed resume state") {
            std::process::exit(2);
        }
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
        "replay" => replay_workflow(args.collect::<Vec<_>>()),
        "resume" => {
            let repo = parse_repo_arg(args.collect::<Vec<_>>())?;
            resume_workflow(repo)
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

fn replay_workflow(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let mut last = false;
    let mut repo = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--last" => {
                last = true;
                i += 1;
            }
            "--repo" => {
                let Some(value) = args.get(i + 1) else {
                    return Err("--repo requires a path".into());
                };
                repo = Some(PathBuf::from(value));
                i += 2;
            }
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }

    if !last {
        return Err("replay currently supports only --last".into());
    }

    let repo = repo.unwrap_or_else(|| PathBuf::from(".")).canonicalize()?;
    let (store, snapshot_path) = open_store_for_repo(&repo)?;
    let records = store.load()?;
    let run = load_latest_run(&store, &snapshot_path)?;

    let Some(run) = run else {
        println!("no workflow runs found");
        return Ok(());
    };

    let template = workflow_template(WorkflowTemplateId::ScanPlanDiffVerify);
    let current_step = template.steps.get(run.step_index).map(|step| step.step_id);
    let next_step = template.steps.get(run.step_index).map(|step| step.step_id);
    let (system, plan, diff, verify) = artifact_flags(run.step_index);
    let last_log_seq = records.iter().map(|record| record.seq).max().unwrap_or(0);

    println!("run_id: {}", run.run_id);
    println!("status: {}", persisted_status_label(run.status));
    println!(
        "current_step: {}",
        current_step.unwrap_or("<completed>")
    );
    println!("next_step: {}", next_step.unwrap_or("<none>"));

    match (
        run.pending_request_id.as_deref(),
        run.pending_tool_id.as_deref(),
        run.pending_invocation_id,
    ) {
        (Some(request_id), Some(tool_id), Some(invocation_id)) => println!(
            "pending_approval: request_id={request_id} tool_id={tool_id} invocation_id={invocation_id}"
        ),
        _ => println!("pending_approval: none"),
    }

    println!(
        "artifacts: system={} plan={} diff={} verify={}",
        system, plan, diff, verify
    );
    println!("last_log_seq: {last_log_seq}");
    Ok(())
}

fn run_workflow(repo: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let repo = repo.canonicalize()?;
    let (mut store, snapshot_path) = open_store_for_repo(&repo)?;

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

    let seq = store.append(PersistedShellEvent::WorkflowRunStarted {
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
    save_snapshots(&store, &snapshot_path, seq)?;

    execute_workflow(
        &repo,
        &mut store,
        &snapshot_path,
        &mut state,
        run_id,
        0,
        1,
        policy_tier,
        None,
    )
}

fn resume_workflow(repo: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let repo = repo.canonicalize()?;
    let (mut store, snapshot_path) = open_store_for_repo(&repo)?;
    let records = store.load()?;
    let Some(run) = load_latest_run(&store, &snapshot_path)? else {
        println!("nothing to resume");
        return Ok(());
    };

    match run.status {
        PersistedWorkflowStatus::Completed | PersistedWorkflowStatus::Failed => {
            println!("nothing to resume");
            return Ok(());
        }
        PersistedWorkflowStatus::AwaitingApproval => {
            let Some(request_id) = run.pending_request_id.clone() else {
                return Err("malformed resume state: awaiting approval without pending request"
                    .into());
            };
            let Some(tool_id) = run.pending_tool_id.clone() else {
                return Err(
                    "malformed resume state: awaiting approval without pending tool".into(),
                );
            };
            let Some(pending_invocation_id) = run.pending_invocation_id else {
                return Err(
                    "malformed resume state: awaiting approval without pending invocation".into(),
                );
            };

            let tool_id_enum = parse_tool_id(tool_id.as_str())?;
            if !prompt_approval(tool_id_enum)? {
                let seq = store.append(PersistedShellEvent::ApprovalResolved {
                    request_id,
                    run_id: run.run_id,
                    decision: "denied".to_string(),
                })?;
                save_snapshots(&store, &snapshot_path, seq)?;
                let seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
                    run_id: run.run_id,
                    status: PersistedWorkflowStatus::Blocked,
                    step_index: run.step_index,
                    reason: Some("approval denied".to_string()),
                })?;
                save_snapshots(&store, &snapshot_path, seq)?;
                println!("workflow blocked: approval denied");
                return Ok(());
            }

            let seq = store.append(PersistedShellEvent::ApprovalResolved {
                request_id,
                run_id: run.run_id,
                decision: "approved".to_string(),
            })?;
            save_snapshots(&store, &snapshot_path, seq)?;

            let seq = store.append(PersistedShellEvent::WorkflowResumed { run_id: run.run_id })?;
            save_snapshots(&store, &snapshot_path, seq)?;

            let mut state = ShellState::new(repo_name(&repo), Personality::Pragmatic);
            let policy_tier = policy_tier_for_run(run.run_id, &records);
            return execute_workflow(
                &repo,
                &mut store,
                &snapshot_path,
                &mut state,
                run.run_id,
                run.step_index,
                run.next_invocation_id,
                policy_tier,
                Some(pending_invocation_id),
            );
        }
        PersistedWorkflowStatus::Running
        | PersistedWorkflowStatus::Blocked => {
            if matches!(run.status, PersistedWorkflowStatus::Blocked)
                && run.blocked_reason.as_deref() != Some("interrupted")
            {
                println!("nothing to resume");
                return Ok(());
            }

            let seq = store.append(PersistedShellEvent::WorkflowResumed { run_id: run.run_id })?;
            save_snapshots(&store, &snapshot_path, seq)?;

            let mut state = ShellState::new(repo_name(&repo), Personality::Pragmatic);
            let policy_tier = policy_tier_for_run(run.run_id, &records);
            execute_workflow(
                &repo,
                &mut store,
                &snapshot_path,
                &mut state,
                run.run_id,
                run.step_index,
                run.next_invocation_id,
                policy_tier,
                None,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_workflow(
    repo: &Path,
    store: &mut ShellEventStore,
    snapshot_path: &Path,
    state: &mut ShellState,
    run_id: u64,
    start_step: usize,
    start_next_invocation: u64,
    policy_tier: PolicyTier,
    first_invocation_override: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let template = workflow_template(WorkflowTemplateId::ScanPlanDiffVerify);
    let executor = SimulatedToolExecutor;
    let context = ToolExecutionContext { cwd: repo };
    let mut next_invocation_id = start_next_invocation.max(1);
    let mut first_override = first_invocation_override;

    for (step_index, step) in template.steps.iter().enumerate().skip(start_step) {
        let spec = ToolRegistry::get(step.tool_id);
        let sim = simulate_tool(policy_tier, step.tool_id);
        reduce(
            state,
            ShellAction::Runtime(RuntimeAction::AssessPolicyGate {
                run_id,
                action: ApprovalAction::Execute,
                risk: spec.risk_class,
                reason: sim.reason.to_string(),
            }),
        );

        if sim.blocked {
            let seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
                run_id,
                status: PersistedWorkflowStatus::Blocked,
                step_index,
                reason: Some(sim.reason.to_string()),
            })?;
            save_snapshots(store, snapshot_path, seq)?;
            println!("workflow blocked at {}: {}", step.step_id, sim.reason);
            return Ok(());
        }

        let invocation_id = if step_index == start_step {
            first_override.take().unwrap_or(next_invocation_id)
        } else {
            next_invocation_id
        };

        if sim.requirement.label() == "require-approval" && first_override.is_none() {
            let request_id = format!("req-{run_id}-{invocation_id}");
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
                state,
                ShellAction::Runtime(RuntimeAction::RequestApproval(request)),
            );
            store.append(PersistedShellEvent::ApprovalRequested {
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
                    state,
                    ShellAction::Runtime(RuntimeAction::ResolveApproval(decision)),
                );
                let seq = store.append(PersistedShellEvent::ApprovalResolved {
                    request_id,
                    run_id,
                    decision: "denied".to_string(),
                })?;
                save_snapshots(store, snapshot_path, seq)?;
                let seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
                    run_id,
                    status: PersistedWorkflowStatus::Blocked,
                    step_index,
                    reason: Some("approval denied".to_string()),
                })?;
                save_snapshots(store, snapshot_path, seq)?;
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
                state,
                ShellAction::Runtime(RuntimeAction::ResolveApproval(decision)),
            );
            let seq = store.append(PersistedShellEvent::ApprovalResolved {
                request_id,
                run_id,
                decision: "approved".to_string(),
            })?;
            save_snapshots(store, snapshot_path, seq)?;
        }

        let invocation = ToolInvocation {
            run_id,
            invocation_id,
            tool_id: step.tool_id.as_str().to_string(),
            requested_tier: policy_tier.label().to_string(),
        };
        store.append(PersistedShellEvent::ToolInvocationIssued {
            run_id,
            invocation_id,
            tool_id: step.tool_id.as_str().to_string(),
        })?;

        let outcome = executor.execute(invocation, &context);
        next_invocation_id = next_invocation_id.max(invocation_id.saturating_add(1));

        apply_execution_outcome(
            state,
            run_id,
            invocation_id,
            payload_to_result(step.tool_id, outcome.payload),
            &outcome.result.logs,
        );

        store.append(PersistedShellEvent::ToolResultRecorded {
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
        let seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
            run_id,
            status: workflow_status,
            step_index: step_index.saturating_add(1),
            reason: if outcome.result.status == ToolInvocationStatus::Succeeded {
                None
            } else {
                Some("tool execution did not succeed".to_string())
            },
        })?;
        save_snapshots(store, snapshot_path, seq)?;

        if outcome.result.status != ToolInvocationStatus::Succeeded {
            println!(
                "workflow ended at {} with status {}",
                step.step_id,
                status_label(outcome.result.status)
            );
            return Ok(());
        }
    }

    let seq = store.append(PersistedShellEvent::WorkflowStatusChanged {
        run_id,
        status: PersistedWorkflowStatus::Completed,
        step_index: template.steps.len(),
        reason: None,
    })?;
    save_snapshots(store, snapshot_path, seq)?;

    println!("workflow {run_id} completed");
    println!("events: {}", store_path(repo).join("workflow-events.jsonl").display());
    println!("snapshot: {}", store_path(repo).join("snapshot.json").display());
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
        (ToolId::GeneratePlan, ToolExecutionPayload::Plan { steps }) => {
            StepResult::Plan(PlanArtifact {
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
            })
        }
        (ToolId::ComputeDiff, ToolExecutionPayload::Diff { unified_diff }) => {
            StepResult::Diff(DiffArtifact {
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
            })
        }
        (ToolId::Verify, ToolExecutionPayload::Verify { checks, passing }) => {
            StepResult::Verify(VerifyArtifact {
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
            })
        }
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
            reduce(
                state,
                ShellAction::Runtime(RuntimeAction::SetSystemArtifact(artifact)),
            );
        }
        StepResult::Plan(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(
                state,
                ShellAction::Runtime(RuntimeAction::SetPlanArtifact(artifact)),
            );
        }
        StepResult::Diff(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(
                state,
                ShellAction::Runtime(RuntimeAction::SetDiffArtifact(artifact)),
            );
        }
        StepResult::Verify(mut artifact) => {
            artifact.run_id = run_id;
            artifact.artifact_id = artifact_id;
            reduce(
                state,
                ShellAction::Runtime(RuntimeAction::SetVerifyArtifact(artifact)),
            );
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

fn open_store_for_repo(repo: &Path) -> Result<(ShellEventStore, PathBuf), Box<dyn std::error::Error>> {
    let dao_dir = store_path(repo);
    fs::create_dir_all(&dao_dir)?;
    let events_path = dao_dir.join("workflow-events.jsonl");
    let snapshot_path = dao_dir.join("snapshot.json");
    let store = ShellEventStore::open(events_path)?;
    Ok((store, snapshot_path))
}

fn store_path(repo: &Path) -> PathBuf {
    repo.join(".dao")
}

fn load_latest_run(
    store: &ShellEventStore,
    snapshot_path: &Path,
) -> Result<Option<ReplayedWorkflowRun>, Box<dyn std::error::Error>> {
    let snapshot = load_snapshot_preferred(store, snapshot_path)?;
    if let Some(snapshot) = snapshot {
        let tail = store.load_since(snapshot.seq)?;
        return Ok(replay_workflow_from(snapshot.workflow, &tail));
    }
    let records = store.load()?;
    Ok(replay_latest_workflow(&records))
}

fn load_snapshot_preferred(
    store: &ShellEventStore,
    snapshot_path: &Path,
) -> Result<Option<PersistedShellSnapshot>, Box<dyn std::error::Error>> {
    if snapshot_path.exists() {
        let bytes = fs::read(snapshot_path)?;
        let parsed = serde_json::from_slice::<PersistedShellSnapshot>(&bytes)?;
        return Ok(Some(parsed));
    }
    Ok(store.load_snapshot()?)
}

fn save_snapshots(
    store: &ShellEventStore,
    snapshot_path: &Path,
    seq: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let workflow = replay_latest_workflow(&store.load()?).map(|mut run| {
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
    store.save_snapshot(&snapshot)?;
    fs::write(snapshot_path, serde_json::to_vec_pretty(&snapshot)?)?;
    Ok(())
}

fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .to_string()
}

fn parse_tool_id(raw: &str) -> Result<ToolId, Box<dyn std::error::Error>> {
    match raw {
        "scan_repo" => Ok(ToolId::ScanRepo),
        "generate_plan" => Ok(ToolId::GeneratePlan),
        "compute_diff" => Ok(ToolId::ComputeDiff),
        "verify" => Ok(ToolId::Verify),
        _ => Err(format!("unknown tool id in replay state: {raw}").into()),
    }
}

fn policy_tier_for_run(run_id: u64, records: &[PersistedShellEventRecord]) -> PolicyTier {
    for record in records.iter().rev() {
        if let PersistedShellEvent::WorkflowRunStarted {
            run_id: event_run_id,
            policy_tier,
            ..
        } = &record.event
            && *event_run_id == run_id
        {
            return match policy_tier.as_str() {
                "strict" => PolicyTier::Strict,
                "permissive" => PolicyTier::Permissive,
                _ => PolicyTier::Balanced,
            };
        }
    }
    PolicyTier::Balanced
}

fn artifact_flags(step_index: usize) -> (bool, bool, bool, bool) {
    (
        step_index >= 1,
        step_index >= 2,
        step_index >= 3,
        step_index >= 4,
    )
}

fn persisted_status_label(status: PersistedWorkflowStatus) -> &'static str {
    match status {
        PersistedWorkflowStatus::Running => "running",
        PersistedWorkflowStatus::AwaitingApproval => "awaiting_approval",
        PersistedWorkflowStatus::Blocked => "blocked",
        PersistedWorkflowStatus::Completed => "completed",
        PersistedWorkflowStatus::Failed => "failed",
    }
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
    println!("  dao replay --last --repo PATH");
    println!("  dao resume --repo PATH");
    println!("  dao --help");
    println!("  dao --version");
}
