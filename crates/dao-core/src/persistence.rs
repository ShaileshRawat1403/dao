use std::fs::File;
use std::fs::OpenOptions;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedExecutionMode {
    Simulated,
    Runtime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedWorkflowStatus {
    Running,
    AwaitingApproval,
    Blocked,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistedShellEvent {
    WorkflowRunStarted {
        run_id: u64,
        template_id: String,
        execution_mode: PersistedExecutionMode,
        policy_tier: String,
        persona_policy: PersistedPersonaPolicy,
    },
    WorkflowStatusChanged {
        run_id: u64,
        status: PersistedWorkflowStatus,
        step_index: usize,
        reason: Option<String>,
    },
    ToolInvocationIssued {
        run_id: u64,
        invocation_id: u64,
        tool_id: String,
    },
    ToolResultRecorded {
        run_id: u64,
        invocation_id: u64,
        tool_id: String,
        status: String,
    },
    ApprovalRequested {
        request_id: String,
        run_id: u64,
        invocation_id: u64,
        tool_id: String,
        risk: String,
        preview: String,
    },
    ApprovalResolved {
        request_id: String,
        run_id: u64,
        decision: String,
    },
    WorkflowResumed {
        run_id: u64,
    },
    PolicyChanged {
        tier: String,
        source: String,
    },
    PersonaPolicyChanged {
        persona: String,
        policy: PersistedPersonaPolicy,
        source: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedPersonaPolicy {
    pub tier_ceiling: String,
    pub explanation_depth: String,
    pub output_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedShellEventRecord {
    pub seq: u64,
    pub ts_ms: i64,
    #[serde(flatten)]
    pub event: PersistedShellEvent,
}

#[derive(Debug)]
pub struct ShellEventStore {
    path: PathBuf,
    snapshot_path: PathBuf,
    next_seq: u64,
}

impl ShellEventStore {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existing = load_records(path.as_path())?;
        let next_seq = existing
            .iter()
            .map(|record| record.seq)
            .max()
            .map_or(1, |seq| seq.saturating_add(1));
        let snapshot_path = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("workflow-snapshot.json");
        Ok(Self {
            path,
            snapshot_path,
            next_seq,
        })
    }

    pub fn append(&mut self, event: PersistedShellEvent) -> std::io::Result<u64> {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let record = PersistedShellEventRecord {
            seq,
            ts_ms: chrono::Utc::now().timestamp_millis(),
            event,
        };
        let line = serde_json::to_string(&record)
            .map_err(|err| std::io::Error::other(format!("serialize: {err}")))?;
        append_line(self.path.as_path(), line.as_str())?;
        Ok(seq)
    }

    pub fn load(&self) -> std::io::Result<Vec<PersistedShellEventRecord>> {
        load_records(self.path.as_path())
    }

    pub fn load_since(
        &self,
        seq_exclusive: u64,
    ) -> std::io::Result<Vec<PersistedShellEventRecord>> {
        let records = self.load()?;
        Ok(records
            .into_iter()
            .filter(|record| record.seq > seq_exclusive)
            .collect())
    }

    pub fn save_snapshot(&self, snapshot: &PersistedShellSnapshot) -> std::io::Result<()> {
        let encoded = serde_json::to_vec(snapshot)
            .map_err(|err| std::io::Error::other(format!("serialize snapshot: {err}")))?;
        std::fs::write(&self.snapshot_path, encoded)
    }

    pub fn load_snapshot(&self) -> std::io::Result<Option<PersistedShellSnapshot>> {
        if !self.snapshot_path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&self.snapshot_path)?;
        let snapshot = serde_json::from_slice::<PersistedShellSnapshot>(&bytes)
            .map_err(|err| std::io::Error::other(format!("parse snapshot: {err}")))?;
        Ok(Some(snapshot))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayedWorkflowRun {
    pub run_id: u64,
    pub template_id: String,
    pub execution_mode: PersistedExecutionMode,
    pub step_index: usize,
    pub status: PersistedWorkflowStatus,
    pub pending_request_id: Option<String>,
    pub pending_tool_id: Option<String>,
    pub pending_invocation_id: Option<u64>,
    pub next_invocation_id: u64,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedShellSnapshot {
    pub version: u8,
    pub seq: u64,
    pub workflow: Option<ReplayedWorkflowRun>,
}

pub fn replay_latest_workflow(
    records: &[PersistedShellEventRecord],
) -> Option<ReplayedWorkflowRun> {
    replay_workflow_from(None, records)
}

pub fn replay_workflow_from(
    initial: Option<ReplayedWorkflowRun>,
    records: &[PersistedShellEventRecord],
) -> Option<ReplayedWorkflowRun> {
    let mut sorted = records.to_vec();
    sorted.sort_by_key(|record| record.seq);

    let mut latest = initial;
    for record in sorted {
        match record.event {
            PersistedShellEvent::WorkflowRunStarted {
                run_id,
                template_id,
                execution_mode,
                ..
            } => {
                latest = Some(ReplayedWorkflowRun {
                    run_id,
                    template_id,
                    execution_mode,
                    step_index: 0,
                    status: PersistedWorkflowStatus::Running,
                    pending_request_id: None,
                    pending_tool_id: None,
                    pending_invocation_id: None,
                    next_invocation_id: 1,
                    blocked_reason: None,
                });
            }
            PersistedShellEvent::WorkflowStatusChanged {
                run_id,
                status,
                step_index,
                reason,
            } => {
                if let Some(run) = latest.as_mut() {
                    if run.run_id == run_id {
                        run.status = status;
                        run.step_index = step_index;
                        run.blocked_reason = reason;
                        if !matches!(status, PersistedWorkflowStatus::AwaitingApproval) {
                            run.pending_request_id = None;
                            run.pending_tool_id = None;
                            run.pending_invocation_id = None;
                        }
                    }
                }
            }
            PersistedShellEvent::ToolResultRecorded {
                run_id, status, ..
            } => {
                if let Some(run) = latest.as_mut() {
                    if run.run_id == run_id && status == "succeeded" {
                        run.step_index = run.step_index.saturating_add(1);
                        run.next_invocation_id = run.next_invocation_id.saturating_add(1);
                    }
                }
            }
            PersistedShellEvent::ApprovalRequested {
                request_id,
                run_id,
                invocation_id,
                tool_id,
                ..
            } => {
                if let Some(run) = latest.as_mut() {
                    if run.run_id == run_id {
                        run.status = PersistedWorkflowStatus::AwaitingApproval;
                        run.blocked_reason = None;
                        run.pending_request_id = Some(request_id);
                        run.pending_tool_id = Some(tool_id);
                        run.pending_invocation_id = Some(invocation_id);
                        run.next_invocation_id = invocation_id.saturating_add(1);
                    }
                }
            }
            PersistedShellEvent::ApprovalResolved {
                request_id,
                run_id,
                decision,
            } => {
                if let Some(run) = latest.as_mut() {
                    if run.run_id == run_id
                        && run.pending_request_id.as_deref() == Some(request_id.as_str())
                    {
                        if decision == "approved" {
                            run.status = PersistedWorkflowStatus::Running;
                        } else {
                            run.status = PersistedWorkflowStatus::Blocked;
                        }
                        run.blocked_reason = None;
                        run.pending_request_id = None;
                        run.pending_tool_id = None;
                        run.pending_invocation_id = None;
                    }
                }
            }
            PersistedShellEvent::WorkflowResumed { run_id } => {
                if let Some(run) = latest.as_mut() {
                    if run.run_id == run_id {
                        run.status = PersistedWorkflowStatus::Running;
                        run.blocked_reason = None;
                        run.pending_request_id = None;
                        run.pending_tool_id = None;
                        run.pending_invocation_id = None;
                    }
                }
            }
            PersistedShellEvent::ToolInvocationIssued { .. }
            | PersistedShellEvent::PolicyChanged { .. }
            | PersistedShellEvent::PersonaPolicyChanged { .. } => {}
        }
    }

    latest
}

fn load_records(path: &Path) -> std::io::Result<Vec<PersistedShellEventRecord>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<PersistedShellEventRecord>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::PersistedExecutionMode;
    use super::PersistedPersonaPolicy;
    use super::PersistedShellEvent;
    use super::PersistedShellSnapshot;
    use super::PersistedWorkflowStatus;
    use super::ShellEventStore;
    use super::replay_latest_workflow;
    use super::replay_workflow_from;
    use pretty_assertions::assert_eq;

    fn policy() -> PersistedPersonaPolicy {
        PersistedPersonaPolicy {
            tier_ceiling: "balanced".to_string(),
            explanation_depth: "detailed".to_string(),
            output_format: "impact-first".to_string(),
        }
    }

    #[test]
    fn append_records_are_monotonic() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("events.jsonl");
        let mut store = ShellEventStore::open(path).expect("open");
        let seq1 = store
            .append(PersistedShellEvent::WorkflowRunStarted {
                run_id: 1,
                template_id: "scan_plan_diff_verify".to_string(),
                execution_mode: PersistedExecutionMode::Simulated,
                policy_tier: "balanced".to_string(),
                persona_policy: policy(),
            })
            .expect("append");
        let seq2 = store
            .append(PersistedShellEvent::WorkflowStatusChanged {
                run_id: 1,
                status: PersistedWorkflowStatus::Running,
                step_index: 0,
                reason: None,
            })
            .expect("append");

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        let loaded = store.load().expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].seq, 1);
        assert_eq!(loaded[1].seq, 2);
    }

    #[test]
    fn replay_workflow_tracks_approval_lifecycle() {
        let records = vec![
            super::PersistedShellEventRecord {
                seq: 1,
                ts_ms: 0,
                event: PersistedShellEvent::WorkflowRunStarted {
                    run_id: 7,
                    template_id: "scan_plan_diff_verify".to_string(),
                    execution_mode: PersistedExecutionMode::Runtime,
                    policy_tier: "balanced".to_string(),
                    persona_policy: policy(),
                },
            },
            super::PersistedShellEventRecord {
                seq: 2,
                ts_ms: 0,
                event: PersistedShellEvent::ApprovalRequested {
                    request_id: "req-1".to_string(),
                    run_id: 7,
                    invocation_id: 3,
                    tool_id: "compute_diff".to_string(),
                    risk: "patch-only".to_string(),
                    preview: "workflow-tool compute_diff".to_string(),
                },
            },
            super::PersistedShellEventRecord {
                seq: 3,
                ts_ms: 0,
                event: PersistedShellEvent::ApprovalResolved {
                    request_id: "req-1".to_string(),
                    run_id: 7,
                    decision: "approved".to_string(),
                },
            },
        ];

        let run = replay_latest_workflow(&records).expect("replay");
        assert_eq!(run.run_id, 7);
        assert_eq!(run.status, PersistedWorkflowStatus::Running);
        assert!(run.pending_request_id.is_none());
    }

    #[test]
    fn replay_tracks_succeeded_results_into_step_index() {
        let records = vec![
            super::PersistedShellEventRecord {
                seq: 1,
                ts_ms: 0,
                event: PersistedShellEvent::WorkflowRunStarted {
                    run_id: 9,
                    template_id: "scan_plan_diff_verify".to_string(),
                    execution_mode: PersistedExecutionMode::Simulated,
                    policy_tier: "strict".to_string(),
                    persona_policy: policy(),
                },
            },
            super::PersistedShellEventRecord {
                seq: 2,
                ts_ms: 0,
                event: PersistedShellEvent::ToolResultRecorded {
                    run_id: 9,
                    invocation_id: 1,
                    tool_id: "scan_repo".to_string(),
                    status: "succeeded".to_string(),
                },
            },
            super::PersistedShellEventRecord {
                seq: 3,
                ts_ms: 0,
                event: PersistedShellEvent::ToolResultRecorded {
                    run_id: 9,
                    invocation_id: 2,
                    tool_id: "generate_plan".to_string(),
                    status: "succeeded".to_string(),
                },
            },
        ];

        let run = replay_latest_workflow(&records).expect("replay");
        assert_eq!(run.step_index, 2);
    }

    #[test]
    fn snapshot_round_trip_and_bounded_replay() {
        let dir = tempdir().expect("tmpdir");
        let path = dir.path().join("events.jsonl");
        let mut store = ShellEventStore::open(path).expect("open");

        let seq1 = store
            .append(PersistedShellEvent::WorkflowRunStarted {
                run_id: 10,
                template_id: "scan_plan_diff_verify".to_string(),
                execution_mode: PersistedExecutionMode::Simulated,
                policy_tier: "balanced".to_string(),
                persona_policy: policy(),
            })
            .expect("append");
        let seq2 = store
            .append(PersistedShellEvent::ToolResultRecorded {
                run_id: 10,
                invocation_id: 1,
                tool_id: "scan_repo".to_string(),
                status: "succeeded".to_string(),
            })
            .expect("append");
        let before_snapshot = replay_latest_workflow(&store.load().expect("load")).expect("run");
        store
            .save_snapshot(&PersistedShellSnapshot {
                version: 1,
                seq: seq2,
                workflow: Some(before_snapshot),
            })
            .expect("save snapshot");
        let _seq3 = store
            .append(PersistedShellEvent::ToolResultRecorded {
                run_id: 10,
                invocation_id: 2,
                tool_id: "generate_plan".to_string(),
                status: "succeeded".to_string(),
            })
            .expect("append");
        assert_eq!(seq1, 1);

        let snapshot = store
            .load_snapshot()
            .expect("load snapshot")
            .expect("snapshot present");
        let tail = store.load_since(snapshot.seq).expect("tail");
        let replayed = replay_workflow_from(snapshot.workflow, &tail).expect("replayed");
        assert_eq!(replayed.step_index, 2);
    }
}
