#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Personality {
    Friendly,
    Pragmatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadId(pub String);


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion(pub u16);

pub const ARTIFACT_SCHEMA_V1: SchemaVersion = SchemaVersion(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearReason {
    SessionReset,
    UserRequest,
    Superseded,
    InvalidatedByNewRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellTab {
    Chat,
    Overview,
    System,
    Plan,
    Diff,
    Explain,
    Logs,
}

impl ShellTab {
    pub fn next(self) -> Self {
        match self {
            Self::Chat => Self::Overview,
            Self::Overview => Self::System,
            Self::System => Self::Plan,
            Self::Plan => Self::Diff,
            Self::Diff => Self::Explain,
            Self::Explain => Self::Logs,
            Self::Logs => Self::Chat,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Chat => Self::Logs,
            Self::Overview => Self::Chat,
            Self::System => Self::Overview,
            Self::Plan => Self::System,
            Self::Diff => Self::Plan,
            Self::Explain => Self::Diff,
            Self::Logs => Self::Explain,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Overview => "Overview",
            Self::System => "System",
            Self::Plan => "Plan",
            Self::Diff => "Diff",
            Self::Explain => "Explain",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JourneyStep {
    Idea,
    Understand,
    Plan,
    Preview,
    Approve,
    Verify,
    Learn,
}

impl JourneyStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idea => "Idea",
            Self::Understand => "Understand system",
            Self::Plan => "Plan change",
            Self::Preview => "Preview change",
            Self::Approve => "Approve",
            Self::Verify => "Verify",
            Self::Learn => "Learn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JourneyState {
    Idle,
    Scanning,
    Planning,
    Diffing,
    ReviewReady,
    AwaitingApproval,
    Verifying,
    Completed,
    Failed,
}

impl JourneyState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Scanning => "Scanning",
            Self::Planning => "Planning",
            Self::Diffing => "Diffing",
            Self::ReviewReady => "Review ready",
            Self::AwaitingApproval => "Awaiting approval",
            Self::Verifying => "Verifying",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    UserInput,
    Runtime,
    External,
    Unknown,
}

impl ErrorKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::UserInput => "user-input",
            Self::Runtime => "runtime",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyError {
    pub kind: ErrorKind,
    pub message: Arc<str>,
    pub run_id: u64,
}

impl JourneyError {
    pub fn new(kind: ErrorKind, message: impl Into<Arc<str>>, run_id: u64) -> Self {
        Self {
            kind,
            message: message.into(),
            run_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JourneyStatus {
    pub state: JourneyState,
    pub step: JourneyStep,
    pub error: Option<JourneyError>,
    pub active_run_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyMode {
    Safe,
    Supervised,
    FullAccess,
}

impl SafetyMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Safe => "Safe",
            Self::Supervised => "Supervised",
            Self::FullAccess => "Full access",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Unknown,
    Running,
    Ok,
    Warn,
    Fail,
}

impl ScanStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Running => "Running",
            Self::Ok => "Done",
            Self::Warn => "Warn",
            Self::Fail => "Fail",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyStatus {
    NotApplied,
    Applied,
}

impl ApplyStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotApplied => "Not applied",
            Self::Applied => "Applied",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyStatus {
    NotRun,
    Pass,
    Fail,
}

impl VerifyStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotRun => "Not run",
            Self::Pass => "Pass",
            Self::Fail => "Fail",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl RiskLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyTier {
    Strict,
    Balanced,
    Permissive,
}

impl PolicyTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Balanced => "balanced",
            Self::Permissive => "permissive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAction {
    Read,
    Patch,
    Execute,
    Elicitation,
}

impl ApprovalAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Patch => "patch",
            Self::Execute => "execute",
            Self::Elicitation => "elicitation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRiskClass {
    ReadOnly,
    PatchOnly,
    Execution,
    Destructive,
}

impl ApprovalRiskClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::PatchOnly => "patch-only",
            Self::Execution => "execution",
            Self::Destructive => "destructive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalGateRequirement {
    Allow,
    RequireApproval,
    Deny,
}

impl ApprovalGateRequirement {
    pub fn label(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireApproval => "require-approval",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecisionKind {
    Approved,
    Denied,
}

impl ApprovalDecisionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequestRecord {
    pub request_id: String,
    pub run_id: u64,
    pub action: ApprovalAction,
    pub risk: ApprovalRiskClass,
    pub reason: Arc<str>,
    pub preview: Arc<str>,
    pub created_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDecisionRecord {
    pub request_id: String,
    pub run_id: u64,
    pub action: ApprovalAction,
    pub decision: ApprovalDecisionKind,
    pub timestamp_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub request: ApprovalRequestRecord,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyGateState {
    pub run_id: u64,
    pub action: ApprovalAction,
    pub risk: ApprovalRiskClass,
    pub requirement: ApprovalGateRequirement,
    pub reason: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct ApprovalState {
    pub policy_tier: PolicyTier,
    pub pending: Option<PendingApproval>,
    pub last_decision: Option<ApprovalDecisionRecord>,
    pub last_gate: Option<PolicyGateState>,
    pub next_request_seq: u64,
}

impl Default for ApprovalState {
    fn default() -> Self {
        Self {
            policy_tier: PolicyTier::Balanced,
            pending: None,
            last_decision: None,
            last_gate: None,
            next_request_seq: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellOverlay {
    None,
    ActionPalette { selected: usize, query: Arc<str> },
    Onboarding { step: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiTheme {
    Classic,
    Cyberpunk,
    NeonNoir,
    SolarFlare,
    ForestZen,
}

impl UiTheme {
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Cyberpunk => "cyberpunk",
            Self::NeonNoir => "neon-noir",
            Self::SolarFlare => "solar-flare",
            Self::ForestZen => "forest-zen",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Classic => Self::Cyberpunk,
            Self::Cyberpunk => Self::NeonNoir,
            Self::NeonNoir => Self::SolarFlare,
            Self::SolarFlare => Self::ForestZen,
            Self::ForestZen => Self::Classic,
        }
    }

    pub fn accent(self) -> &'static str {
        match self {
            Self::Classic => "cyan",
            Self::Cyberpunk => "magenta",
            Self::NeonNoir => "light-blue",
            Self::SolarFlare => "light-yellow",
            Self::ForestZen => "light-green",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapPreset {
    Standard,
    Mac,
    Windows,
}

impl KeymapPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Mac => "mac",
            Self::Windows => "windows",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Standard => Self::Mac,
            Self::Mac => Self::Windows,
            Self::Windows => Self::Standard,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShellHeader {
    pub project_name: Arc<str>,
    pub safety_mode: SafetyMode,
    pub scan: ScanStatus,
    pub apply: ApplyStatus,
    pub verify: VerifyStatus,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone)]
pub struct ShellRouting {
    pub journey: JourneyStep,
    pub tab: ShellTab,
}

#[derive(Debug, Clone)]
pub struct ShellInteraction {
    pub overlay: ShellOverlay,
    pub focus_in_chat: bool,
}

#[derive(Debug, Clone)]
pub struct ShellCustomization {
    pub theme: UiTheme,
    pub keymap_preset: KeymapPreset,
    pub show_journey: bool,
    pub show_overview: bool,
    pub show_action_bar: bool,
    pub auto_follow_intent: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    pub context_remaining_percent: Option<i64>,
    pub total_tokens: Option<i64>,
    pub primary_window_label: Option<Arc<str>>,
    pub primary_remaining_percent: Option<u8>,
    pub secondary_window_label: Option<Arc<str>>,
    pub secondary_remaining_percent: Option<u8>,
    pub credits_label: Option<Arc<str>>,
}

#[derive(Debug, Clone)]
pub struct SubjectMatterState {
    pub personality: Personality,
    pub persona_policy_defaults: PersonaPolicy,
    pub persona_policy_overrides: PersonaPolicyOverrides,
    pub persona_policy: PersonaPolicy,
    pub skills_enabled_count: usize,
    pub collaboration_mode_label: Arc<str>,
    pub model_slug: Option<Arc<str>>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PersonaPolicyOverrides {
    pub tier_ceiling: Option<PolicyTier>,
    pub explanation_depth: Option<ExplanationDepth>,
    pub output_format: Option<PersonaOutputFormat>,
}

impl PersonaPolicyOverrides {
    pub fn is_empty(self) -> bool {
        self.tier_ceiling.is_none()
            && self.explanation_depth.is_none()
            && self.output_format.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplanationDepth {
    Brief,
    Standard,
    Detailed,
}

impl ExplanationDepth {
    pub fn label(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Standard => "standard",
            Self::Detailed => "detailed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaOutputFormat {
    ImpactFirst,
    TechnicalFirst,
}

impl PersonaOutputFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::ImpactFirst => "impact-first",
            Self::TechnicalFirst => "technical-first",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersonaPolicy {
    pub tier_ceiling: PolicyTier,
    pub explanation_depth: ExplanationDepth,
    pub output_format: PersonaOutputFormat,
    pub tab_order: &'static [ShellTab],
    pub visible_tools: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub struct ArtifactError {
    pub kind: ErrorKind,
    pub message: Arc<str>,
}

#[derive(Debug, Clone)]
pub struct SystemArtifact {
    pub schema_version: SchemaVersion,
    pub run_id: u64,
    pub artifact_id: u64,
    pub repo_root: String,
    pub detected_stack: Vec<String>,
    pub entrypoints: Vec<String>,
    pub risk_flags: Vec<String>,
    pub summary: String,
    pub error: Option<ArtifactError>,
}

#[derive(Debug, Clone)]
pub struct PlanStep {
    pub id: String,
    pub label: String,
    pub status: StepStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PlanArtifact {
    pub schema_version: SchemaVersion,
    pub run_id: u64,
    pub artifact_id: u64,
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub assumptions: Vec<String>,
    pub error: Option<ArtifactError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Remove,
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    pub path: String,
    pub status: DiffFileStatus,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone)]
pub struct DiffArtifact {
    pub schema_version: SchemaVersion,
    pub run_id: u64,
    pub artifact_id: u64,
    pub files: Vec<DiffFile>,
    pub summary: String,
    pub error: Option<ArtifactError>,
}

#[derive(Debug, Clone)]
pub struct VerifyCheck {
    pub name: String,
    pub status: VerifyCheckStatus,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyCheckStatus {
    Pending,
    Running,
    Pass,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOverall {
    Unknown,
    Passing,
    Failing,
}

#[derive(Debug, Clone)]
pub struct VerifyArtifact {
    pub schema_version: SchemaVersion,
    pub run_id: u64,
    pub artifact_id: u64,
    pub checks: Vec<VerifyCheck>,
    pub overall: VerifyOverall,
    pub error: Option<ArtifactError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    App,
    Runtime,
    Shell,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub seq: u64,
    pub level: LogLevel,
    pub ts_ms: Option<u64>,
    pub source: LogSource,
    pub context: Option<String>,
    pub message: String,
    pub run_id: u64,
}

#[derive(Debug, Clone)]
pub struct LogBuffer {
    cap: usize,
    next_seq: u64,
    buf: VecDeque<LogEntry>,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            next_seq: 1,
            buf: VecDeque::with_capacity(cap),
        }
    }

    pub fn append(&mut self, mut entry: LogEntry) {
        entry.seq = self.next_seq;
        self.next_seq += 1;

        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(entry);
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.next_seq = 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &LogEntry> {
        self.buf.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct ShellArtifacts {
    pub schema_version: SchemaVersion,
    pub system: Option<SystemArtifact>,
    pub plan: Option<PlanArtifact>,
    pub diff: Option<DiffArtifact>,
    pub verify: Option<VerifyArtifact>,
    pub logs: LogBuffer,
}

impl Default for ShellArtifacts {
    fn default() -> Self {
        Self {
            schema_version: ARTIFACT_SCHEMA_V1,
            system: None,
            plan: None,
            diff: None,
            verify: None,
            logs: LogBuffer::new(2_000),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeFlagState {
    pub active: bool,
    pub run_id: u64,
}

#[derive(Debug, Clone)]
pub struct RuntimeFlags {
    pub scanning: RuntimeFlagState,
    pub planning: RuntimeFlagState,
    pub diffing: RuntimeFlagState,
    pub awaiting_approval: RuntimeFlagState,
    pub verifying: RuntimeFlagState,
    pub next_run_id: u64,
}

impl Default for RuntimeFlags {
    fn default() -> Self {
        Self {
            scanning: RuntimeFlagState::default(),
            planning: RuntimeFlagState::default(),
            diffing: RuntimeFlagState::default(),
            awaiting_approval: RuntimeFlagState::default(),
            verifying: RuntimeFlagState::default(),
            next_run_id: 1,
        }
    }
}

impl RuntimeFlags {
    pub fn clear_all(&mut self) {
        self.scanning.active = false;
        self.planning.active = false;
        self.diffing.active = false;
        self.awaiting_approval.active = false;
        self.verifying.active = false;
    }

    pub fn current_active_run_id(&self) -> u64 {
        [
            self.scanning,
            self.planning,
            self.diffing,
            self.awaiting_approval,
            self.verifying,
        ]
        .into_iter()
        .filter(|flag| flag.active)
        .map(|flag| flag.run_id)
        .max()
        .unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ShellSelection {
    pub selected_diff_file: Option<String>,
    pub selected_plan_step: Option<String>,
    pub log_level_filter: Option<LogLevel>,
    pub log_search: String,
}

#[derive(Debug, Clone)]
pub struct ShellState {
    pub header: ShellHeader,
    pub usage: UsageSnapshot,
    pub routing: ShellRouting,
    pub journey_status: JourneyStatus,
    pub interaction: ShellInteraction,
    pub customization: ShellCustomization,
    pub sm: SubjectMatterState,
    pub artifacts: ShellArtifacts,
    pub runtime_flags: RuntimeFlags,
    pub approval: ApprovalState,
    pub selection: ShellSelection,
    pub thread_id: Option<ThreadId>,
    pub cwd: Option<PathBuf>,
}

const FRIENDLY_VISIBLE_TOOLS: &[&str] = &["scan_repo", "generate_plan", "verify"];
const PRAGMATIC_VISIBLE_TOOLS: &[&str] = &["scan_repo", "generate_plan", "compute_diff", "verify"];
const FRIENDLY_TAB_ORDER: &[ShellTab] = &[
    ShellTab::Chat,
    ShellTab::Overview,
    ShellTab::Plan,
    ShellTab::Explain,
    ShellTab::Diff,
    ShellTab::Logs,
    ShellTab::System,
];
const PRAGMATIC_TAB_ORDER: &[ShellTab] = &[
    ShellTab::Chat,
    ShellTab::Diff,
    ShellTab::Logs,
    ShellTab::Plan,
    ShellTab::System,
    ShellTab::Explain,
    ShellTab::Overview,
];

pub fn persona_policy_for(personality: Personality) -> PersonaPolicy {
    match personality {
        Personality::Friendly => PersonaPolicy {
            tier_ceiling: PolicyTier::Balanced,
            explanation_depth: ExplanationDepth::Detailed,
            output_format: PersonaOutputFormat::ImpactFirst,
            tab_order: FRIENDLY_TAB_ORDER,
            visible_tools: FRIENDLY_VISIBLE_TOOLS,
        },
        Personality::Pragmatic => PersonaPolicy {
            tier_ceiling: PolicyTier::Permissive,
            explanation_depth: ExplanationDepth::Brief,
            output_format: PersonaOutputFormat::TechnicalFirst,
            tab_order: PRAGMATIC_TAB_ORDER,
            visible_tools: PRAGMATIC_VISIBLE_TOOLS,
        },
    }
}

pub fn apply_persona_policy_overrides(
    defaults: PersonaPolicy,
    overrides: PersonaPolicyOverrides,
) -> PersonaPolicy {
    PersonaPolicy {
        tier_ceiling: overrides.tier_ceiling.unwrap_or(defaults.tier_ceiling),
        explanation_depth: overrides
            .explanation_depth
            .unwrap_or(defaults.explanation_depth),
        output_format: overrides.output_format.unwrap_or(defaults.output_format),
        tab_order: defaults.tab_order,
        visible_tools: defaults.visible_tools,
    }
}

impl ShellState {
    pub fn new(project_name: String, personality: Personality) -> Self {
        let persona_policy_defaults = persona_policy_for(personality);
        let persona_policy_overrides = PersonaPolicyOverrides::default();
        Self {
            header: ShellHeader {
                project_name: project_name.into(),
                safety_mode: SafetyMode::Safe,
                scan: ScanStatus::Unknown,
                apply: ApplyStatus::NotApplied,
                verify: VerifyStatus::NotRun,
                risk: RiskLevel::Low,
            },
            usage: UsageSnapshot::default(),
            routing: ShellRouting {
                journey: JourneyStep::Idea,
                tab: persona_policy_defaults.tab_order[0],
            },
            journey_status: JourneyStatus {
                state: JourneyState::Idle,
                step: JourneyStep::Idea,
                error: None,
                active_run_id: 0,
            },
            interaction: ShellInteraction {
                overlay: ShellOverlay::None,
                focus_in_chat: true,
            },
            customization: ShellCustomization {
                theme: UiTheme::Classic,
                keymap_preset: if cfg!(target_os = "macos") {
                    KeymapPreset::Mac
                } else {
                    KeymapPreset::Standard
                },
                show_journey: false,
                show_overview: true,
                show_action_bar: false,
                auto_follow_intent: false,
            },
            sm: SubjectMatterState {
                personality,
                persona_policy_defaults,
                persona_policy_overrides,
                persona_policy: apply_persona_policy_overrides(
                    persona_policy_defaults,
                    persona_policy_overrides,
                ),
                skills_enabled_count: 0,
                collaboration_mode_label: "code".into(),
                model_slug: None,
                reasoning_effort: None,
            },
            artifacts: ShellArtifacts::default(),
            runtime_flags: RuntimeFlags::default(),
            approval: ApprovalState::default(),
            selection: ShellSelection::default(),
            thread_id: None,
            cwd: None,
        }
    }

    pub fn current_run_id(&self) -> u64 {
        let artifact_run_id = [
            self.artifacts.system.as_ref().map(|a| a.run_id),
            self.artifacts.plan.as_ref().map(|a| a.run_id),
            self.artifacts.diff.as_ref().map(|a| a.run_id),
            self.artifacts.verify.as_ref().map(|a| a.run_id),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(0);

        self.runtime_flags
            .current_active_run_id()
            .max(artifact_run_id)
            .max(
                self.approval
                    .pending
                    .as_ref()
                    .map(|pending| pending.request.run_id)
                    .unwrap_or(0),
            )
            .max(
                self.approval
                    .last_decision
                    .as_ref()
                    .map(|decision| decision.run_id)
                    .unwrap_or(0),
            )
            .max(self.journey_status.active_run_id)
    }

    pub fn ordered_tabs(&self) -> &'static [ShellTab] {
        self.sm.persona_policy.tab_order
    }

    pub fn next_tab(&self) -> ShellTab {
        next_tab_from(self.routing.tab, self.ordered_tabs())
    }

    pub fn prev_tab(&self) -> ShellTab {
        prev_tab_from(self.routing.tab, self.ordered_tabs())
    }
}

fn next_tab_from(current: ShellTab, order: &[ShellTab]) -> ShellTab {
    if order.is_empty() {
        return current;
    }

    if let Some((idx, _)) = order.iter().enumerate().find(|(_, tab)| **tab == current) {
        return order[(idx + 1) % order.len()];
    }

    order[0]
}

fn prev_tab_from(current: ShellTab, order: &[ShellTab]) -> ShellTab {
    if order.is_empty() {
        return current;
    }

    if let Some((idx, _)) = order.iter().enumerate().find(|(_, tab)| **tab == current) {
        if idx == 0 {
            return order[order.len().saturating_sub(1)];
        }
        return order[idx - 1];
    }

    order[0]
}

#[derive(Debug, Clone, Copy)]
pub struct JourneyProjection {
    pub state: JourneyState,
    pub step: JourneyStep,
    pub active_run_id: u64,
}

pub fn derive_journey(
    artifacts: &ShellArtifacts,
    flags: &RuntimeFlags,
    approval: &ApprovalState,
    journey_error: Option<&JourneyError>,
) -> JourneyProjection {
    let active_run_id = [
        flags.scanning,
        flags.planning,
        flags.diffing,
        flags.awaiting_approval,
        flags.verifying,
    ]
    .into_iter()
    .filter(|flag| flag.active)
    .map(|flag| flag.run_id)
    .chain(
        [
            artifacts.system.as_ref().map(|a| a.run_id),
            artifacts.plan.as_ref().map(|a| a.run_id),
            artifacts.diff.as_ref().map(|a| a.run_id),
            artifacts.verify.as_ref().map(|a| a.run_id),
            approval
                .pending
                .as_ref()
                .map(|pending| pending.request.run_id),
        ]
        .into_iter()
        .flatten(),
    )
    .max()
    .unwrap_or(0);

    if let Some(err) = journey_error {
        if err.run_id == active_run_id {
            return JourneyProjection {
                state: JourneyState::Failed,
                step: JourneyStep::Learn,
                active_run_id,
            };
        }
    }

    if approval
        .pending
        .as_ref()
        .is_some_and(|pending| pending.request.run_id == active_run_id)
        || (flags.awaiting_approval.active && flags.awaiting_approval.run_id == active_run_id)
    {
        return JourneyProjection {
            state: JourneyState::AwaitingApproval,
            step: JourneyStep::Approve,
            active_run_id,
        };
    }

    if flags.verifying.active && flags.verifying.run_id == active_run_id {
        return JourneyProjection {
            state: JourneyState::Verifying,
            step: JourneyStep::Verify,
            active_run_id,
        };
    }

    if flags.diffing.active && flags.diffing.run_id == active_run_id {
        return JourneyProjection {
            state: JourneyState::Diffing,
            step: JourneyStep::Preview,
            active_run_id,
        };
    }

    if flags.planning.active && flags.planning.run_id == active_run_id {
        return JourneyProjection {
            state: JourneyState::Planning,
            step: JourneyStep::Plan,
            active_run_id,
        };
    }

    if flags.scanning.active && flags.scanning.run_id == active_run_id {
        return JourneyProjection {
            state: JourneyState::Scanning,
            step: JourneyStep::Understand,
            active_run_id,
        };
    }

    if let Some(verify) = artifacts.verify.as_ref() {
        if verify.run_id == active_run_id && verify.overall == VerifyOverall::Passing {
            return JourneyProjection {
                state: JourneyState::Completed,
                step: JourneyStep::Learn,
                active_run_id,
            };
        }
    }

    if let Some(diff) = artifacts.diff.as_ref() {
        if diff.run_id == active_run_id {
            return JourneyProjection {
                state: JourneyState::ReviewReady,
                step: JourneyStep::Preview,
                active_run_id,
            };
        }
    }

    JourneyProjection {
        state: JourneyState::Idle,
        step: JourneyStep::Idea,
        active_run_id,
    }
}

pub fn artifact_is_newer(
    new_run_id: u64,
    new_artifact_id: u64,
    current: Option<(u64, u64)>,
) -> bool {
    match current {
        None => true,
        Some((run_id, artifact_id)) => {
            new_run_id > run_id || (new_run_id == run_id && new_artifact_id >= artifact_id)
        }
    }
}

pub fn policy_requirement_for_risk(
    tier: PolicyTier,
    risk: ApprovalRiskClass,
) -> ApprovalGateRequirement {
    match tier {
        PolicyTier::Strict => match risk {
            ApprovalRiskClass::ReadOnly => ApprovalGateRequirement::Allow,
            ApprovalRiskClass::PatchOnly | ApprovalRiskClass::Execution => {
                ApprovalGateRequirement::RequireApproval
            }
            ApprovalRiskClass::Destructive => ApprovalGateRequirement::Deny,
        },
        PolicyTier::Balanced => match risk {
            ApprovalRiskClass::ReadOnly | ApprovalRiskClass::PatchOnly => {
                ApprovalGateRequirement::Allow
            }
            ApprovalRiskClass::Execution | ApprovalRiskClass::Destructive => {
                ApprovalGateRequirement::RequireApproval
            }
        },
        PolicyTier::Permissive => match risk {
            ApprovalRiskClass::Destructive => ApprovalGateRequirement::RequireApproval,
            ApprovalRiskClass::ReadOnly
            | ApprovalRiskClass::PatchOnly
            | ApprovalRiskClass::Execution => ApprovalGateRequirement::Allow,
        },
    }
}
