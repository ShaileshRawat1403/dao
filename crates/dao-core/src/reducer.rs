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
    SubmitChat {
        message: String,
        context: Option<String>,
    },
    CopyToClipboard(String),
    StartProviderAuth {
        provider: String,
    },
}

use super::actions::filtered_palette_indices;
use super::actions::ClearWhich;
use super::actions::PaletteCommand;
use super::actions::RuntimeAction;
use super::actions::RuntimeFlag;
use super::actions::ShellAction;
use super::actions::UserAction;
use super::actions::PALETTE_ITEMS;
use super::policy_engine::DecisionOutcome;
use super::policy_engine::PolicyDecision;
use super::policy_engine::Signals;
use super::state::apply_persona_policy_overrides;
use super::state::artifact_is_newer;
use super::state::derive_journey;
use super::state::persona_policy_for;
use super::state::policy_requirement_for_risk;
use super::state::ApprovalGateRequirement;
use super::state::ApprovalRiskClass;
use super::state::ClearReason;
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
use super::state::ARTIFACT_SCHEMA_V1;

pub const AVAILABLE_MODELS: &[&str] = &[
    "gpt-5",
    "gpt-5.3",
    "gpt-5.2",
    "gpt-5.1",
    "gpt-5-mini",
    "gpt-4.1",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
    "phi3:mini-128k",
    "llama3",
    "mistral",
    "gemma:7b",
    "codellama",
    "qwen2.5-coder",
    "deepseek-coder",
];

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
                    query: String::new(),
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
                *query = next;
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
                *query = next;
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
                *query = next;
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
            state.selection.plan_stick_to_running = false;
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
        UserAction::ScrollLogs(delta) => {
            state.selection.log_stick_to_bottom = false;
            state.selection.log_scroll = state.selection.log_scroll.saturating_add_signed(delta);
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetLogScroll(scroll) => {
            state.selection.log_scroll = scroll;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetLogStickToBottom(stick) => {
            state.selection.log_stick_to_bottom = stick;
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
        UserAction::ChatInput(c) => {
            state.interaction.chat_input.push(c);
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ChatBackspace => {
            state.interaction.chat_input.pop();
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ChatSubmit => {
            let input = std::mem::take(&mut state.interaction.chat_input);
            let trimmed_input = input.trim();
            if !trimmed_input.is_empty() {
                if trimmed_input.starts_with('/') {
                    state.interaction.chat_history.push(input.clone());
                    state.interaction.chat_history_index = None;
                    state.interaction.chat_input.clear();

                    let mut parts = trimmed_input.split_whitespace();
                    let command = parts.next().unwrap_or_default();
                    let argument = parts.next();
                    let argument_tail = trimmed_input
                        .split_once(char::is_whitespace)
                        .map(|(_, rest)| rest.trim())
                        .unwrap_or("");

                    match command {
                        "/m" | "/models" => {
                            state.interaction.overlay =
                                ShellOverlay::ModelSelection { selected: 0 };
                        }
                        "/model" => {
                            if let Some(model) = argument {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::SetModelSlug(Some(model.to_string())),
                                );
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Model set to {}",
                                        model
                                    )),
                                );
                            } else {
                                state.interaction.overlay =
                                    ShellOverlay::ModelSelection { selected: 0 };
                            }
                        }
                        "/provider" => {
                            if let Some(provider) = argument {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::SetModelProvider(Some(provider.to_string())),
                                );
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Provider set to {}",
                                        provider
                                    )),
                                );
                            } else {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Usage: /provider <ollama|codex|gemini>".to_string(),
                                    ),
                                );
                            }
                        }
                        "/status" => {
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(format!(
                                    "[meta] Status | tab:{} | journey:{} | mode:{} | provider:{} | model:{} | risk:{}",
                                    state.routing.tab.label(),
                                    state.journey_status.state.label(),
                                    state.header.safety_mode.label(),
                                    state.sm.model_provider.as_deref().unwrap_or("ollama"),
                                    state.sm.model_slug.as_deref().unwrap_or("default"),
                                    state.header.risk.label()
                                )),
                            );
                        }
                        "/search" => {
                            if argument_tail.is_empty() {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Usage: /search <text|clear> | current: {}",
                                        if state.selection.log_search.is_empty() {
                                            "(none)".to_string()
                                        } else {
                                            state.selection.log_search.clone()
                                        }
                                    )),
                                );
                            } else if argument_tail.eq_ignore_ascii_case("clear")
                                || argument_tail.eq_ignore_ascii_case("off")
                            {
                                state.selection.log_search.clear();
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Chat search filter cleared".to_string(),
                                    ),
                                );
                            } else {
                                state.selection.log_search = argument_tail.to_string();
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Chat search filter set to '{}'",
                                        argument_tail
                                    )),
                                );
                            }
                        }
                        "/streammeta" => {
                            let arg = argument_tail.to_ascii_lowercase();
                            match arg.as_str() {
                                "" | "toggle" => {
                                    state.interaction.stream_meta_enabled =
                                        !state.interaction.stream_meta_enabled;
                                }
                                "on" | "true" | "1" => {
                                    state.interaction.stream_meta_enabled = true;
                                }
                                "off" | "false" | "0" => {
                                    state.interaction.stream_meta_enabled = false;
                                }
                                "status" => {}
                                _ => {
                                    reduce_runtime(
                                        state,
                                        RuntimeAction::AppendLog(
                                            "[meta] Usage: /streammeta <on|off|toggle|status>"
                                                .to_string(),
                                        ),
                                    );
                                    return vec![DaoEffect::RequestFrame];
                                }
                            }
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(format!(
                                    "[meta] Stream metadata: {}",
                                    if state.interaction.stream_meta_enabled {
                                        "on"
                                    } else {
                                        "off"
                                    }
                                )),
                            );
                        }
                        "/auth" | "/login" | "/signin" => {
                            let provider_name = if argument_tail.is_empty() {
                                "codex"
                            } else {
                                argument_tail
                            };
                            if provider_name.eq_ignore_ascii_case("codex") {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Starting Codex authentication flow. Complete the browser/device verification prompt when shown."
                                            .to_string(),
                                    ),
                                );
                                return vec![
                                    DaoEffect::StartProviderAuth {
                                        provider: "codex".to_string(),
                                    },
                                    DaoEffect::RequestFrame,
                                ];
                            } else {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Unsupported auth provider '{}'. Supported: codex",
                                        provider_name
                                    )),
                                );
                            }
                        }
                        "/tab" => {
                            if argument_tail.is_empty() {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Usage: /tab <chat|overview|telemetry|system|plan|diff|explain|logs|files|1-9>"
                                            .to_string(),
                                    ),
                                );
                            } else if let Some(tab) = parse_shell_tab(argument_tail) {
                                state.routing.tab = tab;
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Switched tab to {}",
                                        tab.label()
                                    )),
                                );
                            } else {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Unknown tab '{}'",
                                        argument_tail
                                    )),
                                );
                            }
                        }
                        "/theme" => {
                            if argument_tail.is_empty() {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Usage: /theme <classic|cyberpunk|neon-noir|solar-flare|forest-zen|next|prev>"
                                            .to_string(),
                                    ),
                                );
                            } else if argument_tail.eq_ignore_ascii_case("next") {
                                state.customization.theme = state.customization.theme.next();
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Theme set to {}",
                                        state.customization.theme.label()
                                    )),
                                );
                            } else if argument_tail.eq_ignore_ascii_case("prev") {
                                state.customization.theme = state.customization.theme.prev();
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Theme set to {}",
                                        state.customization.theme.label()
                                    )),
                                );
                            } else if let Some(theme) = parse_theme(argument_tail) {
                                state.customization.theme = theme;
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Theme set to {}",
                                        theme.label()
                                    )),
                                );
                            } else {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(format!(
                                        "[meta] Unknown theme '{}'",
                                        argument_tail
                                    )),
                                );
                            }
                        }
                        "/panel" => {
                            if argument_tail.is_empty() {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Usage: /panel <journey|context|actions>"
                                            .to_string(),
                                    ),
                                );
                            } else {
                                match argument_tail.to_ascii_lowercase().as_str() {
                                    "journey" => {
                                        state.customization.show_journey =
                                            !state.customization.show_journey;
                                        reduce_runtime(
                                            state,
                                            RuntimeAction::AppendLog(format!(
                                                "[meta] Journey rail: {}",
                                                if state.customization.show_journey {
                                                    "on"
                                                } else {
                                                    "off"
                                                }
                                            )),
                                        );
                                    }
                                    "context" | "overview" => {
                                        state.customization.show_overview =
                                            !state.customization.show_overview;
                                        reduce_runtime(
                                            state,
                                            RuntimeAction::AppendLog(format!(
                                                "[meta] Context rail: {}",
                                                if state.customization.show_overview {
                                                    "on"
                                                } else {
                                                    "off"
                                                }
                                            )),
                                        );
                                    }
                                    "actions" | "actionbar" => {
                                        state.customization.show_action_bar =
                                            !state.customization.show_action_bar;
                                        reduce_runtime(
                                            state,
                                            RuntimeAction::AppendLog(format!(
                                                "[meta] Action bar: {}",
                                                if state.customization.show_action_bar {
                                                    "on"
                                                } else {
                                                    "off"
                                                }
                                            )),
                                        );
                                    }
                                    _ => {
                                        reduce_runtime(
                                            state,
                                            RuntimeAction::AppendLog(format!(
                                                "[meta] Unknown panel '{}'",
                                                argument_tail
                                            )),
                                        );
                                    }
                                }
                            }
                        }
                        "/telemetry" => {
                            state.routing.tab = super::state::ShellTab::Telemetry;
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(
                                    "[meta] Switched tab to Telemetry".to_string(),
                                ),
                            );
                        }
                        "/copylast" => {
                            if let Some(text) = latest_assistant_text(state) {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Copied last assistant response to clipboard"
                                            .to_string(),
                                    ),
                                );
                                return vec![
                                    DaoEffect::CopyToClipboard(text),
                                    DaoEffect::RequestFrame,
                                ];
                            }
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(
                                    "[meta] No assistant response available to copy".to_string(),
                                ),
                            );
                        }
                        "/copydiff" => {
                            if let Some(text) = full_diff_text(state) {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Copied full diff to clipboard".to_string(),
                                    ),
                                );
                                return vec![
                                    DaoEffect::CopyToClipboard(text),
                                    DaoEffect::RequestFrame,
                                ];
                            }
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(
                                    "[meta] No diff available to copy".to_string(),
                                ),
                            );
                        }
                        "/copychat" => {
                            if let Some(text) = full_chat_text(state) {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Copied chat transcript to clipboard".to_string(),
                                    ),
                                );
                                return vec![
                                    DaoEffect::CopyToClipboard(text),
                                    DaoEffect::RequestFrame,
                                ];
                            }
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(
                                    "[meta] No chat transcript available to copy".to_string(),
                                ),
                            );
                        }
                        "/copylogs" => {
                            if let Some(text) = full_logs_text(state) {
                                reduce_runtime(
                                    state,
                                    RuntimeAction::AppendLog(
                                        "[meta] Copied logs to clipboard".to_string(),
                                    ),
                                );
                                return vec![
                                    DaoEffect::CopyToClipboard(text),
                                    DaoEffect::RequestFrame,
                                ];
                            }
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(
                                    "[meta] No logs available to copy".to_string(),
                                ),
                            );
                        }
                        "/z" | "/focus" => {
                            state.customization.focus_mode = !state.customization.focus_mode;
                        }
                        "/clear" => {
                            reduce_runtime(
                                state,
                                RuntimeAction::ClearLogs(ClearReason::UserRequest),
                            );
                        }
                        "/h" | "/help" => {
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(
                                    "[meta] Commands: /models, /model <name>, /provider <name>, /tab <name>, /theme <name|next|prev>, /panel <journey|context|actions>, /search <text|clear>, /streammeta <on|off|toggle|status>, /auth [codex], /login [codex], /telemetry, /status, /copylast, /copydiff, /copychat, /copylogs, /focus, /clear, /help"
                                        .to_string(),
                                ),
                            );
                        }
                        _ => {
                            reduce_runtime(
                                state,
                                RuntimeAction::AppendLog(format!(
                                    "Unknown command: {}",
                                    trimmed_input
                                )),
                            );
                        }
                    }
                    return vec![DaoEffect::RequestFrame];
                }

                state.interaction.chat_history.push(input.clone());
                state.interaction.chat_history_index = None;
                state.interaction.is_thinking = true;
                state.interaction.live_assistant_preview.clear();
                reduce_runtime(state, RuntimeAction::AppendLog(format!("> {}", input)));
                let context = build_chat_context(state);
                vec![
                    DaoEffect::RequestFrame,
                    DaoEffect::SubmitChat {
                        message: input,
                        context,
                    },
                ]
            } else {
                vec![DaoEffect::RequestFrame]
            }
        }
        UserAction::SetChatFocus(focus) => {
            state.interaction.focus_in_chat = focus;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ResetSession => {
            state.interaction.overlay = ShellOverlay::ConfirmReset;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ConfirmReset => {
            state.interaction.overlay = ShellOverlay::None;
            reduce_runtime(state, RuntimeAction::SetJourneyState(JourneyState::Idle));
            reduce_runtime(
                state,
                RuntimeAction::ClearSystemArtifact(ClearReason::UserRequest),
            );
            reduce_runtime(
                state,
                RuntimeAction::ClearPlanArtifact(ClearReason::UserRequest),
            );
            reduce_runtime(
                state,
                RuntimeAction::ClearDiffArtifact(ClearReason::UserRequest),
            );
            reduce_runtime(
                state,
                RuntimeAction::ClearVerifyArtifact(ClearReason::UserRequest),
            );
            reduce_runtime(state, RuntimeAction::ClearLogs(ClearReason::UserRequest));
            reduce_runtime(
                state,
                RuntimeAction::ClearApprovalState(ClearReason::UserRequest),
            );
            state.interaction.chat_input.clear();
            vec![DaoEffect::RequestFrame]
        }
        UserAction::CancelReset => {
            state.interaction.overlay = ShellOverlay::None;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ShowHelp => {
            state.interaction.overlay = ShellOverlay::Help;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ChatHistoryUp => {
            if let Some(idx) = state.interaction.chat_history_index {
                if idx > 0 {
                    let new_idx = idx - 1;
                    state.interaction.chat_history_index = Some(new_idx);
                    state.interaction.chat_input = state.interaction.chat_history[new_idx].clone();
                }
            } else if !state.interaction.chat_history.is_empty() {
                let new_idx = state.interaction.chat_history.len() - 1;
                state.interaction.chat_history_index = Some(new_idx);
                state.interaction.chat_input = state.interaction.chat_history[new_idx].clone();
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ChatHistoryDown => {
            if let Some(idx) = state.interaction.chat_history_index {
                if idx < state.interaction.chat_history.len() - 1 {
                    let new_idx = idx + 1;
                    state.interaction.chat_history_index = Some(new_idx);
                    state.interaction.chat_input = state.interaction.chat_history[new_idx].clone();
                } else {
                    state.interaction.chat_history_index = None;
                    state.interaction.chat_input = String::new();
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ReviewChanges => {
            state.routing.tab = super::state::ShellTab::Diff;
            let input = "Please review the changes in the current diff.".to_string();
            state.interaction.chat_history.push(input.clone());
            state.interaction.chat_history_index = None;
            state.interaction.is_thinking = true;
            reduce_runtime(state, RuntimeAction::AppendLog(format!("> {}", input)));
            let context = build_chat_context(state);
            vec![
                DaoEffect::RequestFrame,
                DaoEffect::SubmitChat {
                    message: input,
                    context,
                },
            ]
        }
        UserAction::ResizeInput(delta) => {
            let new_height = state.customization.input_height as i16 + delta;
            state.customization.input_height = new_height.clamp(1, 20) as u16;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ToggleFocusMode => {
            state.customization.focus_mode = !state.customization.focus_mode;
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ShowModelSelection => {
            state.interaction.overlay = ShellOverlay::ModelSelection { selected: 0 };
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ModelListMoveUp => {
            if let ShellOverlay::ModelSelection { selected } = &mut state.interaction.overlay {
                if *selected > 0 {
                    *selected -= 1;
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ModelListMoveDown => {
            if let ShellOverlay::ModelSelection { selected } = &mut state.interaction.overlay {
                if *selected < AVAILABLE_MODELS.len() - 1 {
                    *selected += 1;
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::ModelListSubmit => {
            if let ShellOverlay::ModelSelection { selected } = state.interaction.overlay {
                state.interaction.overlay = ShellOverlay::None;
                reduce_runtime(
                    state,
                    RuntimeAction::SetModelSlug(Some(AVAILABLE_MODELS[selected].to_string())),
                );
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::CopyDiffToClipboard => {
            if let Some(diff) = &state.artifacts.diff {
                let content = if let Some(selected) = &state.selection.selected_diff_file {
                    diff.files
                        .iter()
                        .find(|f| f.path == *selected)
                        .map(|f| {
                            let mut s = String::new();
                            for hunk in &f.hunks {
                                s.push_str(&hunk.header);
                                s.push('\n');
                                for line in &hunk.lines {
                                    s.push_str(&line.text);
                                    s.push('\n');
                                }
                            }
                            s
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                if !content.is_empty() {
                    return vec![DaoEffect::CopyToClipboard(content), DaoEffect::RequestFrame];
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::SetPlanStickToRunning(active) => {
            state.selection.plan_stick_to_running = active;
            if active {
                reconcile_selected_plan_step(state);
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::PlanStepUp => {
            if let Some(plan) = &state.artifacts.plan {
                state.selection.plan_stick_to_running = false;
                let current_idx = state
                    .selection
                    .selected_plan_step
                    .as_ref()
                    .and_then(|id| plan.steps.iter().position(|s| s.id == *id));

                if let Some(idx) = current_idx {
                    if idx > 0 {
                        state.selection.selected_plan_step = Some(plan.steps[idx - 1].id.clone());
                    }
                } else if !plan.steps.is_empty() {
                    state.selection.selected_plan_step =
                        Some(plan.steps.last().unwrap().id.clone());
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::PlanStepDown => {
            if let Some(plan) = &state.artifacts.plan {
                state.selection.plan_stick_to_running = false;
                let current_idx = state
                    .selection
                    .selected_plan_step
                    .as_ref()
                    .and_then(|id| plan.steps.iter().position(|s| s.id == *id));

                if let Some(idx) = current_idx {
                    if idx < plan.steps.len() - 1 {
                        state.selection.selected_plan_step = Some(plan.steps[idx + 1].id.clone());
                    }
                } else if !plan.steps.is_empty() {
                    state.selection.selected_plan_step = Some(plan.steps[0].id.clone());
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::TogglePlanStepExpansion => {
            if let Some(selected) = &state.selection.selected_plan_step {
                if let Some(pos) = state
                    .selection
                    .expanded_plan_steps
                    .iter()
                    .position(|id| id == selected)
                {
                    state.selection.expanded_plan_steps.remove(pos);
                } else {
                    state.selection.expanded_plan_steps.push(selected.clone());
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::PlanStepPageUp => {
            if let Some(plan) = &state.artifacts.plan {
                state.selection.plan_stick_to_running = false;
                let current_idx = state
                    .selection
                    .selected_plan_step
                    .as_ref()
                    .and_then(|id| plan.steps.iter().position(|s| s.id == *id));

                if let Some(idx) = current_idx {
                    let new_idx = idx.saturating_sub(10);
                    state.selection.selected_plan_step = Some(plan.steps[new_idx].id.clone());
                } else if !plan.steps.is_empty() {
                    state.selection.selected_plan_step =
                        Some(plan.steps.last().unwrap().id.clone());
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::PlanStepPageDown => {
            if let Some(plan) = &state.artifacts.plan {
                state.selection.plan_stick_to_running = false;
                let current_idx = state
                    .selection
                    .selected_plan_step
                    .as_ref()
                    .and_then(|id| plan.steps.iter().position(|s| s.id == *id));

                if let Some(idx) = current_idx {
                    let new_idx = (idx + 10).min(plan.steps.len().saturating_sub(1));
                    state.selection.selected_plan_step = Some(plan.steps[new_idx].id.clone());
                } else if !plan.steps.is_empty() {
                    state.selection.selected_plan_step = Some(plan.steps[0].id.clone());
                }
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::FileBrowserUp => {
            if state.file_browser.selected > 0 {
                state.file_browser.selected -= 1;
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::FileBrowserDown => {
            if state.file_browser.selected < state.file_browser.entries.len() - 1 {
                state.file_browser.selected += 1;
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::FileBrowserEnter => {
            let selected_entry = state.file_browser.entries[state.file_browser.selected].clone();
            let mut new_path = state.file_browser.current_path.clone();
            new_path.push(selected_entry);
            if new_path.is_dir() {
                state.file_browser.current_path = new_path;
                state.file_browser.selected = 0;
            }
            vec![DaoEffect::RequestFrame]
        }
        UserAction::FileBrowserBack => {
            if state.file_browser.current_path.pop() {
                state.file_browser.selected = 0;
            }
            vec![DaoEffect::RequestFrame]
        }
    }
}

fn build_chat_context(state: &ShellState) -> Option<String> {
    let mut context = String::new();
    const MAX_CONTEXT_CHARS: usize = 32_000;

    if let Some(diff) = &state.artifacts.diff {
        context.push_str("Current Diff:\n");
        'outer: for file in &diff.files {
            let file_header = format!("File: {} ({:?})\n", file.path, file.status);
            if context.len() + file_header.len() > MAX_CONTEXT_CHARS {
                context.push_str("... (truncated)\n");
                break 'outer;
            }
            context.push_str(&file_header);

            for hunk in &file.hunks {
                if context.len() + hunk.header.len() + 1 > MAX_CONTEXT_CHARS {
                    context.push_str("... (truncated)\n");
                    break 'outer;
                }
                context.push_str(&hunk.header);
                context.push('\n');
                for line in &hunk.lines {
                    if context.len() + line.text.len() + 1 > MAX_CONTEXT_CHARS {
                        context.push_str("... (truncated)\n");
                        break 'outer;
                    }
                    context.push_str(&line.text);
                    context.push('\n');
                }
            }
        }
        context.push('\n');
    }

    if context.is_empty() {
        None
    } else {
        Some(context)
    }
}

fn parse_shell_tab(input: &str) -> Option<super::state::ShellTab> {
    match input.trim().to_ascii_lowercase().as_str() {
        "1" | "chat" => Some(super::state::ShellTab::Chat),
        "2" | "overview" => Some(super::state::ShellTab::Overview),
        "3" | "telemetry" => Some(super::state::ShellTab::Telemetry),
        "4" | "system" => Some(super::state::ShellTab::System),
        "5" | "plan" => Some(super::state::ShellTab::Plan),
        "6" | "diff" => Some(super::state::ShellTab::Diff),
        "7" | "explain" => Some(super::state::ShellTab::Explain),
        "8" | "logs" => Some(super::state::ShellTab::Logs),
        "9" | "files" | "file" | "filebrowser" => Some(super::state::ShellTab::FileBrowser),
        _ => None,
    }
}

fn parse_theme(input: &str) -> Option<super::state::UiTheme> {
    match input.trim().to_ascii_lowercase().as_str() {
        "classic" => Some(super::state::UiTheme::Classic),
        "cyberpunk" => Some(super::state::UiTheme::Cyberpunk),
        "neon-noir" | "neonnoir" => Some(super::state::UiTheme::NeonNoir),
        "solar-flare" | "solarflare" => Some(super::state::UiTheme::SolarFlare),
        "forest-zen" | "forestzen" => Some(super::state::UiTheme::ForestZen),
        _ => None,
    }
}

fn latest_assistant_text(state: &ShellState) -> Option<String> {
    state.artifacts.logs.iter().rev().find_map(|entry| {
        entry
            .message
            .strip_prefix("[assistant] ")
            .map(|s| s.to_string())
    })
}

fn full_diff_text(state: &ShellState) -> Option<String> {
    let diff = state.artifacts.diff.as_ref()?;
    let mut out = String::new();
    for file in &diff.files {
        out.push_str(&format!("--- {} ({:?})\n", file.path, file.status));
        for hunk in &file.hunks {
            out.push_str(&hunk.header);
            out.push('\n');
            for line in &hunk.lines {
                out.push_str(&line.text);
                out.push('\n');
            }
        }
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn full_chat_text(state: &ShellState) -> Option<String> {
    let mut out = String::new();
    for entry in state
        .artifacts
        .logs
        .iter()
        .filter(|e| e.source == LogSource::Shell || e.source == LogSource::Runtime)
    {
        out.push_str(&entry.message);
        out.push('\n');
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn full_logs_text(state: &ShellState) -> Option<String> {
    let mut out = String::new();
    for entry in state.artifacts.logs.iter() {
        out.push_str(&format!("[{:?}] {}\n", entry.level, entry.message));
    }
    if out.trim().is_empty() {
        None
    } else {
        Some(out)
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
            state.sm.collaboration_mode_label = label;
        }
        RuntimeAction::SetModelSlug(model) => {
            state.sm.model_slug = model;
        }
        RuntimeAction::SetModelProvider(provider) => {
            state.sm.model_provider = provider;
        }
        RuntimeAction::SetReasoningEffort(effort) => {
            state.sm.reasoning_effort = effort;
        }
        RuntimeAction::SetTab(tab) => {
            maybe_follow_tab(state, tab);
        }
        RuntimeAction::SetJourney(_) => {} // No-op, handled by SetJourneyState
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
            state.journey_status.error = Some(JourneyError::new(kind, message, run_id));
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
        RuntimeAction::SetReviewPolicy(policy) => {
            state.approval.active_policy = Some(policy);
        }
        RuntimeAction::AssessPolicyGate {
            run_id,
            action,
            risk,
            reason,
        } => {
            if let Some(policy) = &state.approval.active_policy {
                let signals = Signals {
                    diff_files_changed: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| d.files.len())
                        .unwrap_or(0),
                    diff_lines_added: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| {
                            d.files
                                .iter()
                                .flat_map(|f| f.hunks.iter())
                                .flat_map(|h| h.lines.iter())
                                .filter(|l| l.kind == DiffLineKind::Add)
                                .count()
                        })
                        .unwrap_or(0),
                    diff_lines_deleted: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| {
                            d.files
                                .iter()
                                .flat_map(|f| f.hunks.iter())
                                .flat_map(|h| h.lines.iter())
                                .filter(|l| l.kind == DiffLineKind::Remove)
                                .count()
                        })
                        .unwrap_or(0),
                    risk_class: risk.label().to_string(),
                    diff_file_names: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| {
                            d.files
                                .iter()
                                .map(|f| f.path.clone())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default(),
                    commit_message: reason.clone(),
                    diff_added_content: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| {
                            d.files
                                .iter()
                                .flat_map(|f| f.hunks.iter())
                                .flat_map(|h| h.lines.iter())
                                .filter_map(|l| {
                                    if l.kind == DiffLineKind::Add {
                                        Some(l.text.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default(),
                    new_file_contents: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| {
                            d.files
                                .iter()
                                .filter(|f| f.status == DiffFileStatus::Added)
                                .map(|f| {
                                    f.hunks
                                        .iter()
                                        .flat_map(|h| h.lines.iter())
                                        .map(|l| l.text.clone())
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    new_file_paths: state
                        .artifacts
                        .diff
                        .as_ref()
                        .map(|d| {
                            d.files
                                .iter()
                                .filter(|f| f.status == DiffFileStatus::Added)
                                .map(|f| f.path.clone())
                                .collect()
                        })
                        .unwrap_or_default(),
                };

                let decision: PolicyDecision = policy.evaluate(&signals);
                let requirement = match decision.decision {
                    DecisionOutcome::Allowed => ApprovalGateRequirement::Allow,
                    DecisionOutcome::Blocked => ApprovalGateRequirement::Deny,
                    DecisionOutcome::ApprovalRequired => ApprovalGateRequirement::RequireApproval,
                };

                state.approval.last_gate = Some(PolicyGateState {
                    run_id,
                    action,
                    risk,
                    requirement,
                    reason: decision.message,
                });
            } else {
                let requirement = policy_requirement_for_risk(state.approval.policy_tier, risk);
                state.approval.last_gate = Some(PolicyGateState {
                    run_id,
                    action,
                    risk,
                    requirement,
                    reason,
                });
            }
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
                    message: format!("approval request queued for run {}", run_id),
                    run_id,
                });
            }
        }
        RuntimeAction::ResolveApproval(decision) => {
            if let Some(pending) = state.approval.pending.as_ref() {
                if pending.request.request_id == decision.request_id
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
                        reason: format!(
                            "request {} {}",
                            decision.request_id,
                            decision.decision.label()
                        ),
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
        RuntimeAction::SetThinking(is_thinking) => {
            state.interaction.is_thinking = is_thinking;
            dirty = true;
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
        state.sm.persona_policy_defaults.clone(),
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

    if let Some(current) = state.selection.selected_diff_file.as_deref() {
        if diff.files.iter().any(|file| file.path == current) {
            return;
        }
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

    if state.selection.plan_stick_to_running {
        if let Some(step) = plan
            .steps
            .iter()
            .find(|step| matches!(step.status, StepStatus::Running))
        {
            state.selection.selected_plan_step = Some(step.id.clone());
            return;
        }
    }

    if let Some(current) = state.selection.selected_plan_step.as_deref() {
        if plan.steps.iter().any(|step| step.id == current) {
            return;
        }
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
        if let Some(hunk_value) = hunk.take() {
            if let Some(file_value) = file.as_mut() {
                file_value.hunks.push(hunk_value);
            }
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
                header: format!("@@{}", header),
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
