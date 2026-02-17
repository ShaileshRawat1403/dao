use std::sync::Arc;

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn projection_only_system_artifact_is_idle() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(1, 1, "system")),
    );
    assert_eq!(state.journey_status.state, JourneyState::Idle);
}

#[test]
fn projection_planning_flag_without_plan_artifact() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetRuntimeFlag {
            flag: RuntimeFlag::Planning,
            active: true,
            run_id: 9,
        },
    );
    assert_eq!(state.journey_status.state, JourneyState::Planning);
}

#[test]
fn projection_diffing_flag_without_diff_artifact() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetRuntimeFlag {
            flag: RuntimeFlag::Diffing,
            active: true,
            run_id: 7,
        },
    );
    assert_eq!(state.journey_status.state, JourneyState::Diffing);
    assert_eq!(state.journey_status.step, JourneyStep::Preview);
}

#[test]
fn projection_diff_artifact_without_verify_is_review_ready() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            3,
            1,
            vec![diff_file("a.rs", DiffFileStatus::Modified)],
        )),
    );
    assert_eq!(state.journey_status.state, JourneyState::ReviewReady);
}

#[test]
fn projection_verify_passing_beats_diff_presence() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            8,
            1,
            vec![diff_file("a.rs", DiffFileStatus::Modified)],
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetVerifyArtifact(verify_artifact(8, 2, VerifyOverall::Passing)),
    );
    assert_eq!(state.journey_status.state, JourneyState::Completed);
}

#[test]
fn projection_awaiting_approval_dominates_diff() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            4,
            1,
            vec![diff_file("a.rs", DiffFileStatus::Modified)],
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetRuntimeFlag {
            flag: RuntimeFlag::AwaitingApproval,
            active: true,
            run_id: 4,
        },
    );
    assert_eq!(state.journey_status.state, JourneyState::AwaitingApproval);
}

#[test]
fn artifact_error_does_not_force_failed_journey() {
    let mut state = state();
    let mut verify = verify_artifact(5, 1, VerifyOverall::Failing);
    verify.error = Some(ArtifactError {
        kind: ErrorKind::Runtime,
        message: Arc::<str>::from("check failed"),
    });

    run_runtime(&mut state, RuntimeAction::SetVerifyArtifact(verify));

    assert_ne!(state.journey_status.state, JourneyState::Failed);
    assert!(state
        .artifacts
        .verify
        .as_ref()
        .and_then(|artifact| artifact.error.as_ref())
        .is_some());
}
