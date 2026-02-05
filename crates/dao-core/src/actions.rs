#![allow(dead_code)]

use super::state::Personality;
use super::state::ReasoningEffort;
use super::state::ThreadId;

use std::path::PathBuf;


use super::state::ApplyStatus;
use super::state::ApprovalAction;
use super::state::ApprovalDecisionRecord;
use super::state::ApprovalRequestRecord;
use super::state::ApprovalRiskClass;
use super::state::ClearReason;
use super::state::DiffArtifact;
use super::state::ErrorKind;
use super::state::ExplanationDepth;
use super::state::JourneyError;
use super::state::JourneyState;
use super::state::JourneyStep;
use super::state::KeymapPreset;
use super::state::LogEntry;
use super::state::LogLevel;
use super::state::PersonaOutputFormat;
use super::state::PlanArtifact;
use super::state::PolicyTier;
use super::state::RiskLevel;
use super::state::SafetyMode;
use super::state::ScanStatus;
use super::state::ShellTab;
use super::state::SystemArtifact;
use super::state::UiTheme;
use super::state::UsageSnapshot;
use super::state::VerifyArtifact;
use super::state::VerifyStatus;

#[derive(Debug, Clone)]
pub enum ShellAction {
    User(UserAction),
    Runtime(RuntimeAction),
}

#[derive(Debug, Clone)]
pub enum UserAction {
    ToggleActionPalette,
    ShowOnboarding,
    NextOnboardingStep,
    PrevOnboardingStep,
    CompleteOnboarding,
    SetKeymapPreset(KeymapPreset),
    CycleKeymapPreset,
    SetTheme(UiTheme),
    CycleTheme,
    ToggleJourneyPanel,
    ToggleOverviewPanel,
    ToggleActionBar,
    ToggleAutoIntentFollow,
    CloseOverlay,
    NextTab,
    PrevTab,
    SelectTab(ShellTab),
    NextJourneyStep,
    PrevJourneyStep,
    OverlayMoveUp,
    OverlayMoveDown,
    OverlayQueryInput(char),
    OverlayQueryBackspace,
    OverlayQueryPaste(String),
    OverlaySubmit,
    SelectDiffFile {
        path: String,
    },
    SelectPlanStep {
        id: String,
    },
    SetLogLevelFilter(Option<LogLevel>),
    SetLogSearch(String),
    ClearArtifact {
        which: ClearWhich,
        reason: ClearReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFlag {
    Scanning,
    Planning,
    Diffing,
    AwaitingApproval,
    Verifying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearWhich {
    System,
    Plan,
    Diff,
    Verify,
    Logs,
}

#[derive(Debug, Clone)]
pub enum RuntimeAction {
    SetProjectName(String),
    SetThreadId(Option<ThreadId>),
    SetCwd(Option<PathBuf>),
    SetSafetyMode(SafetyMode),
    SetScanStatus(ScanStatus),
    SetApplyStatus(ApplyStatus),
    SetVerifyStatus(VerifyStatus),
    SetRiskLevel(RiskLevel),
    SetUsage(UsageSnapshot),
    SetKeymapPreset(KeymapPreset),
    SetPersonality(Personality),
    SetPersonaTierCeilingOverride(Option<PolicyTier>),
    SetPersonaExplanationDepthOverride(Option<ExplanationDepth>),
    SetPersonaOutputFormatOverride(Option<PersonaOutputFormat>),
    ClearPersonaPolicyOverrides,
    SetSkillsEnabledCount(usize),
    SetCollaborationModeLabel(String),
    SetModelSlug(Option<String>),
    SetReasoningEffort(Option<ReasoningEffort>),
    SetTab(ShellTab),
    SetJourney(JourneyStep),
    SetJourneyState(JourneyState),
    SetJourneyError {
        kind: ErrorKind,
        message: String,
    },
    ClearJourneyError,

    SetSystemArtifact(SystemArtifact),
    SetPlanArtifact(PlanArtifact),
    SetDiffArtifact(DiffArtifact),
    SetVerifyArtifact(VerifyArtifact),

    ClearSystemArtifact(ClearReason),
    ClearPlanArtifact(ClearReason),
    ClearDiffArtifact(ClearReason),
    ClearVerifyArtifact(ClearReason),

    SetRuntimeFlag {
        flag: RuntimeFlag,
        active: bool,
        run_id: u64,
    },

    SetJourneyErrorState(Option<JourneyError>),
    SetPolicyTier(PolicyTier),
    AssessPolicyGate {
        run_id: u64,
        action: ApprovalAction,
        risk: ApprovalRiskClass,
        reason: String,
    },
    RequestApproval(ApprovalRequestRecord),
    ResolveApproval(ApprovalDecisionRecord),
    ClearApprovalState(ClearReason),

    AppendStructuredLog(LogEntry),
    ClearLogs(ClearReason),

    // Compatibility actions while app/runtime adapter migrates.
    SetOverview(String),
    SetSystem(String),
    SetPlan(String),
    SetDiff(String),
    SetExplain(String),
    AppendLog(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    ContinueInChat,
    ShowOnboarding,
    SetKeymapPreset(KeymapPreset),
    SetTheme(UiTheme),
    CycleTheme,
    ToggleJourneyPanel,
    ToggleOverviewPanel,
    ToggleActionBar,
    ToggleAutoIntentFollow,
    OpenPermissions,
    OpenApprovals,
    OpenSkills,
    StartNewSession,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: &'static str,
    pub command: PaletteCommand,
}

pub const PALETTE_ITEMS: [PaletteItem; 20] = [
    PaletteItem {
        label: "Continue in chat",
        command: PaletteCommand::ContinueInChat,
    },
    PaletteItem {
        label: "Show onboarding guide",
        command: PaletteCommand::ShowOnboarding,
    },
    PaletteItem {
        label: "Keymap: Standard",
        command: PaletteCommand::SetKeymapPreset(KeymapPreset::Standard),
    },
    PaletteItem {
        label: "Keymap: Mac",
        command: PaletteCommand::SetKeymapPreset(KeymapPreset::Mac),
    },
    PaletteItem {
        label: "Keymap: Windows",
        command: PaletteCommand::SetKeymapPreset(KeymapPreset::Windows),
    },
    PaletteItem {
        label: "Theme: Classic",
        command: PaletteCommand::SetTheme(UiTheme::Classic),
    },
    PaletteItem {
        label: "Theme: Cyberpunk",
        command: PaletteCommand::SetTheme(UiTheme::Cyberpunk),
    },
    PaletteItem {
        label: "Theme: Neon Noir",
        command: PaletteCommand::SetTheme(UiTheme::NeonNoir),
    },
    PaletteItem {
        label: "Theme: Solar Flare",
        command: PaletteCommand::SetTheme(UiTheme::SolarFlare),
    },
    PaletteItem {
        label: "Theme: Forest Zen",
        command: PaletteCommand::SetTheme(UiTheme::ForestZen),
    },
    PaletteItem {
        label: "Switch theme",
        command: PaletteCommand::CycleTheme,
    },
    PaletteItem {
        label: "Toggle journey rail",
        command: PaletteCommand::ToggleJourneyPanel,
    },
    PaletteItem {
        label: "Toggle context panel",
        command: PaletteCommand::ToggleOverviewPanel,
    },
    PaletteItem {
        label: "Toggle action bar",
        command: PaletteCommand::ToggleActionBar,
    },
    PaletteItem {
        label: "Toggle intent auto-follow",
        command: PaletteCommand::ToggleAutoIntentFollow,
    },
    PaletteItem {
        label: "Open permissions",
        command: PaletteCommand::OpenPermissions,
    },
    PaletteItem {
        label: "Open approvals",
        command: PaletteCommand::OpenApprovals,
    },
    PaletteItem {
        label: "Open skills",
        command: PaletteCommand::OpenSkills,
    },
    PaletteItem {
        label: "Start new session",
        command: PaletteCommand::StartNewSession,
    },
    PaletteItem {
        label: "Quit A-Eye",
        command: PaletteCommand::Quit,
    },
];

pub fn filtered_palette_indices(query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return (0..PALETTE_ITEMS.len()).collect();
    }

    PALETTE_ITEMS
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            if item.label.to_ascii_lowercase().contains(&query) {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}
