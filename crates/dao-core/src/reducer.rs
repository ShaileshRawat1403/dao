use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaoHostEvent {
    OpenPermissionsPopup,
    OpenApprovalsPopup,
    OpenSkillsList,
    NewSession,
    ExitShutdownFirst,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaoEffect {
    EmitHostEvent(DaoHostEvent),
    RequestFrame,
}

use super::state::ARTIFACT_SCHEMA_V1;
use super::state::ApprovalGateRequirement;
use super::state::ApprovalRiskClass;
use super::state::DiffArtifact;
use super::state::DiffFile;
use super::state::DiffFileStatus;
use super::state::DiffHunk;
use super::state::DiffLine;
use super::state::DiffLineKind;
use super::state::JourneyError;
use super::state::JourneyState;
use super::state::LogEntry;
use super::state::LogLevel;
use super::state::LogSource;
use super::state::PendingApproval;
use super::state::PersonaPolicyOverrides;
use super::state::PlanArtifact;
use super::state::PlanStep;
use super::state::PolicyGateState;
use super::state::ShellOverlay;
use super::state::ShellState;
use super::state::StepStatus;
use super::state::SystemArtifact;
use super::state::apply_persona_policy_overrides;
use super::state::artifact_is_newer;
use super::state::derive_journey;
use super::state::persona_policy_for;
use super::state::policy_requirement_for_risk;
use super::actions::ClearWhich;
use super::actions::PALETTE_ITEMS;
use super::actions::PaletteCommand;
use super::actions::RuntimeAction;
use super::actions::RuntimeFlag;
use super::actions::ShellAction;
use super::actions::UserAction;
use super::actions::filtered_palette_indices;

pub fn reduce(state: &mut ShellState, action: ShellAction) -> Vec<DaoEffect> {
    match action {
        ShellAction::User(user) => reduce_user(state, user),
        ShellAction::Runtime(runtime) => {
            reduce_runtime(state, runtime);
            Vec::new()
        }
    }
}

fn reduce_user(state: &mut ShellState, action: UserAction) -> Vec<DaoEffect> {
    match action {
        UserAction::ToggleActionPalette => {
            state.interaction.overlay = match state.interaction.overlay {
                ShellOverlay::ActionPalette { .. } => ShellOverlay::None,
                _ => ShellOverlay::ActionPalette {
                    selected: 0,
                    query: Arc::<str>::from(""),
                },
            };
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ShowOnboarding => {
            state.interaction.overlay = ShellOverlay::Onboarding { step: 0 };
            vec![DaoEffect::RequestFrame]
        }
        UserAction::NextOnboardingStep => {
            if let ShellOverlay::Onboarding { step } = &mut state.interaction.overlay {
                if *step >= 3 {
                    state.interaction.overlay = ShellOverlay::None;
                } else {
                    *step += 1;
                }
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::PrevOnboardingStep => {
            if let ShellOverlay::Onboarding { step } = &mut state.interaction.overlay {
                *step = step.saturating_sub(1);
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::CompleteOnboarding => {
            state.interaction.overlay = ShellOverlay::None;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetKeymapPreset(preset) => {
            state.customization.keymap_preset = preset;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::CycleKeymapPreset => {
            state.customization.keymap_preset = state.customization.keymap_preset.next();
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetTheme(theme) => {
            state.customization.theme = theme;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::CycleTheme => {
            state.customization.theme = state.customization.theme.next();
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ToggleJourneyPanel => {
            state.customization.show_journey = !state.customization.show_journey;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ToggleOverviewPanel => {
            state.customization.show_overview = !state.customization.show_overview;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ToggleActionBar => {
            state.customization.show_action_bar = !state.customization.show_action_bar;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ToggleAutoIntentFollow => {
            state.customization.auto_follow_intent = !state.customization.auto_follow_intent;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::CloseOverlay => {
            state.interaction.overlay = ShellOverlay::None;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::NextTab => {
            state.routing.tab = state.next_tab();
            vec![DaoEffect::RequestFrame]
        }
        UserAction::PrevTab => {
            state.routing.tab = state.prev_tab();
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SelectTab(tab) => {
            state.routing.tab = tab;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::NextJourneyStep | UserAction::PrevJourneyStep => Vec::new(),
        UserAction::OverlayMoveUp => {
            if let ShellOverlay::ActionPalette { selected, query } = &mut state.interaction.overlay
            {
                let filtered = filtered_palette_indices(query.as_ref());
                if !filtered.is_empty() {
                    if *selected == 0 {
                        *selected = filtered.len().saturating_sub(1);
                    } else {
                        *selected -= 1;
                    }
                }
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::OverlayMoveDown => {
            if let ShellOverlay::ActionPalette { selected, query } = &mut state.interaction.overlay
            {
                let filtered = filtered_palette_indices(query.as_ref());
                if !filtered.is_empty() {
                    *selected = (*selected + 1) % filtered.len();
                }
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::OverlayQueryInput(ch) => {
            if let ShellOverlay::ActionPalette { selected, query } = &mut state.interaction.overlay
            {
                let mut next = query.to_string();
                next.push(ch);
                *query = Arc::<str>::from(next);
                *selected = 0;
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::OverlayQueryBackspace => {
            if let ShellOverlay::ActionPalette { selected, query } = &mut state.interaction.overlay
            {
                let mut next = query.to_string();
                next.pop();
                *query = Arc::<str>::from(next);
                *selected = 0;
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::OverlayQueryPaste(text) => {
            if let ShellOverlay::ActionPalette { selected, query } = &mut state.interaction.overlay
            {
                let mut next = query.to_string();
                next.push_str(&text);
                *query = Arc::<str>::from(next);
                *selected = 0;
                return vec![DaoEffect::RequestFrame];
            }
            Vec::new()
        }
        UserAction::OverlaySubmit => {
            let (selected, query) = match &state.interaction.overlay {
                ShellOverlay::ActionPalette { selected, query } => (*selected, query.to_string()),
                _ => return Vec::new(),
            };

            let filtered = filtered_palette_indices(&query);
            let Some(palette_idx) = filtered.get(selected).copied() else {
                return Vec::new();
            };
            let command = PALETTE_ITEMS[palette_idx].command;
            state.interaction.overlay = ShellOverlay::None;
            let mut effects = command_to_effects(state, command);
            effects.push(DaoEffect::RequestFrame);
            effects
        }
        UserAction::SelectDiffFile { path } => {
            state.selection.selected_diff_file = Some(path);
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SelectPlanStep { id } => {
            state.selection.selected_plan_step = Some(id);
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetLogLevelFilter(filter) => {
            state.selection.log_level_filter = filter;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetLogSearch(search) => {
            state.selection.log_search = search;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ClearArtifact { which, reason } => {
            reduce_runtime(
                state,
                match which {
                    ClearWhich::System => RuntimeAction::ClearSystemArtifact(reason),
                    ClearWhich::Plan => RuntimeAction::ClearPlanArtifact(reason),
                    ClearWhich::Diff => RuntimeAction::ClearDiffArtifact(reason),
                    ClearWhich::Verify => RuntimeAction::ClearVerifyArtifact(reason),
                    ClearWhich::Logs => RuntimeAction::ClearLogs(reason),
                },
            );
            vec![DaoEffect::RequestFrame]
        }
    }
}

fn command_to_effects(state: &mut ShellState, command: PaletteCommand) -> Vec<DaoEffect> {
    match command {
        PaletteCommand::ContinueInChat => Vec::new(),
        PaletteCommand::ShowOnboarding => {
            state.interaction.overlay = ShellOverlay::Onboarding { step: 0 };
            Vec::new()
        }
        PaletteCommand::SetKeymapPreset(preset) => {
            state.customization.keymap_preset = preset;
            Vec::new()
        }
        PaletteCommand::SetTheme(theme) => {
            state.customization.theme = theme;
            Vec::new()
        }
        PaletteCommand::CycleTheme => {
            state.customization.theme = state.customization.theme.next();
            Vec::new()
        }
        PaletteCommand::ToggleJourneyPanel => {
            state.customization.show_journey = !state.customization.show_journey;
            Vec::new()
        }
        PaletteCommand::ToggleOverviewPanel => {
            state.customization.show_overview = !state.customization.show_overview;
            Vec::new()
        }
        PaletteCommand::ToggleActionBar => {
            state.customization.show_action_bar = !state.customization.show_action_bar;
            Vec::new()
        }
        PaletteCommand::ToggleAutoIntentFollow => {
            state.customization.auto_follow_intent = !state.customization.auto_follow_intent;
            Vec::new()
        }
        PaletteCommand::OpenPermissions => {
            vec![DaoEffect::EmitHostEvent(DaoHostEvent::OpenPermissionsPopup)]
        }
        PaletteCommand::OpenApprovals => {
            vec![DaoEffect::EmitHostEvent(DaoHostEvent::OpenApprovalsPopup)]
        }
        PaletteCommand::OpenSkills => {
            vec![DaoEffect::EmitHostEvent(DaoHostEvent::OpenSkillsList)]
        }
        PaletteCommand::StartNewSession => {
            vec![DaoEffect::EmitHostEvent(DaoHostEvent::NewSession)]
        }
        PaletteCommand::Quit => vec![DaoEffect::EmitHostEvent(DaoHostEvent::ExitShutdownFirst)],
    }
}

fn tab_for_journey(state: JourneyState) -> super::state::ShellTab {
    match state {
        JourneyState::Idle => super::state::ShellTab::Chat,
        JourneyState::Scanning => super::state::ShellTab::System,
        JourneyState::Planning => super::state::ShellTab::Plan,
        JourneyState::Diffing | JourneyState::ReviewReady => super::state::ShellTab::Diff,
        JourneyState::AwaitingApproval | JourneyState::Verifying | JourneyState::Failed => {
            super::state::ShellTab::Logs
        }
        JourneyState::Completed => super::state::ShellTab::Explain,
    }
}

fn reduce_runtime(state: &mut ShellState, action: RuntimeAction) {
    let mut dirty = false;

    match action {
        RuntimeAction::SetProjectName(name) => {
            state.header.project_name = name.into();
        }
        RuntimeAction::SetThreadId(thread_id) => {
            state.thread_id = thread_id;
        }
        RuntimeAction::SetCwd(cwd) => {
            state.cwd = cwd;
        }
        RuntimeAction::SetSafetyMode(mode) => {
            state.header.safety_mode = mode;
        }
        RuntimeAction::SetScanStatus(status) => {
            state.header.scan = status;
        }
        RuntimeAction::SetApplyStatus(status) => {
            state.header.apply = status;
        }
        RuntimeAction::SetVerifyStatus(status) => {
            state.header.verify = status;
        }
        RuntimeAction::SetRiskLevel(level) => {
            state.header.risk = level;
        }
        RuntimeAction::SetUsage(snapshot) => {
            state.usage = snapshot;
        }
        RuntimeAction::SetKeymapPreset(preset) => {
            state.customization.keymap_preset = preset;
        }
        RuntimeAction::SetPersonality(personality) => {
            state.sm.personality = personality;
            state.sm.persona_policy_defaults = persona_policy_for(personality);
            refresh_persona_policy(state);
        }
        RuntimeAction::SetPersonaTierCeilingOverride(tier_ceiling) => {
            state.sm.persona_policy_overrides.tier_ceiling = tier_ceiling;
            refresh_persona_policy(state);
        }
        RuntimeAction::SetPersonaExplanationDepthOverride(explanation_depth) => {
            state.sm.persona_policy_overrides.explanation_depth = explanation_depth;
            refresh_persona_policy(state);
        }
        RuntimeAction::SetPersonaOutputFormatOverride(output_format) => {
            state.sm.persona_policy_overrides.output_format = output_format;
            refresh_persona_policy(state);
        }
        RuntimeAction::ClearPersonaPolicyOverrides => {
            state.sm.persona_policy_overrides = PersonaPolicyOverrides::default();
            refresh_persona_policy(state);
        }
        RuntimeAction::SetSkillsEnabledCount(count) => {
            state.sm.skills_enabled_count = count;
        }
        RuntimeAction::SetCollaborationModeLabel(label) => {
            state.sm.collaboration_mode_label = label.into();
        }
        RuntimeAction::SetModelSlug(model) => {
            state.sm.model_slug = model.map(Arc::<str>::from);
        }
        RuntimeAction::SetReasoningEffort(effort) => {
            state.sm.reasoning_effort = effort;
        }
        RuntimeAction::SetTab(tab) => {
            maybe_follow_tab(state, tab);
        }
        RuntimeAction::SetJourney(_) => {}
        RuntimeAction::SetJourneyState(next) => {
            dirty = true;
            match next {
                JourneyState::Idle => {
                    state.runtime_flags.clear_all();
                    state.journey_status.error = None;
                    state.approval.pending = None;
                }
                JourneyState::Scanning => {
                    let run_id = state.runtime_flags.next_run_id;
                    state.runtime_flags.next_run_id += 1;
                    state.runtime_flags.clear_all();
                    state.runtime_flags.scanning.active = true;
                    state.runtime_flags.scanning.run_id = run_id;
                }
                JourneyState::Planning => {
                    let run_id = state.runtime_flags.next_run_id;
                    state.runtime_flags.next_run_id += 1;
                    state.runtime_flags.clear_all();
                    state.runtime_flags.planning.active = true;
                    state.runtime_flags.planning.run_id = run_id;
                }
                JourneyState::Diffing => {
                    let run_id = state.current_run_id().max(1);
                    state.runtime_flags.clear_all();
                    state.runtime_flags.diffing.active = true;
                    state.runtime_flags.diffing.run_id = run_id;
                }
                JourneyState::ReviewReady => {
                    state.runtime_flags.clear_all();
                }
                JourneyState::AwaitingApproval => {
                    let run_id = state.current_run_id().max(1);
                    state.runtime_flags.clear_all();
                    state.runtime_flags.awaiting_approval.active = true;
                    state.runtime_flags.awaiting_approval.run_id = run_id;
                }
                JourneyState::Verifying => {
                    let run_id = state.current_run_id().max(1);
                    state.runtime_flags.clear_all();
                    state.runtime_flags.verifying.active = true;
                    state.runtime_flags.verifying.run_id = run_id;
                }
                JourneyState::Completed => {
                    state.runtime_flags.clear_all();
                }
                JourneyState::Failed => {
                    state.runtime_flags.clear_all();
                }
            }
        }
        RuntimeAction::SetJourneyError { kind, message } => {
            dirty = true;
            let run_id = state.current_run_id();
            state.journey_status.error =
                Some(JourneyError::new(kind, Arc::<str>::from(message), run_id));
        }
        RuntimeAction::ClearJourneyError => {
            dirty = true;
            state.journey_status.error = None;
        }
        RuntimeAction::SetSystemArtifact(artifact) => {
            let current = state
                .artifacts
                .system
                .as_ref()
                .map(|a| (a.run_id, a.artifact_id));
            if artifact_is_newer(artifact.run_id, artifact.artifact_id, current) {
                state.artifacts.system = Some(artifact);
                if matches!(state.routing.tab, super::state::ShellTab::Overview)
                    && state.customization.auto_follow_intent
                {
                    state.routing.tab = super::state::ShellTab::System;
                }
                dirty = true;
            }
        }
        RuntimeAction::SetPlanArtifact(artifact) => {
            let current = state
                .artifacts
                .plan
                .as_ref()
                .map(|a| (a.run_id, a.artifact_id));
            if artifact_is_newer(artifact.run_id, artifact.artifact_id, current) {
                state.artifacts.plan = Some(artifact);
                reconcile_selected_plan_step(state);
                if matches!(
                    state.routing.tab,
                    super::state::ShellTab::Overview | super::state::ShellTab::System
                ) && state.customization.auto_follow_intent
                {
                    state.routing.tab = super::state::ShellTab::Plan;
                }
                dirty = true;
            }
        }
        RuntimeAction::SetDiffArtifact(artifact) => {
            let current = state
                .artifacts
                .diff
                .as_ref()
                .map(|a| (a.run_id, a.artifact_id));
            if artifact_is_newer(artifact.run_id, artifact.artifact_id, current) {
                state.artifacts.diff = Some(artifact);
                reconcile_selected_diff_file(state);
                maybe_follow_tab(state, super::state::ShellTab::Diff);
                dirty = true;
            }
        }
        RuntimeAction::SetVerifyArtifact(artifact) => {
            let current = state
                .artifacts
                .verify
                .as_ref()
                .map(|a| (a.run_id, a.artifact_id));
            if artifact_is_newer(artifact.run_id, artifact.artifact_id, current) {
                state.artifacts.verify = Some(artifact);
                dirty = true;
            }
        }
        RuntimeAction::ClearSystemArtifact(_) => {
            state.artifacts.system = None;
            dirty = true;
        }
        RuntimeAction::ClearPlanArtifact(_) => {
            state.artifacts.plan = None;
            state.selection.selected_plan_step = None;
            dirty = true;
        }
        RuntimeAction::ClearDiffArtifact(_) => {
            state.artifacts.diff = None;
            state.selection.selected_diff_file = None;
            dirty = true;
        }
        RuntimeAction::ClearVerifyArtifact(_) => {
            state.artifacts.verify = None;
            dirty = true;
        }
        RuntimeAction::SetRuntimeFlag {
            flag,
            active,
            run_id,
        } => {
            dirty = true;
            let target = match flag {
                RuntimeFlag::Scanning => &mut state.runtime_flags.scanning,
                RuntimeFlag::Planning => &mut state.runtime_flags.planning,
                RuntimeFlag::Diffing => &mut state.runtime_flags.diffing,
                RuntimeFlag::AwaitingApproval => &mut state.runtime_flags.awaiting_approval,
                RuntimeFlag::Verifying => &mut state.runtime_flags.verifying,
            };
            target.active = active;
            target.run_id = run_id;
            if run_id >= state.runtime_flags.next_run_id {
                state.runtime_flags.next_run_id = run_id + 1;
            }
        }
        RuntimeAction::SetJourneyErrorState(error) => {
            dirty = true;
            state.journey_status.error = error;
        }
        RuntimeAction::SetPolicyTier(tier) => {
            state.approval.policy_tier = tier;
        }
        RuntimeAction::AssessPolicyGate {
            run_id,
            action,
            risk,
            reason,
        } => {
            let requirement = policy_requirement_for_risk(state.approval.policy_tier, risk);
            state.approval.last_gate = Some(PolicyGateState {
                run_id,
                action,
                risk,
                requirement,
                reason: Arc::<str>::from(reason),
            });
        }
        RuntimeAction::RequestApproval(mut request) => {
            let run_id = request.run_id.max(1);
            let latest_approval_run_id = state
                .approval
                .pending
                .as_ref()
                .map(|pending| pending.request.run_id)
                .into_iter()
                .chain(
                    state
                        .approval
                        .last_decision
                        .as_ref()
                        .map(|decision| decision.run_id),
                )
                .max()
                .unwrap_or(0);
            if run_id >= latest_approval_run_id {
                dirty = true;
                request.run_id = run_id;
                let requirement =
                    policy_requirement_for_risk(state.approval.policy_tier, request.risk);
                state.approval.last_gate = Some(PolicyGateState {
                    run_id,
                    action: request.action,
                    risk: request.risk,
                    requirement,
                    reason: request.reason.clone(),
                });
                let sequence = state.approval.next_request_seq;
                state.approval.next_request_seq = state.approval.next_request_seq.saturating_add(1);
                state.approval.pending = Some(PendingApproval { request, sequence });
                state.runtime_flags.awaiting_approval.active = true;
                state.runtime_flags.awaiting_approval.run_id = run_id;
                state.artifacts.logs.append(LogEntry {
                    seq: 0,
                    level: LogLevel::Warn,
                    ts_ms: None,
                    source: LogSource::Shell,
                    context: Some("approval".to_string()),
                    message: format!("approval request queued for run {run_id}"),
                    run_id,
                });
            }
        }
        RuntimeAction::ResolveApproval(decision) => {
            if let Some(pending) = state.approval.pending.as_ref()
                && pending.request.request_id == decision.request_id
                && pending.request.run_id == decision.run_id
            {
                dirty = true;
                state.approval.pending = None;
                state.approval.last_decision = Some(decision.clone());
                if state.runtime_flags.awaiting_approval.run_id == decision.run_id {
                    state.runtime_flags.awaiting_approval.active = false;
                }
                state.approval.last_gate = Some(PolicyGateState {
                    run_id: decision.run_id,
                    action: decision.action,
                    risk: state
                        .approval
                        .last_gate
                        .as_ref()
                        .filter(|gate| {
                            gate.run_id == decision.run_id && gate.action == decision.action
                        })
                        .map_or(ApprovalRiskClass::Execution, |gate| gate.risk),
                    requirement: ApprovalGateRequirement::Allow,
                    reason: Arc::<str>::from(format!(
                        "request {} {}",
                        decision.request_id,
                        decision.decision.label()
                    )),
                });
                state.artifacts.logs.append(LogEntry {
                    seq: 0,
                    level: LogLevel::Info,
                    ts_ms: Some(decision.timestamp_ms),
                    source: LogSource::Shell,
                    context: Some("approval".to_string()),
                    message: format!(
                        "approval {} for request {}",
                        decision.decision.label(),
                        decision.request_id
                    ),
                    run_id: decision.run_id,
                });
            }
        }
        RuntimeAction::ClearApprovalState(_) => {
            dirty = true;
            state.approval.pending = None;
            state.approval.last_decision = None;
            state.approval.last_gate = None;
            state.runtime_flags.awaiting_approval.active = false;
        }
        RuntimeAction::AppendStructuredLog(entry) => {
            state.artifacts.logs.append(entry);
        }
        RuntimeAction::ClearLogs(_) => {
            state.artifacts.logs.clear();
        }
        RuntimeAction::SetOverview(value) => {
            state.artifacts.logs.append(LogEntry {
                seq: 0,
                level: LogLevel::Debug,
                ts_ms: None,
                source: LogSource::Shell,
                context: Some("overview".to_string()),
                message: value,
                run_id: state.current_run_id(),
            });
        }
        RuntimeAction::SetSystem(value) => {
            let run_id = state.current_run_id().max(1);
            let artifact_id = next_system_artifact_id(state);
            let artifact = SystemArtifact {
                schema_version: ARTIFACT_SCHEMA_V1,
                run_id,
                artifact_id,
                repo_root: state
                    .cwd
                    .as_ref()
                    .map_or_else(String::new, |cwd| cwd.display().to_string()),
                detected_stack: Vec::new(),
                entrypoints: Vec::new(),
                risk_flags: Vec::new(),
                summary: value,
                error: None,
            };
            reduce_runtime(state, RuntimeAction::SetSystemArtifact(artifact));
        }
        RuntimeAction::SetPlan(value) => {
            let run_id = state.current_run_id().max(1);
            let artifact_id = next_plan_artifact_id(state);
            let steps = value
                .lines()
                .enumerate()
                .filter_map(|(idx, line)| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(PlanStep {
                            id: format!("step-{}", idx + 1),
                            label: trimmed.to_string(),
                            status: StepStatus::Pending,
                        })
                    }
                })
                .collect();
            let artifact = PlanArtifact {
                schema_version: ARTIFACT_SCHEMA_V1,
                run_id,
                artifact_id,
                title: "Plan".to_string(),
                steps,
                assumptions: Vec::new(),
                error: None,
            };
            reduce_runtime(state, RuntimeAction::SetPlanArtifact(artifact));
        }
        RuntimeAction::SetDiff(value) => {
            let run_id = state.current_run_id().max(1);
            let artifact_id = next_diff_artifact_id(state);
            let files = legacy_diff_files_from_text(value.as_str());
            let artifact = DiffArtifact {
                schema_version: ARTIFACT_SCHEMA_V1,
                run_id,
                artifact_id,
                files,
                summary: "Patch preview".to_string(),
                error: None,
            };
            reduce_runtime(state, RuntimeAction::SetDiffArtifact(artifact));
        }
        RuntimeAction::SetExplain(value) => {
            state.artifacts.logs.append(LogEntry {
                seq: 0,
                level: LogLevel::Info,
                ts_ms: None,
                source: LogSource::Shell,
                context: Some("explain".to_string()),
                message: value,
                run_id: state.current_run_id(),
            });
        }
        RuntimeAction::AppendLog(value) => {
            state.artifacts.logs.append(LogEntry {
                seq: 0,
                level: LogLevel::Info,
                ts_ms: None,
                source: LogSource::Runtime,
                context: None,
                message: value,
                run_id: state.current_run_id(),
            });
        }
    }

    if dirty {
        recompute_journey(state);
    }
}

fn recompute_journey(state: &mut ShellState) {
    let projection = derive_journey(
        &state.artifacts,
        &state.runtime_flags,
        &state.approval,
        state.journey_status.error.as_ref(),
    );
    state.journey_status.state = projection.state;
    state.journey_status.step = projection.step;
    state.journey_status.active_run_id = projection.active_run_id;
    state.routing.journey = projection.step;
    if state.customization.auto_follow_intent {
        state.routing.tab = tab_for_journey(projection.state);
    }
}

fn maybe_follow_tab(state: &mut ShellState, tab: super::state::ShellTab) {
    if state.customization.auto_follow_intent {
        state.routing.tab = tab;
    }
}

fn refresh_persona_policy(state: &mut ShellState) {
    state.sm.persona_policy = apply_persona_policy_overrides(
        state.sm.persona_policy_defaults,
        state.sm.persona_policy_overrides,
    );
}

fn next_system_artifact_id(state: &ShellState) -> u64 {
    state
        .artifacts
        .system
        .as_ref()
        .map_or(1, |a| a.artifact_id.saturating_add(1))
}

fn next_plan_artifact_id(state: &ShellState) -> u64 {
    state
        .artifacts
        .plan
        .as_ref()
        .map_or(1, |a| a.artifact_id.saturating_add(1))
}

fn next_diff_artifact_id(state: &ShellState) -> u64 {
    state
        .artifacts
        .diff
        .as_ref()
        .map_or(1, |a| a.artifact_id.saturating_add(1))
}

fn reconcile_selected_diff_file(state: &mut ShellState) {
    let Some(diff) = state.artifacts.diff.as_ref() else {
        state.selection.selected_diff_file = None;
        return;
    };

    if let Some(current) = state.selection.selected_diff_file.as_deref()
        && diff.files.iter().any(|file| file.path == current)
    {
        return;
    }

    if let Some(file) = diff
        .files
        .iter()
        .find(|file| matches!(file.status, DiffFileStatus::Modified))
    {
        state.selection.selected_diff_file = Some(file.path.clone());
        return;
    }

    state.selection.selected_diff_file = diff.files.first().map(|file| file.path.clone());
}

fn reconcile_selected_plan_step(state: &mut ShellState) {
    let Some(plan) = state.artifacts.plan.as_ref() else {
        state.selection.selected_plan_step = None;
        return;
    };

    if let Some(current) = state.selection.selected_plan_step.as_deref()
        && plan.steps.iter().any(|step| step.id == current)
    {
        return;
    }

    if let Some(step) = plan
        .steps
        .iter()
        .find(|step| matches!(step.status, StepStatus::Running))
    {
        state.selection.selected_plan_step = Some(step.id.clone());
        return;
    }

    if let Some(step) = plan
        .steps
        .iter()
        .find(|step| matches!(step.status, StepStatus::Pending))
    {
        state.selection.selected_plan_step = Some(step.id.clone());
        return;
    }

    state.selection.selected_plan_step = plan.steps.first().map(|step| step.id.clone());
}

fn legacy_diff_files_from_text(text: &str) -> Vec<DiffFile> {
    let mut files = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    let finish_hunk = |file: &mut Option<DiffFile>, hunk: &mut Option<DiffHunk>| {
        if let Some(hunk_value) = hunk.take()
            && let Some(file_value) = file.as_mut()
        {
            file_value.hunks.push(hunk_value);
        }
    };

    let finish_file =
        |files: &mut Vec<DiffFile>, file: &mut Option<DiffFile>, hunk: &mut Option<DiffHunk>| {
            finish_hunk(file, hunk);
            if let Some(file_value) = file.take() {
                files.push(file_value);
            }
        };

    for line in text.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            finish_file(&mut files, &mut current_file, &mut current_hunk);
            current_file = Some(DiffFile {
                path: path.to_string(),
                status: DiffFileStatus::Modified,
                hunks: Vec::new(),
            });
            continue;
        }

        if let Some(path) = line.strip_prefix("+++ ") {
            finish_file(&mut files, &mut current_file, &mut current_hunk);
            current_file = Some(DiffFile {
                path: path.to_string(),
                status: DiffFileStatus::Modified,
                hunks: Vec::new(),
            });
            continue;
        }

        if let Some(header) = line.strip_prefix("@@") {
            finish_hunk(&mut current_file, &mut current_hunk);
            current_hunk = Some(DiffHunk {
                header: format!("@@{header}"),
                lines: Vec::new(),
            });
            continue;
        }

        let kind = if line.starts_with('+') {
            Some(DiffLineKind::Add)
        } else if line.starts_with('-') {
            Some(DiffLineKind::Remove)
        } else if !line.is_empty() {
            Some(DiffLineKind::Context)
        } else {
            None
        };

        if let Some(kind) = kind {
            if current_hunk.is_none() {
                current_hunk = Some(DiffHunk {
                    header: "@@".to_string(),
                    lines: Vec::new(),
                });
            }
            if let Some(hunk) = current_hunk.as_mut() {
                hunk.lines.push(DiffLine {
                    kind,
                    text: line.to_string(),
                });
            }
        }
    }

    finish_file(&mut files, &mut current_file, &mut current_hunk);

    if files.is_empty() {
        files.push(DiffFile {
            path: "<patch>".to_string(),
            status: DiffFileStatus::Modified,
            hunks: vec![DiffHunk {
                header: "@@".to_string(),
                lines: text
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
        });
    }

    files
}

#[cfg(test)]
mod tests;
