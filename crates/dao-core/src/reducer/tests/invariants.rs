use std::sync::Arc;

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn toggle_action_palette_opens_and_closes_overlay() {
    let mut state = state();

    let effects = reduce(
        &mut state,
        ShellAction::User(UserAction::ToggleActionPalette),
    );
    assert!(matches!(
        state.interaction.overlay,
        ShellOverlay::ActionPalette { .. }
    ));
    assert!(matches!(effects.as_slice(), [DaoEffect::RequestFrame]));

    let effects = reduce(
        &mut state,
        ShellAction::User(UserAction::ToggleActionPalette),
    );
    assert!(matches!(state.interaction.overlay, ShellOverlay::None));
    assert!(matches!(effects.as_slice(), [DaoEffect::RequestFrame]));
}

#[test]
fn journey_projection_matches_after_runtime_mutations() {
    let mut state = state();

    run_runtime(
        &mut state,
        RuntimeAction::SetSystemArtifact(system_artifact(1, 1, "sys")),
    );
    assert_projection_sync(&state);

    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            1,
            1,
            vec![plan_step("1", StepStatus::Pending)],
        )),
    );
    assert_projection_sync(&state);

    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            1,
            1,
            vec![diff_file("a.rs", DiffFileStatus::Modified)],
        )),
    );
    assert_projection_sync(&state);

    run_runtime(
        &mut state,
        RuntimeAction::SetRuntimeFlag {
            flag: RuntimeFlag::Verifying,
            active: true,
            run_id: 1,
        },
    );
    assert_projection_sync(&state);

    run_runtime(
        &mut state,
        RuntimeAction::SetJourneyErrorState(Some(JourneyError::new(
            ErrorKind::Runtime,
            Arc::<str>::from("boom"),
            1,
        ))),
    );
    assert_projection_sync(&state);

    run_runtime(&mut state, RuntimeAction::ClearJourneyError);
    assert_projection_sync(&state);

    run_runtime(
        &mut state,
        RuntimeAction::ClearDiffArtifact(ClearReason::UserRequest),
    );
    assert_projection_sync(&state);
}

#[test]
fn append_log_does_not_change_journey_projection() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetRuntimeFlag {
            flag: RuntimeFlag::Planning,
            active: true,
            run_id: 2,
        },
    );
    let before = (
        state.journey_status.state,
        state.journey_status.step,
        state.journey_status.active_run_id,
    );

    run_runtime(
        &mut state,
        RuntimeAction::AppendStructuredLog(LogEntry {
            seq: 0,
            level: LogLevel::Info,
            ts_ms: Some(1),
            source: LogSource::Runtime,
            context: Some("plan".to_string()),
            message: "log line".to_string(),
            run_id: 2,
        }),
    );

    let after = (
        state.journey_status.state,
        state.journey_status.step,
        state.journey_status.active_run_id,
    );
    assert_eq!(before, after);
}

#[test]
fn new_run_keeps_existing_plan_without_implicit_clears() {
    let mut state = state();
    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            3,
            1,
            vec![plan_step("p1", StepStatus::Done)],
        )),
    );
    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            4,
            1,
            vec![diff_file("d.rs", DiffFileStatus::Modified)],
        )),
    );

    assert!(state.artifacts.plan.is_some());
    assert_eq!(state.journey_status.active_run_id, 4);
    assert_eq!(state.routing.tab, ShellTab::Chat);
}
