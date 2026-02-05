use super::*;
use crate::state::Personality;
use pretty_assertions::assert_eq;

fn run_user(state: &mut ShellState, action: UserAction) {
    let _ = reduce(state, ShellAction::User(action));
}

#[test]
fn pragmatic_persona_defaults_to_chat_first_tab_priority() {
    let state = ShellState::new("project".to_string(), Personality::Pragmatic);
    assert_eq!(state.routing.tab, ShellTab::Chat);
    assert_eq!(
        state.ordered_tabs(),
        &[
            ShellTab::Chat,
            ShellTab::Diff,
            ShellTab::Logs,
            ShellTab::Plan,
            ShellTab::System,
            ShellTab::Explain,
            ShellTab::Overview,
        ]
    );
}

#[test]
fn next_prev_tab_follow_persona_ordering() {
    let mut state = ShellState::new("project".to_string(), Personality::Pragmatic);

    run_user(&mut state, UserAction::NextTab);
    assert_eq!(state.routing.tab, ShellTab::Diff);

    run_user(&mut state, UserAction::PrevTab);
    assert_eq!(state.routing.tab, ShellTab::Chat);
}

#[test]
fn persona_updates_do_not_mutate_gate_or_pending_approval_state() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(ApprovalRequestRecord {
            request_id: "req-1".to_string(),
            run_id: 3,
            action: ApprovalAction::Execute,
            risk: ApprovalRiskClass::Execution,
            reason: "approval needed".into(),
            preview: "cmd".into(),
            created_at_ms: None,
        }),
    );
    let before = state.approval.clone();

    run_runtime(
        &mut state,
        RuntimeAction::SetPersonality(Personality::Pragmatic),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaTierCeilingOverride(Some(PolicyTier::Strict)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaExplanationDepthOverride(Some(ExplanationDepth::Standard)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaOutputFormatOverride(Some(PersonaOutputFormat::ImpactFirst)),
    );

    assert_eq!(state.approval.pending, before.pending);
    assert_eq!(state.approval.last_gate, before.last_gate);
    assert_eq!(state.approval.last_decision, before.last_decision);
    assert_eq!(state.approval.policy_tier, before.policy_tier);
}
