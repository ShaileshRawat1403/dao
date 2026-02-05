use super::*;
use pretty_assertions::assert_eq;

#[test]
fn log_buffer_seq_is_monotonic() {
    let mut state = state();
    run_runtime(&mut state, RuntimeAction::AppendLog("one".to_string()));
    run_runtime(&mut state, RuntimeAction::AppendLog("two".to_string()));
    run_runtime(&mut state, RuntimeAction::AppendLog("three".to_string()));

    let seqs: Vec<u64> = state.artifacts.logs.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
}

#[test]
fn log_buffer_capacity_eviction_is_fifo() {
    let mut state = state();
    state.artifacts.logs = LogBuffer::new(3);

    for value in ["1", "2", "3", "4", "5"] {
        run_runtime(&mut state, RuntimeAction::AppendLog(value.to_string()));
    }

    let seqs: Vec<u64> = state.artifacts.logs.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, vec![3, 4, 5]);
}

#[test]
fn clear_logs_resets_sequence_to_one() {
    let mut state = state();
    run_runtime(&mut state, RuntimeAction::AppendLog("1".to_string()));
    run_runtime(&mut state, RuntimeAction::AppendLog("2".to_string()));
    run_runtime(
        &mut state,
        RuntimeAction::ClearLogs(ClearReason::UserRequest),
    );
    run_runtime(&mut state, RuntimeAction::AppendLog("3".to_string()));

    let seqs: Vec<u64> = state.artifacts.logs.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, vec![1]);
}

#[test]
fn changing_log_filter_does_not_mutate_log_ordering() {
    let mut state = state();
    run_runtime(&mut state, RuntimeAction::AppendLog("1".to_string()));
    run_runtime(&mut state, RuntimeAction::AppendLog("2".to_string()));

    let before: Vec<(u64, String)> = state
        .artifacts
        .logs
        .iter()
        .map(|entry| (entry.seq, entry.message.clone()))
        .collect();

    let effects = reduce(
        &mut state,
        ShellAction::User(UserAction::SetLogLevelFilter(Some(LogLevel::Warn))),
    );
    assert!(matches!(effects.as_slice(), [DaoEffect::RequestFrame]));

    let after: Vec<(u64, String)> = state
        .artifacts
        .logs
        .iter()
        .map(|entry| (entry.seq, entry.message.clone()))
        .collect();
    assert_eq!(before, after);
}
