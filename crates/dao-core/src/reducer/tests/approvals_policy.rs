use super::*;
use crate::state::Personality;
use pretty_assertions::assert_eq;

fn approval_request(id: &str, run_id: u64, risk: ApprovalRiskClass) -> ApprovalRequestRecord {
    ApprovalRequestRecord {
        request_id: id.to_string(),
        run_id,
        action: ApprovalAction::Execute,
        risk,
        reason: "approval needed".into(),
        preview: "cmd".into(),
        created_at_ms: None,
    }
}

fn approval_decision(id: &str, run_id: u64, approved: bool) -> ApprovalDecisionRecord {
    ApprovalDecisionRecord {
        request_id: id.to_string(),
        run_id,
        action: ApprovalAction::Execute,
        decision: if approved {
            ApprovalDecisionKind::Approved
        } else {
            ApprovalDecisionKind::Denied
        },
        timestamp_ms: 0,
    }
}

#[test]
fn request_approval_rejects_older_run_after_newer_pending_exists() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request(
            "req-new",
            4,
            ApprovalRiskClass::Execution,
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request(
            "req-old",
            3,
            ApprovalRiskClass::Execution,
        )),
    );

    let pending = state
        .approval
        .pending
        .as_ref()
        .map(|pending| pending.request.request_id.as_str());
    assert_eq!(pending, Some("req-new"));
}

#[test]
fn resolve_approval_ignores_non_matching_request_id() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request("req-1", 2, ApprovalRiskClass::Execution)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::ResolveApproval(approval_decision("req-2", 2, true)),
    );

    assert_eq!(
        state
            .approval
            .pending
            .as_ref()
            .map(|pending| pending.request.request_id.as_str()),
        Some("req-1")
    );
    assert!(state.approval.last_decision.is_none());
}

#[test]
fn resolve_approval_ignores_non_matching_run_id() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request("req-1", 2, ApprovalRiskClass::Execution)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::ResolveApproval(approval_decision("req-1", 1, true)),
    );

    assert!(state.approval.pending.is_some());
    assert!(state.approval.last_decision.is_none());
}

#[test]
fn resolve_approval_clears_pending_and_records_decision() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request("req-1", 2, ApprovalRiskClass::Execution)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::ResolveApproval(approval_decision("req-1", 2, true)),
    );

    assert!(state.approval.pending.is_none());
    assert_eq!(
        state
            .approval
            .last_decision
            .as_ref()
            .map(|decision| decision.decision),
        Some(ApprovalDecisionKind::Approved)
    );
    assert!(!state.runtime_flags.awaiting_approval.active);
}

#[test]
fn policy_tier_controls_gate_requirement() {
    let mut state = state();

    run_runtime(&mut state, RuntimeAction::SetPolicyTier(PolicyTier::Strict));
    run_runtime(
        &mut state,
        RuntimeAction::AssessPolicyGate {
            run_id: 1,
            action: ApprovalAction::Execute,
            risk: ApprovalRiskClass::Destructive,
            reason: "danger".to_string(),
        },
    );
    assert_eq!(
        state
            .approval
            .last_gate
            .as_ref()
            .map(|gate| gate.requirement),
        Some(policy_requirement_for_risk(
            PolicyTier::Strict,
            ApprovalRiskClass::Destructive,
        ))
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetPolicyTier(PolicyTier::Permissive),
    );
    run_runtime(
        &mut state,
        RuntimeAction::AssessPolicyGate {
            run_id: 1,
            action: ApprovalAction::Execute,
            risk: ApprovalRiskClass::Execution,
            reason: "normal".to_string(),
        },
    );
    assert_eq!(
        state
            .approval
            .last_gate
            .as_ref()
            .map(|gate| gate.requirement),
        Some(policy_requirement_for_risk(
            PolicyTier::Permissive,
            ApprovalRiskClass::Execution,
        ))
    );
}

#[test]
fn pending_approval_sets_journey_to_awaiting_approval() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request("req-1", 5, ApprovalRiskClass::Execution)),
    );

    assert_eq!(state.journey_status.state, JourneyState::AwaitingApproval);
    assert_eq!(state.journey_status.step, JourneyStep::Approve);
    assert_eq!(state.journey_status.active_run_id, 5);
}

#[test]
fn clearing_approval_state_removes_pending_and_gate() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request("req-1", 2, ApprovalRiskClass::Execution)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::ClearApprovalState(ClearReason::UserRequest),
    );

    assert!(state.approval.pending.is_none());
    assert!(state.approval.last_decision.is_none());
    assert!(state.approval.last_gate.is_none());
}

#[test]
fn approval_actions_keep_projection_in_sync() {
    let mut state = state();

    run_runtime(
        &mut state,
        RuntimeAction::RequestApproval(approval_request("req-1", 7, ApprovalRiskClass::Execution)),
    );
    assert_projection_sync(&state);

    run_runtime(
        &mut state,
        RuntimeAction::ResolveApproval(approval_decision("req-1", 7, false)),
    );
    assert_projection_sync(&state);
}

#[test]
fn persona_update_applies_persona_policy_defaults() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonality(Personality::Pragmatic),
    );

    assert_eq!(state.sm.personality, Personality::Pragmatic);
    assert_eq!(state.sm.persona_policy.tier_ceiling, PolicyTier::Permissive);
    assert_eq!(state.sm.persona_policy.explanation_depth.label(), "brief");
    assert_eq!(
        state.sm.persona_policy.output_format.label(),
        "technical-first"
    );
    assert_eq!(
        state.sm.persona_policy.visible_tools,
        &["scan_repo", "generate_plan", "compute_diff", "verify"]
    );
}

#[test]
fn persona_policy_overrides_apply_and_persist_across_personality_changes() {
    let mut state = state();

    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaTierCeilingOverride(Some(PolicyTier::Strict)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaExplanationDepthOverride(Some(ExplanationDepth::Brief)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaOutputFormatOverride(Some(PersonaOutputFormat::TechnicalFirst)),
    );

    assert_eq!(state.sm.persona_policy.tier_ceiling, PolicyTier::Strict);
    assert_eq!(
        state.sm.persona_policy.explanation_depth,
        ExplanationDepth::Brief
    );
    assert_eq!(
        state.sm.persona_policy.output_format,
        PersonaOutputFormat::TechnicalFirst
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetPersonality(Personality::Pragmatic),
    );

    assert_eq!(
        state.sm.persona_policy_defaults.tier_ceiling,
        PolicyTier::Permissive
    );
    assert_eq!(state.sm.persona_policy.tier_ceiling, PolicyTier::Strict);
    assert_eq!(
        state.sm.persona_policy.explanation_depth,
        ExplanationDepth::Brief
    );
    assert_eq!(
        state.sm.persona_policy.output_format,
        PersonaOutputFormat::TechnicalFirst
    );
}

#[test]
fn clearing_persona_policy_overrides_restores_personality_defaults() {
    let mut state = state();

    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaTierCeilingOverride(Some(PolicyTier::Strict)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaExplanationDepthOverride(Some(ExplanationDepth::Brief)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPersonaOutputFormatOverride(Some(PersonaOutputFormat::TechnicalFirst)),
    );
    assert!(!state.sm.persona_policy_overrides.is_empty());

    run_runtime(&mut state, RuntimeAction::ClearPersonaPolicyOverrides);

    assert!(state.sm.persona_policy_overrides.is_empty());
    assert_eq!(
        state.sm.persona_policy.tier_ceiling,
        state.sm.persona_policy_defaults.tier_ceiling
    );
    assert_eq!(
        state.sm.persona_policy.explanation_depth,
        state.sm.persona_policy_defaults.explanation_depth
    );
    assert_eq!(
        state.sm.persona_policy.output_format,
        state.sm.persona_policy_defaults.output_format
    );
}
