use super::*;
use pretty_assertions::assert_eq;

#[test]
fn diff_selection_reconciles_when_selected_file_missing() {
    let mut state = state();
    state.selection.selected_diff_file = Some("b.rs".to_string());

    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            1,
            1,
            vec![
                diff_file("a.rs", DiffFileStatus::Added),
                diff_file("c.rs", DiffFileStatus::Modified),
            ],
        )),
    );

    assert_eq!(state.selection.selected_diff_file.as_deref(), Some("c.rs"));
}

#[test]
fn diff_selection_preserves_existing_file() {
    let mut state = state();
    state.selection.selected_diff_file = Some("b.rs".to_string());

    run_runtime(
        &mut state,
        RuntimeAction::SetDiffArtifact(diff_artifact(
            1,
            1,
            vec![
                diff_file("a.rs", DiffFileStatus::Modified),
                diff_file("b.rs", DiffFileStatus::Added),
            ],
        )),
    );

    assert_eq!(state.selection.selected_diff_file.as_deref(), Some("b.rs"));
}

#[test]
fn plan_selection_reconciles_when_selected_step_missing() {
    let mut state = state();
    state.selection.selected_plan_step = Some("2".to_string());

    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            1,
            1,
            vec![
                plan_step("1", StepStatus::Pending),
                plan_step("3", StepStatus::Running),
            ],
        )),
    );

    assert_eq!(state.selection.selected_plan_step.as_deref(), Some("3"));
}

#[test]
fn plan_selection_preserves_existing_step() {
    let mut state = state();
    state.selection.selected_plan_step = Some("2".to_string());

    run_runtime(
        &mut state,
        RuntimeAction::SetPlanArtifact(plan_artifact(
            1,
            1,
            vec![
                plan_step("1", StepStatus::Pending),
                plan_step("2", StepStatus::Done),
            ],
        )),
    );

    assert_eq!(state.selection.selected_plan_step.as_deref(), Some("2"));
}
