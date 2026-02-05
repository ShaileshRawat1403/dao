use super::*;
use pretty_assertions::assert_eq;

#[test]
fn system_artifact_guard_matrix() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(10, 1, "v10")),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(11, 0, "v11")),
    );
    assert_eq!(
        state.artifacts.system.as_ref().map(|a| a.summary.as_str()),
        Some("v11")
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(9, 99, "old")),
    );
    assert_eq!(
        state.artifacts.system.as_ref().map(|a| a.summary.as_str()),
        Some("v11")
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(11, 2, "v11.2")),
    );
    assert_eq!(
        state.artifacts.system.as_ref().map(|a| a.summary.as_str()),
        Some("v11.2")
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(11, 1, "stale")),
    );
    assert_eq!(
        state.artifacts.system.as_ref().map(|a| a.summary.as_str()),
        Some("v11.2")
    );
}

#[test]
fn plan_artifact_guard_matrix() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            10,
            1,
            vec![plan_step("step-1", StepStatus::Pending)],
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            11,
            0,
            vec![plan_step("step-2", StepStatus::Pending)],
        )),
    );
    assert_eq!(state.artifacts.plan.as_ref().map(|a| a.run_id), Some(11));

    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            9,
            99,
            vec![plan_step("old", StepStatus::Pending)],
        )),
    );
    assert_eq!(state.artifacts.plan.as_ref().map(|a| a.run_id), Some(11));

    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            11,
            2,
            vec![plan_step("step-3", StepStatus::Running)],
        )),
    );
    assert_eq!(
        state.artifacts.plan.as_ref().map(|a| a.artifact_id),
        Some(2)
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            11,
            1,
            vec![plan_step("stale", StepStatus::Pending)],
        )),
    );
    assert_eq!(
        state.artifacts.plan.as_ref().map(|a| a.artifact_id),
        Some(2)
    );
}

#[test]
fn diff_artifact_guard_matrix() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            10,
            1,
            vec![diff_file("a.rs", DiffFileStatus::Modified)],
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            11,
            0,
            vec![diff_file("b.rs", DiffFileStatus::Modified)],
        )),
    );
    assert_eq!(state.artifacts.diff.as_ref().map(|a| a.run_id), Some(11));

    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            9,
            99,
            vec![diff_file("old.rs", DiffFileStatus::Modified)],
        )),
    );
    assert_eq!(state.artifacts.diff.as_ref().map(|a| a.run_id), Some(11));

    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            11,
            2,
            vec![diff_file("c.rs", DiffFileStatus::Modified)],
        )),
    );
    assert_eq!(
        state.artifacts.diff.as_ref().map(|a| a.artifact_id),
        Some(2)
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            11,
            1,
            vec![diff_file("stale.rs", DiffFileStatus::Modified)],
        )),
    );
    assert_eq!(
        state.artifacts.diff.as_ref().map(|a| a.artifact_id),
        Some(2)
    );
}

#[test]
fn verify_artifact_guard_matrix() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetVerifyArtifact(verify_artifact(10, 1, VerifyOverall::Unknown)),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetVerifyArtifact(verify_artifact(11, 0, VerifyOverall::Unknown)),
    );
    assert_eq!(state.artifacts.verify.as_ref().map(|a| a.run_id), Some(11));

    run_runtime(
        &mut state,
        RuntimeAction::SetVerifyArtifact(verify_artifact(9, 99, VerifyOverall::Passing)),
    );
    assert_eq!(state.artifacts.verify.as_ref().map(|a| a.run_id), Some(11));

    run_runtime(
        &mut state,
        RuntimeAction::SetVerifyArtifact(verify_artifact(11, 2, VerifyOverall::Passing)),
    );
    assert_eq!(
        state.artifacts.verify.as_ref().map(|a| a.artifact_id),
        Some(2)
    );

    run_runtime(
        &mut state,
        RuntimeAction::SetVerifyArtifact(verify_artifact(11, 1, VerifyOverall::Failing)),
    );
    assert_eq!(
        state.artifacts.verify.as_ref().map(|a| a.artifact_id),
        Some(2)
    );
}

#[test]
fn older_cross_artifact_arrival_is_stored_but_not_active() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            12,
            1,
            vec![diff_file("new.rs", DiffFileStatus::Modified)],
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            11,
            1,
            vec![plan_step("p1", StepStatus::Pending)],
        )),
    );

    assert_eq!(state.artifacts.plan.as_ref().map(|a| a.run_id), Some(11));
    assert_eq!(state.journey_status.active_run_id, 12);
    assert_eq!(state.journey_status.state, JourneyState::ReviewReady);
}
