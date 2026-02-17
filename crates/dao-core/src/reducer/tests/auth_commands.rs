use super::*;

#[test]
fn auth_codex_command_emits_auth_effect() {
    let mut state = state();
    state.interaction.chat_input = "/auth codex".to_string();

    let effects = reduce(&mut state, ShellAction::User(UserAction::ChatSubmit));

    assert!(effects.iter().any(|e| {
        matches!(
            e,
            DaoEffect::StartProviderAuth { provider } if provider == "codex"
        )
    }));
    assert!(effects.iter().any(|e| matches!(e, DaoEffect::RequestFrame)));
}

#[test]
fn auth_without_arg_defaults_to_codex() {
    let mut state = state();
    state.interaction.chat_input = "/auth".to_string();

    let effects = reduce(&mut state, ShellAction::User(UserAction::ChatSubmit));

    assert!(effects.iter().any(|e| {
        matches!(
            e,
            DaoEffect::StartProviderAuth { provider } if provider == "codex"
        )
    }));
}
