use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Sparkline, Tabs, Wrap,
};
use ratatui::Terminal;

use dao_core::actions::RuntimeAction;
use dao_core::actions::{filtered_palette_indices, ShellAction, UserAction, PALETTE_ITEMS};
use dao_core::reducer::{reduce, DaoEffect, AVAILABLE_MODELS};
use dao_core::state::{
    DiffLineKind, JourneyState, LogLevel, ShellOverlay, ShellState, ShellTab, StepStatus, UiTheme,
};

use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn get_syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn get_theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

struct TuiGuard;

impl Drop for TuiGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            crossterm::cursor::Show
        );
    }
}

pub fn run(mut state: ShellState, repo: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        crossterm::cursor::Hide
    )?;
    let _guard = TuiGuard; // Ensures terminal is restored on exit or panic

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    run_app(&mut terminal, &mut state, &repo).map_err(|e| e.into())
}

enum UiEvent {
    Token(String),
    StreamMeta(String),
    Finished { elapsed_ms: u64, bytes: usize },
    AuthOutput(String),
    AuthFinished { provider: String, success: bool },
}

fn resolved_model_slug(state: &ShellState) -> &str {
    let provider = resolved_provider(state);
    state
        .sm
        .model_slug
        .as_deref()
        .or(state.config.model.default_model.as_deref())
        .unwrap_or(match provider {
            "codex" => "gpt-5",
            "gemini" => "gemini-2.5-pro",
            _ => "phi3:mini-128k",
        })
}

fn resolved_provider(state: &ShellState) -> &str {
    state
        .sm
        .model_provider
        .as_deref()
        .or(state.config.model.default_provider.as_deref())
        .unwrap_or("ollama")
}

fn chat_line_count(state: &ShellState) -> usize {
    let filter = state.selection.log_search.trim().to_ascii_lowercase();
    let mut lines = 0_usize;
    let mut last_role = "";
    for entry in state.artifacts.logs.iter().filter(|l| {
        l.source == dao_core::state::LogSource::Shell
            || l.source == dao_core::state::LogSource::Runtime
    }) {
        let (role, text) = if entry.message.starts_with("> ") {
            ("you", entry.message.trim_start_matches("> "))
        } else if let Some(content) = entry.message.strip_prefix("[assistant] ") {
            ("assistant", content)
        } else if let Some(content) = entry.message.strip_prefix("[meta] ") {
            ("meta", content)
        } else {
            ("system", entry.message.as_str())
        };
        if !filter.is_empty() && !text.to_ascii_lowercase().contains(&filter) {
            continue;
        }
        if role != last_role {
            lines += 1; // role label
            last_role = role;
        }
        lines += text.split('\n').count();
        lines += 1; // spacer
    }
    if state.interaction.is_thinking && !state.interaction.live_assistant_preview.is_empty() {
        lines += 1; // preview label
        lines += state.interaction.live_assistant_preview.split('\n').count();
        lines += 1; // spacer
    }
    lines
}

const CHAT_COMMAND_SUGGESTIONS: &[&str] = &[
    "/help",
    "/status",
    "/auth [codex]",
    "/login [codex]",
    "/search <text|clear>",
    "/streammeta <on|off|toggle|status>",
    "/models",
    "/model <name>",
    "/provider <ollama|codex|gemini>",
    "/tab <chat|overview|telemetry|system|plan|diff|explain|logs|files|1-9>",
    "/theme <classic|cyberpunk|neon-noir|solar-flare|forest-zen|next|prev>",
    "/panel <journey|context|actions>",
    "/telemetry",
    "/copylast",
    "/copydiff",
    "/copychat",
    "/copylogs",
    "/focus",
    "/clear",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChatRole {
    User,
    Assistant,
    Meta,
    System,
}

fn parse_chat_role(message: &str) -> (ChatRole, String) {
    if message.starts_with("> ") {
        (ChatRole::User, message.trim_start_matches("> ").to_string())
    } else if let Some(content) = message.strip_prefix("[assistant] ") {
        (ChatRole::Assistant, content.to_string())
    } else if let Some(content) = message.strip_prefix("[meta] ") {
        (ChatRole::Meta, content.to_string())
    } else {
        (ChatRole::System, message.to_string())
    }
}

fn message_matches_filter(message: &str, filter_lower: &str) -> bool {
    if filter_lower.is_empty() {
        return true;
    }
    message.to_ascii_lowercase().contains(filter_lower)
}

fn role_style(role: ChatRole, palette: UiPalette) -> Style {
    let color = match role {
        ChatRole::User => palette.accent,
        ChatRole::Assistant => palette.success,
        ChatRole::Meta => palette.muted,
        ChatRole::System => palette.warning,
    };
    Style::default().fg(color)
}

fn push_with_inline_code(
    out: &mut Vec<Line<'static>>,
    prefix: &str,
    text: &str,
    base: Style,
    code: Style,
    strong: Style,
    italic: Style,
) {
    let mut spans = vec![Span::styled(prefix.to_string(), base)];
    let mut rest = text;
    let mut code_mode = false;
    while let Some(idx) = rest.find('`') {
        let (head, tail) = rest.split_at(idx);
        if !head.is_empty() {
            if code_mode {
                spans.push(Span::styled(head.to_string(), code));
            } else {
                append_emphasis_spans(&mut spans, head, base, strong, italic);
            }
        }
        rest = &tail[1..];
        code_mode = !code_mode;
    }
    if !rest.is_empty() {
        if code_mode {
            spans.push(Span::styled(rest.to_string(), code));
        } else {
            append_emphasis_spans(&mut spans, rest, base, strong, italic);
        }
    }
    out.push(Line::from(spans));
}

fn append_emphasis_spans(
    spans: &mut Vec<Span<'static>>,
    input: &str,
    base: Style,
    strong: Style,
    italic: Style,
) {
    let mut i = 0;
    let bytes = input.as_bytes();
    let mut strong_on = false;
    let mut italic_on = false;
    let mut start = 0;
    while i < bytes.len() {
        let mut marker_len = 0;
        if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'*' {
            marker_len = 2;
        } else if bytes[i] == b'*' {
            marker_len = 1;
        }
        if marker_len == 0 {
            i += 1;
            continue;
        }
        if start < i {
            let part = &input[start..i];
            let style = if strong_on && italic_on {
                strong.add_modifier(Modifier::ITALIC)
            } else if strong_on {
                strong
            } else if italic_on {
                italic
            } else {
                base
            };
            spans.push(Span::styled(part.to_string(), style));
        }
        if marker_len == 2 {
            strong_on = !strong_on;
        } else {
            italic_on = !italic_on;
        }
        i += marker_len;
        start = i;
    }
    if start < input.len() {
        let part = &input[start..];
        let style = if strong_on && italic_on {
            strong.add_modifier(Modifier::ITALIC)
        } else if strong_on {
            strong
        } else if italic_on {
            italic
        } else {
            base
        };
        spans.push(Span::styled(part.to_string(), style));
    }
}

fn render_code_block(
    out: &mut Vec<Line<'static>>,
    lines: &[String],
    lang: &str,
    palette: UiPalette,
) {
    out.push(Line::from(Span::styled(
        format!("  ```{}", lang),
        Style::default().fg(palette.muted),
    )));
    for raw in lines {
        let spans = vec![
            Span::styled("  ".to_string(), Style::default().fg(palette.muted)),
            Span::styled(
                raw.to_string(),
                Style::default()
                    .fg(palette.accent_alt)
                    .bg(palette.selected_bg),
            ),
        ];
        out.push(Line::from(spans));
        if raw.is_empty() {
            out.push(Line::from(Span::styled(
                "  ".to_string(),
                Style::default().fg(palette.muted),
            )));
        }
    }
    out.push(Line::from(Span::styled(
        "  ```".to_string(),
        Style::default().fg(palette.muted),
    )));
}

fn render_chat_message(
    out: &mut Vec<Line<'static>>,
    role: ChatRole,
    message: &str,
    palette: UiPalette,
) {
    let base = role_style(role, palette);
    let code_inline = Style::default()
        .fg(palette.accent_alt)
        .bg(palette.selected_bg)
        .add_modifier(Modifier::BOLD);
    let strong = base.add_modifier(Modifier::BOLD);
    let italic = base.add_modifier(Modifier::ITALIC);
    let mut in_code = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();

    for raw in message.split('\n') {
        let trimmed = raw.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if !in_code {
                in_code = true;
                code_lang = rest.trim().to_string();
                code_lines.clear();
            } else {
                render_code_block(out, &code_lines, code_lang.as_str(), palette);
                in_code = false;
                code_lang.clear();
                code_lines.clear();
            }
            continue;
        }

        if in_code {
            code_lines.push(raw.to_string());
            continue;
        }

        if role == ChatRole::Meta || role == ChatRole::System {
            push_with_inline_code(out, "  ", raw, base, code_inline, strong, italic);
            continue;
        }

        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = trimmed[level..].trim_start();
            out.push(Line::from(vec![
                Span::styled("  ".to_string(), base),
                Span::styled(
                    format!("H{} ", level.min(6)),
                    Style::default()
                        .fg(palette.accent_alt)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(title.to_string(), strong),
            ]));
            continue;
        }

        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            push_with_inline_code(out, "  • ", item, base, code_inline, strong, italic);
            continue;
        }

        if let Some(item) = trimmed.strip_prefix("• ") {
            push_with_inline_code(out, "  • ", item, base, code_inline, strong, italic);
            continue;
        }

        if let Some(quote) = trimmed.strip_prefix("> ") {
            push_with_inline_code(
                out,
                "  │ ",
                quote,
                Style::default().fg(palette.muted),
                code_inline,
                strong,
                italic,
            );
            continue;
        }

        if let Some(dot) = trimmed.find(". ") {
            if trimmed[..dot].chars().all(|c| c.is_ascii_digit()) {
                let (n, text) = trimmed.split_at(dot + 2);
                let mut spans = vec![Span::styled(
                    "  ".to_string(),
                    Style::default().fg(palette.accent_alt),
                )];
                spans.push(Span::styled(
                    n.to_string(),
                    Style::default()
                        .fg(palette.accent_alt)
                        .add_modifier(Modifier::BOLD),
                ));
                append_emphasis_spans(&mut spans, text, base, strong, italic);
                out.push(Line::from(spans));
                continue;
            }
        }

        push_with_inline_code(out, "  ", raw, base, code_inline, strong, italic);
    }

    if in_code {
        render_code_block(out, &code_lines, code_lang.as_str(), palette);
    }
}

fn build_chat_lines(state: &ShellState, palette: UiPalette) -> Vec<Line<'static>> {
    let filter = state.selection.log_search.trim().to_ascii_lowercase();
    let mut grouped: Vec<(ChatRole, Vec<String>)> = Vec::new();
    for entry in state.artifacts.logs.iter().filter(|l| {
        l.source == dao_core::state::LogSource::Shell
            || l.source == dao_core::state::LogSource::Runtime
    }) {
        let (role, text) = parse_chat_role(&entry.message);
        if !message_matches_filter(&text, &filter) {
            continue;
        }
        if let Some((last_role, lines)) = grouped.last_mut() {
            if *last_role == role {
                lines.push(text);
                continue;
            }
        }
        grouped.push((role, vec![text]));
    }

    let mut out = Vec::new();
    for (role, messages) in grouped {
        let (label, color) = match role {
            ChatRole::User => ("[You]", palette.accent),
            ChatRole::Assistant => ("[Assistant]", palette.success),
            ChatRole::Meta => ("[Meta]", palette.muted),
            ChatRole::System => ("[System]", palette.warning),
        };
        out.push(Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));

        for message in messages {
            render_chat_message(&mut out, role, &message, palette);
            out.push(Line::from(""));
        }
    }

    if state.interaction.is_thinking && !state.interaction.live_assistant_preview.is_empty() {
        out.push(Line::from(Span::styled(
            format!("[Assistant {}]", get_spinner()),
            Style::default()
                .fg(palette.accent_alt)
                .add_modifier(Modifier::BOLD),
        )));
        render_chat_message(
            &mut out,
            ChatRole::Assistant,
            &state.interaction.live_assistant_preview,
            palette,
        );
        out.push(Line::from(""));
    }

    out
}

#[derive(Clone, Copy)]
struct UiPalette {
    accent: Color,
    accent_alt: Color,
    success: Color,
    warning: Color,
    danger: Color,
    muted: Color,
    border: Color,
    panel_bg: Color,
    selected_bg: Color,
}

fn palette_for(theme: UiTheme) -> UiPalette {
    match theme {
        UiTheme::Classic => UiPalette {
            accent: Color::Cyan,
            accent_alt: Color::Blue,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            muted: Color::DarkGray,
            border: Color::Gray,
            panel_bg: Color::Black,
            selected_bg: Color::DarkGray,
        },
        UiTheme::Cyberpunk => UiPalette {
            accent: Color::Magenta,
            accent_alt: Color::Cyan,
            success: Color::LightGreen,
            warning: Color::LightYellow,
            danger: Color::LightRed,
            muted: Color::Gray,
            border: Color::Magenta,
            panel_bg: Color::Black,
            selected_bg: Color::Rgb(58, 0, 58),
        },
        UiTheme::NeonNoir => UiPalette {
            accent: Color::LightBlue,
            accent_alt: Color::LightCyan,
            success: Color::LightGreen,
            warning: Color::Yellow,
            danger: Color::LightRed,
            muted: Color::Gray,
            border: Color::LightBlue,
            panel_bg: Color::Black,
            selected_bg: Color::Rgb(18, 28, 42),
        },
        UiTheme::SolarFlare => UiPalette {
            accent: Color::LightYellow,
            accent_alt: Color::LightRed,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            muted: Color::Gray,
            border: Color::Yellow,
            panel_bg: Color::Black,
            selected_bg: Color::Rgb(42, 28, 0),
        },
        UiTheme::ForestZen => UiPalette {
            accent: Color::LightGreen,
            accent_alt: Color::Green,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            muted: Color::Gray,
            border: Color::LightGreen,
            panel_bg: Color::Black,
            selected_bg: Color::Rgb(8, 32, 10),
        },
    }
}

fn syntect_theme_name(theme: UiTheme) -> &'static str {
    match theme {
        UiTheme::Classic => "base16-ocean.dark",
        UiTheme::Cyberpunk => "base16-eighties.dark",
        UiTheme::NeonNoir => "base16-mocha.dark",
        UiTheme::SolarFlare => "base16-ocean.dark",
        UiTheme::ForestZen => "base16-ocean.dark",
    }
}

fn tab_by_index(state: &ShellState, one_based_index: usize) -> Option<ShellTab> {
    if one_based_index == 0 {
        return None;
    }
    state.ordered_tabs().get(one_based_index - 1).copied()
}

fn resolve_main_content_area(state: &ShellState, content_area: Rect) -> Rect {
    let show_journey = !state.customization.focus_mode && state.customization.show_journey;
    let show_context = !state.customization.focus_mode && state.customization.show_overview;
    if !show_journey && !show_context {
        return content_area;
    }

    let mut cols = Vec::new();
    if show_journey {
        cols.push(Constraint::Length(28));
    }
    cols.push(Constraint::Min(0));
    if show_context {
        cols.push(Constraint::Length(34));
    }
    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(cols)
        .split(content_area);

    let mut idx = 0_usize;
    if show_journey {
        idx += 1;
    }
    sections[idx]
}

fn plan_step_id_at_row(state: &ShellState, main_area: Rect, row: u16) -> Option<String> {
    let plan = state.artifacts.plan.as_ref()?;
    if main_area.height < 3 {
        return None;
    }
    let mut y = main_area.y.saturating_add(1);
    let max_y = main_area.y + main_area.height.saturating_sub(1);
    for step in &plan.steps {
        if y >= max_y {
            break;
        }
        if row == y {
            return Some(step.id.clone());
        }
        y = y.saturating_add(1);
        if state.selection.expanded_plan_steps.contains(&step.id) {
            y = y.saturating_add(2);
        }
    }
    None
}

fn content_height<B: Backend>(state: &ShellState, terminal: &Terminal<B>) -> io::Result<u16> {
    let (header_h, tabs_h) = if state.customization.focus_mode {
        (0, 0)
    } else {
        (3, 3)
    };
    let action_h = if !state.customization.focus_mode && state.customization.show_action_bar {
        2
    } else {
        0
    };
    let term_height = terminal.size()?.height;
    let layout_deduction = 2 + header_h + tabs_h + state.customization.input_height + action_h + 1;
    Ok(term_height
        .saturating_sub(layout_deduction)
        .saturating_sub(2))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn push_sample(history: &mut Vec<u64>, value: u64, cap: usize) {
    if history.len() >= cap {
        history.remove(0);
    }
    history.push(value);
}

fn command_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn parse_macos_cpu_percent() -> Option<f32> {
    let out = command_stdout("top", &["-l", "1", "-n", "0"])?;
    let cpu_line = out.lines().find(|l| l.contains("CPU usage:"))?;
    let idle_chunk = cpu_line
        .split(',')
        .find(|c| c.to_ascii_lowercase().contains("idle"))?;
    let idle_pct = idle_chunk
        .split('%')
        .next()?
        .split_whitespace()
        .last()?
        .parse::<f32>()
        .ok()?;
    Some((100.0 - idle_pct).clamp(0.0, 100.0))
}

fn parse_macos_memory_mb() -> Option<(u64, u64)> {
    let total_bytes = command_stdout("sysctl", &["-n", "hw.memsize"])?
        .parse::<u64>()
        .ok()?;
    let vm = command_stdout("vm_stat", &[])?;
    let page_size = vm
        .lines()
        .next()
        .and_then(|l| l.split("page size of ").nth(1))
        .and_then(|s| s.split(" bytes").next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(4096);
    let mut active = 0_u64;
    let mut wired = 0_u64;
    let mut compressed = 0_u64;
    for line in vm.lines() {
        let value = line
            .split(':')
            .nth(1)
            .map(|v| v.trim().trim_end_matches('.'))
            .and_then(|v| v.replace('.', "").parse::<u64>().ok())
            .unwrap_or(0);
        if line.starts_with("Pages active") {
            active = value;
        } else if line.starts_with("Pages wired down") {
            wired = value;
        } else if line.starts_with("Pages occupied by compressor") {
            compressed = value;
        }
    }
    let used_bytes = (active + wired + compressed).saturating_mul(page_size);
    Some((used_bytes / (1024 * 1024), total_bytes / (1024 * 1024)))
}

fn parse_process_mem_mb() -> Option<u64> {
    let pid = std::process::id().to_string();
    let out = command_stdout("ps", &["-o", "rss=", "-p", &pid])?;
    let kb = out.trim().parse::<u64>().ok()?;
    Some(kb / 1024)
}

fn update_system_telemetry(state: &mut ShellState) {
    let cpu = parse_macos_cpu_percent().unwrap_or(state.telemetry.latest.cpu_percent);
    let (mem_used_mb, mem_total_mb) = parse_macos_memory_mb().unwrap_or((
        state.telemetry.latest.mem_used_mb,
        state.telemetry.latest.mem_total_mb.max(1),
    ));
    let process_mem_mb = parse_process_mem_mb().unwrap_or(state.telemetry.latest.process_mem_mb);
    let mem_ratio = if mem_total_mb == 0 {
        0.0
    } else {
        (mem_used_mb as f64 / mem_total_mb as f64).clamp(0.0, 1.0)
    };

    state.telemetry.latest.cpu_percent = cpu;
    state.telemetry.latest.mem_used_mb = mem_used_mb;
    state.telemetry.latest.mem_total_mb = mem_total_mb;
    state.telemetry.latest.process_mem_mb = process_mem_mb;
    state.telemetry.latest.sample_ts_ms = Some(now_ms());

    push_sample(&mut state.telemetry.cpu_history, cpu.round() as u64, 240);
    push_sample(
        &mut state.telemetry.mem_history,
        (mem_ratio * 100.0).round() as u64,
        240,
    );
}

#[cfg(target_os = "macos")]
fn parse_ioreg_perf_u64(line: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{}\"=", key);
    let start = line.find(&pattern)? + pattern.len();
    let tail = &line[start..];
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

#[cfg(target_os = "macos")]
fn update_gpu_telemetry(state: &mut ShellState) {
    let out = command_stdout(
        "ioreg",
        &["-r", "-d", "1", "-w", "0", "-c", "AGXAccelerator"],
    );
    if let Some(text) = out {
        if let Some(line) = text
            .lines()
            .find(|l| l.contains("\"PerformanceStatistics\""))
        {
            let util = parse_ioreg_perf_u64(line, "Device Utilization %").map(|v| v as f32);
            let used_bytes = parse_ioreg_perf_u64(line, "In use system memory")
                .or_else(|| parse_ioreg_perf_u64(line, "In use system memory (driver)"));
            let used_mb = used_bytes.map(|v| v / (1024 * 1024));

            if util.is_some() || used_mb.is_some() {
                state.telemetry.latest.gpu_util_percent = util.map(|v| v.clamp(0.0, 100.0));
                state.telemetry.latest.gpu_mem_used_mb = used_mb;
                state.telemetry.latest.gpu_mem_total_mb = None;
                state.telemetry.latest.gpu_status = Some(
                    "Live (safe ioreg; unified memory total is not per-GPU on Apple Silicon)"
                        .to_string(),
                );
                return;
            }
        }
    }

    let detected = command_stdout("system_profiler", &["SPDisplaysDataType"])
        .map(|s| s.contains("Type: GPU") || s.contains("Chipset Model:"))
        .unwrap_or(false);

    state.telemetry.latest.gpu_util_percent = None;
    state.telemetry.latest.gpu_mem_used_mb = None;
    state.telemetry.latest.gpu_mem_total_mb = None;
    state.telemetry.latest.gpu_status = if detected {
        Some("Limited (GPU detected; live counters unavailable in current environment)".to_string())
    } else {
        Some("N/A (unsupported)".to_string())
    };
}

#[cfg(target_os = "windows")]
fn update_gpu_telemetry(state: &mut ShellState) {
    let util_out = command_stdout(
        "cmd",
        &[
            "/C",
            "typeperf \"\\\\GPU Engine(*)\\\\Utilization Percentage\" -sc 1",
        ],
    );
    let mut util = None;
    if let Some(text) = util_out {
        let mut max_util = 0.0_f32;
        for token in text.split(',') {
            let token = token.trim().trim_matches('"');
            if let Ok(v) = token.parse::<f32>() {
                if v.is_finite() {
                    max_util = max_util.max(v);
                }
            }
        }
        if max_util > 0.0 {
            util = Some(max_util.clamp(0.0, 100.0));
        }
    }

    let used_out = command_stdout(
        "cmd",
        &[
            "/C",
            "typeperf \"\\\\GPU Adapter Memory(*)\\\\Dedicated Usage\" -sc 1",
        ],
    );
    let mut used_mb = None;
    if let Some(text) = used_out {
        let mut max_bytes = 0_u64;
        for token in text.split(',') {
            let token = token.trim().trim_matches('"');
            if let Ok(v) = token.parse::<f64>() {
                if v.is_finite() && v > 0.0 {
                    max_bytes = max_bytes.max(v as u64);
                }
            }
        }
        if max_bytes > 0 {
            used_mb = Some(max_bytes / (1024 * 1024));
        }
    }

    let total_out = command_stdout(
        "cmd",
        &[
            "/C",
            "wmic path win32_VideoController get AdapterRAM /value",
        ],
    );
    let mut total_mb = None;
    if let Some(text) = total_out {
        let mut max_bytes = 0_u64;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("AdapterRAM=") {
                if let Ok(v) = value.trim().parse::<u64>() {
                    max_bytes = max_bytes.max(v);
                }
            }
        }
        if max_bytes > 0 {
            total_mb = Some(max_bytes / (1024 * 1024));
        }
    }

    state.telemetry.latest.gpu_util_percent = util;
    state.telemetry.latest.gpu_mem_used_mb = used_mb;
    state.telemetry.latest.gpu_mem_total_mb = total_mb;
    state.telemetry.latest.gpu_status = if util.is_some() || total_mb.is_some() {
        Some("Live".to_string())
    } else {
        Some("N/A (unsupported on this Windows host)".to_string())
    };
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn update_gpu_telemetry(state: &mut ShellState) {
    state.telemetry.latest.gpu_util_percent = None;
    state.telemetry.latest.gpu_mem_used_mb = None;
    state.telemetry.latest.gpu_mem_total_mb = None;
    state.telemetry.latest.gpu_status = Some("N/A (unsupported on this OS)".to_string());
}

enum KeyHandlerResult {
    Continue(Vec<DaoEffect>),
    Exit,
}

fn handle_confirm_reset_keys(key: event::KeyEvent, state: &mut ShellState) -> KeyHandlerResult {
    let effects = match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            reduce(state, ShellAction::User(UserAction::ConfirmReset))
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            reduce(state, ShellAction::User(UserAction::CancelReset))
        }
        _ => Vec::new(),
    };
    KeyHandlerResult::Continue(effects)
}

fn handle_help_keys(key: event::KeyEvent, state: &mut ShellState) -> KeyHandlerResult {
    let effects = match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            reduce(state, ShellAction::User(UserAction::CloseOverlay))
        }
        _ => Vec::new(),
    };
    KeyHandlerResult::Continue(effects)
}

fn handle_action_palette_keys(key: event::KeyEvent, state: &mut ShellState) -> KeyHandlerResult {
    let effects = match key.code {
        KeyCode::Esc => reduce(state, ShellAction::User(UserAction::CloseOverlay)),
        KeyCode::Up => reduce(state, ShellAction::User(UserAction::OverlayMoveUp)),
        KeyCode::Down => reduce(state, ShellAction::User(UserAction::OverlayMoveDown)),
        KeyCode::Enter => reduce(state, ShellAction::User(UserAction::OverlaySubmit)),
        KeyCode::Backspace => reduce(state, ShellAction::User(UserAction::OverlayQueryBackspace)),
        KeyCode::Char(c) => reduce(state, ShellAction::User(UserAction::OverlayQueryInput(c))),
        _ => Vec::new(),
    };
    KeyHandlerResult::Continue(effects)
}

fn handle_model_selection_keys(key: event::KeyEvent, state: &mut ShellState) -> KeyHandlerResult {
    let effects = match key.code {
        KeyCode::Esc => reduce(state, ShellAction::User(UserAction::CloseOverlay)),
        KeyCode::Up => reduce(state, ShellAction::User(UserAction::ModelListMoveUp)),
        KeyCode::Down => reduce(state, ShellAction::User(UserAction::ModelListMoveDown)),
        KeyCode::Enter => reduce(state, ShellAction::User(UserAction::ModelListSubmit)),
        _ => Vec::new(),
    };
    KeyHandlerResult::Continue(effects)
}

fn handle_chat_focus_keys(key: event::KeyEvent, state: &mut ShellState) -> KeyHandlerResult {
    let effects = match key.code {
        KeyCode::Esc => reduce(state, ShellAction::User(UserAction::SetChatFocus(false))),
        KeyCode::Enter => reduce(state, ShellAction::User(UserAction::ChatSubmit)),
        KeyCode::Backspace => reduce(state, ShellAction::User(UserAction::ChatBackspace)),
        KeyCode::Char(c) => reduce(state, ShellAction::User(UserAction::ChatInput(c))),
        KeyCode::Up => reduce(state, ShellAction::User(UserAction::ChatHistoryUp)),
        KeyCode::Down => reduce(state, ShellAction::User(UserAction::ChatHistoryDown)),
        _ => Vec::new(),
    };
    KeyHandlerResult::Continue(effects)
}

fn handle_global_keys<B: Backend>(
    key: event::KeyEvent,
    state: &mut ShellState,
    terminal: &mut Terminal<B>,
) -> io::Result<KeyHandlerResult> {
    let mut effects = Vec::new();

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Up {
        effects.extend(reduce(state, ShellAction::User(UserAction::ResizeInput(1))));
        return Ok(KeyHandlerResult::Continue(effects));
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Down {
        effects.extend(reduce(
            state,
            ShellAction::User(UserAction::ResizeInput(-1)),
        ));
        return Ok(KeyHandlerResult::Continue(effects));
    }

    match key.code {
        KeyCode::Char('/') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ToggleActionPalette),
            ));
        }
        KeyCode::Char('q') => return Ok(KeyHandlerResult::Exit),
        KeyCode::Char('i') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::SetChatFocus(true)),
            ));
        }
        KeyCode::Char('z') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ToggleFocusMode),
            ));
        }
        KeyCode::Char('[') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::SetTheme(state.customization.theme.prev())),
            ));
        }
        KeyCode::Char(']') => {
            effects.extend(reduce(state, ShellAction::User(UserAction::CycleTheme)));
        }
        KeyCode::Char('j') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ToggleJourneyPanel),
            ));
        }
        KeyCode::Char('o') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ToggleOverviewPanel),
            ));
        }
        KeyCode::Char('a') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ToggleActionBar),
            ));
        }
        KeyCode::Char('+') | KeyCode::Char('=') => {
            effects.extend(reduce(state, ShellAction::User(UserAction::ResizeInput(1))));
        }
        KeyCode::Char('-') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ResizeInput(-1)),
            ));
        }
        KeyCode::Char('r') => {
            effects.extend(reduce(state, ShellAction::User(UserAction::ResetSession)));
        }
        KeyCode::Char('v') => {
            effects.extend(reduce(state, ShellAction::User(UserAction::ReviewChanges)));
        }
        KeyCode::Char('?') => {
            effects.extend(reduce(state, ShellAction::User(UserAction::ShowHelp)));
        }
        KeyCode::Right | KeyCode::Tab => {
            effects.extend(reduce(state, ShellAction::User(UserAction::NextTab)));
        }
        KeyCode::Left => {
            effects.extend(reduce(state, ShellAction::User(UserAction::PrevTab)));
        }
        KeyCode::Up => {
            if state.routing.tab == ShellTab::Plan {
                effects.extend(reduce(state, ShellAction::User(UserAction::PlanStepUp)));
            } else if state.routing.tab == ShellTab::FileBrowser {
                effects.extend(reduce(state, ShellAction::User(UserAction::FileBrowserUp)));
            } else if (state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat)
                && state.selection.log_stick_to_bottom
            {
                let content_area_h = content_height(state, terminal)?;
                let log_count = if state.routing.tab == ShellTab::Chat {
                    chat_line_count(state)
                } else {
                    let filter = state.selection.log_level_filter;
                    state
                        .artifacts
                        .logs
                        .iter()
                        .filter(|l| filter.map_or(true, |f| l.level >= f))
                        .count()
                };
                let current_scroll = (log_count as u16).saturating_sub(content_area_h);
                let new_scroll = current_scroll.saturating_sub(3);
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogScroll(new_scroll)),
                ));
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogStickToBottom(false)),
                ));
            } else {
                effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(-3))));
            }
        }
        KeyCode::Down => {
            if state.routing.tab == ShellTab::Plan {
                effects.extend(reduce(state, ShellAction::User(UserAction::PlanStepDown)));
            } else if state.routing.tab == ShellTab::FileBrowser {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::FileBrowserDown),
                ));
            } else if state.routing.tab == ShellTab::Diff || state.routing.tab == ShellTab::Explain
            {
                effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(3))));
            } else if (state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat)
                && !state.selection.log_stick_to_bottom
            {
                effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(3))));
            }
        }
        KeyCode::Enter => {
            if state.routing.tab == ShellTab::FileBrowser {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::FileBrowserEnter),
                ));
            }
        }
        KeyCode::Backspace => {
            if state.routing.tab == ShellTab::FileBrowser {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::FileBrowserBack),
                ));
            }
        }
        KeyCode::Char(' ') => {
            if state.routing.tab == ShellTab::Plan {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::TogglePlanStepExpansion),
                ));
            } else if (state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat)
                && !state.selection.log_stick_to_bottom
            {
                effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(3))));
            }
        }
        KeyCode::PageUp => {
            if state.routing.tab == ShellTab::Plan {
                effects.extend(reduce(state, ShellAction::User(UserAction::PlanStepPageUp)));
            } else if (state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat)
                && state.selection.log_stick_to_bottom
            {
                let content_area_h = content_height(state, terminal)?;
                let log_count = if state.routing.tab == ShellTab::Chat {
                    chat_line_count(state)
                } else {
                    let filter = state.selection.log_level_filter;
                    state
                        .artifacts
                        .logs
                        .iter()
                        .filter(|l| filter.map_or(true, |f| l.level >= f))
                        .count()
                };
                let current_scroll = (log_count as u16).saturating_sub(content_area_h);
                let new_scroll = current_scroll.saturating_sub(10);
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogScroll(new_scroll)),
                ));
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogStickToBottom(false)),
                ));
            } else {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::ScrollLogs(-10)),
                ));
            }
        }
        KeyCode::PageDown => {
            if state.routing.tab == ShellTab::Plan {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::PlanStepPageDown),
                ));
            } else if state.routing.tab == ShellTab::Diff || state.routing.tab == ShellTab::Explain
            {
                effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(10))));
            } else if (state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat)
                && !state.selection.log_stick_to_bottom
            {
                effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(10))));
            }
        }
        KeyCode::Home => {
            if state.routing.tab == ShellTab::Logs
                || state.routing.tab == ShellTab::Chat
                || state.routing.tab == ShellTab::Diff
                || state.routing.tab == ShellTab::Explain
            {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogScroll(0)),
                ));
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogStickToBottom(false)),
                ));
            }
        }
        KeyCode::End => {
            if state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogStickToBottom(true)),
                ));
            } else if state.routing.tab == ShellTab::Diff || state.routing.tab == ShellTab::Explain
            {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogScroll(u16::MAX)),
                ));
            }
        }
        KeyCode::Char('G') => {
            if state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogStickToBottom(true)),
                ));
            } else if state.routing.tab == ShellTab::Diff || state.routing.tab == ShellTab::Explain
            {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogScroll(u16::MAX)),
                ));
            }
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::ToggleActionPalette),
            ));
        }
        KeyCode::Char('y') => {
            if state.routing.tab == ShellTab::Diff {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::CopyDiffToClipboard),
                ));
            }
        }
        KeyCode::Char('s') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::SelectTab(ShellTab::System)),
            ));
        }
        KeyCode::Char('t') => {
            effects.extend(reduce(
                state,
                ShellAction::User(UserAction::SelectTab(ShellTab::Telemetry)),
            ));
        }
        KeyCode::Char('1') => {
            if let Some(tab) = tab_by_index(state, 1) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('2') => {
            if let Some(tab) = tab_by_index(state, 2) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('3') => {
            if let Some(tab) = tab_by_index(state, 3) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('4') => {
            if let Some(tab) = tab_by_index(state, 4) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('5') => {
            if let Some(tab) = tab_by_index(state, 5) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('6') => {
            if let Some(tab) = tab_by_index(state, 6) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('7') => {
            if let Some(tab) = tab_by_index(state, 7) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('8') => {
            if let Some(tab) = tab_by_index(state, 8) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('9') => {
            if let Some(tab) = tab_by_index(state, 9) {
                effects.extend(reduce(state, ShellAction::User(UserAction::SelectTab(tab))));
            }
        }
        KeyCode::Char('f') => {
            if state.routing.tab == ShellTab::Plan {
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetPlanStickToRunning(
                        !state.selection.plan_stick_to_running,
                    )),
                ));
            } else {
                let next = match state.selection.log_level_filter {
                    None => Some(LogLevel::Info),
                    Some(LogLevel::Info) => Some(LogLevel::Warn),
                    Some(LogLevel::Warn) => Some(LogLevel::Error),
                    Some(LogLevel::Error) => Some(LogLevel::Debug),
                    Some(LogLevel::Debug) => Some(LogLevel::Trace),
                    Some(LogLevel::Trace) => None,
                };
                effects.extend(reduce(
                    state,
                    ShellAction::User(UserAction::SetLogLevelFilter(next)),
                ));
            }
        }
        _ => {}
    }
    Ok(KeyHandlerResult::Continue(effects))
}

fn handle_key_event<B: Backend>(
    key: event::KeyEvent,
    state: &mut ShellState,
    terminal: &mut Terminal<B>,
) -> io::Result<KeyHandlerResult> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(KeyHandlerResult::Exit);
    }

    match &state.interaction.overlay {
        ShellOverlay::ConfirmReset => Ok(handle_confirm_reset_keys(key, state)),
        ShellOverlay::Help => Ok(handle_help_keys(key, state)),
        ShellOverlay::ActionPalette { .. } => Ok(handle_action_palette_keys(key, state)),
        ShellOverlay::ModelSelection { .. } => Ok(handle_model_selection_keys(key, state)),
        ShellOverlay::None => {
            if state.interaction.focus_in_chat {
                Ok(handle_chat_focus_keys(key, state))
            } else {
                handle_global_keys(key, state, terminal)
            }
        }
        _ => Ok(KeyHandlerResult::Continue(Vec::new())),
    }
}

fn handle_mouse_event<B: Backend>(
    mouse: event::MouseEvent,
    state: &mut ShellState,
    terminal: &mut Terminal<B>,
) -> io::Result<Vec<DaoEffect>> {
    let mut effects = Vec::new();
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Ok(size) = terminal.size() {
                let rect = Rect::new(0, 0, size.width, size.height);
                let (header_h, tabs_h) = if state.customization.focus_mode {
                    (0, 0)
                } else {
                    (3, 3)
                };
                let action_bar_h =
                    if !state.customization.focus_mode && state.customization.show_action_bar {
                        2
                    } else {
                        0
                    };
                let mut constraints = vec![
                    Constraint::Length(header_h),
                    Constraint::Length(tabs_h),
                    Constraint::Min(0),
                    Constraint::Length(state.customization.input_height),
                ];
                if action_bar_h > 0 {
                    constraints.push(Constraint::Length(action_bar_h));
                }
                constraints.push(Constraint::Length(1));
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(1)
                    .constraints(constraints)
                    .split(rect);

                let tabs_area = chunks[1];
                if mouse.row >= tabs_area.y && mouse.row < tabs_area.y + tabs_area.height {
                    let tabs = state.ordered_tabs();
                    let mut current_x = tabs_area.x + 1; // +1 for border
                    for tab in tabs {
                        let label = tab.label();
                        let width = label.len() as u16;
                        if mouse.column >= current_x && mouse.column < current_x + width {
                            effects.extend(reduce(
                                state,
                                ShellAction::User(UserAction::SelectTab(*tab)),
                            ));
                            break;
                        }
                        // Separator " | " is 3 chars
                        current_x += width + 3;
                    }
                }

                let input_area = chunks[3];
                if mouse.row >= input_area.y
                    && mouse.row < input_area.y + input_area.height
                    && mouse.column >= input_area.x
                    && mouse.column < input_area.x + input_area.width
                {
                    effects.extend(reduce(
                        state,
                        ShellAction::User(UserAction::SetChatFocus(true)),
                    ));
                } else if state.interaction.focus_in_chat {
                    effects.extend(reduce(
                        state,
                        ShellAction::User(UserAction::SetChatFocus(false)),
                    ));
                }

                let content_area = chunks[2];
                let main_area = resolve_main_content_area(state, content_area);
                let in_main = mouse.row >= main_area.y
                    && mouse.row < main_area.y + main_area.height
                    && mouse.column >= main_area.x
                    && mouse.column < main_area.x + main_area.width;
                if in_main && state.routing.tab == ShellTab::Plan {
                    if let Some(step_id) = plan_step_id_at_row(state, main_area, mouse.row) {
                        effects.extend(reduce(
                            state,
                            ShellAction::User(UserAction::SelectPlanStep { id: step_id }),
                        ));
                    }
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if state.routing.tab == ShellTab::Chat
                || state.routing.tab == ShellTab::Logs
                || state.routing.tab == ShellTab::Diff
                || state.routing.tab == ShellTab::Explain
            {
                if state.routing.tab == ShellTab::Diff
                    || state.routing.tab == ShellTab::Explain
                    || !state.selection.log_stick_to_bottom
                {
                    effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(3))));
                }
            }
        }
        MouseEventKind::ScrollUp => {
            if state.routing.tab == ShellTab::Chat
                || state.routing.tab == ShellTab::Logs
                || state.routing.tab == ShellTab::Diff
                || state.routing.tab == ShellTab::Explain
            {
                if (state.routing.tab == ShellTab::Logs || state.routing.tab == ShellTab::Chat)
                    && state.selection.log_stick_to_bottom
                {
                    let content_area_h = content_height(state, terminal)?;
                    let log_count = if state.routing.tab == ShellTab::Chat {
                        chat_line_count(state)
                    } else {
                        let filter = state.selection.log_level_filter;
                        state
                            .artifacts
                            .logs
                            .iter()
                            .filter(|l| filter.map_or(true, |f| l.level >= f))
                            .count()
                    };
                    let current_scroll = (log_count as u16).saturating_sub(content_area_h);
                    let new_scroll = current_scroll.saturating_sub(3);
                    effects.extend(reduce(
                        state,
                        ShellAction::User(UserAction::SetLogScroll(new_scroll)),
                    ));
                    effects.extend(reduce(
                        state,
                        ShellAction::User(UserAction::SetLogStickToBottom(false)),
                    ));
                } else {
                    effects.extend(reduce(state, ShellAction::User(UserAction::ScrollLogs(-3))));
                }
            }
        }
        _ => {}
    }
    Ok(effects)
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    state: &mut ShellState,
    repo: &Path,
) -> io::Result<()> {
    let state_path = repo.join(".dao/state.json");
    let mut last_mod = fs::metadata(&state_path).and_then(|m| m.modified()).ok();
    let (tx, rx) = mpsc::channel();
    let mut last_sample = Instant::now()
        .checked_sub(Duration::from_millis(1500))
        .unwrap_or_else(Instant::now);
    let mut last_gpu_sample = Instant::now()
        .checked_sub(Duration::from_secs(4))
        .unwrap_or_else(Instant::now);

    loop {
        // Check for external updates to state.json
        if let Ok(metadata) = fs::metadata(&state_path) {
            if let Ok(modified) = metadata.modified() {
                if last_mod != Some(modified) {
                    if let Ok(bytes) = fs::read(&state_path) {
                        if let Ok(new_state) = serde_json::from_slice::<ShellState>(&bytes) {
                            // Preserve interaction state (e.g. chat input) so typing isn't interrupted
                            let interaction = state.interaction.clone();
                            *state = new_state;
                            state.interaction = interaction;
                            last_mod = Some(modified);
                        }
                    }
                }
            }
        }

        // Process background events (chat responses)
        while let Ok(event) = rx.try_recv() {
            match event {
                UiEvent::Token(token) => {
                    if !token.is_empty() {
                        state.interaction.live_assistant_preview.push_str(&token);
                    }
                }
                UiEvent::StreamMeta(line) => {
                    if state.interaction.stream_meta_enabled && !line.trim().is_empty() {
                        reduce(
                            state,
                            ShellAction::Runtime(RuntimeAction::AppendLog(format!(
                                "[meta][stream] {}",
                                line
                            ))),
                        );
                    }
                }
                UiEvent::Finished { elapsed_ms, bytes } => {
                    let final_text = std::mem::take(&mut state.interaction.live_assistant_preview);
                    if !final_text.trim().is_empty() {
                        reduce(
                            state,
                            ShellAction::Runtime(RuntimeAction::AppendLog(format!(
                                "[assistant] {}",
                                final_text
                            ))),
                        );
                    }
                    let tokens = (bytes / 4).max(1) as u64;
                    let tps = if elapsed_ms == 0 {
                        tokens as f32
                    } else {
                        tokens as f32 / (elapsed_ms as f32 / 1000.0)
                    };
                    state.telemetry.latest.tokens_generated = Some(tokens);
                    state.telemetry.latest.tokens_per_second = Some(tps);
                    push_sample(&mut state.telemetry.tps_history, tps.round() as u64, 240);
                    reduce(
                        state,
                        ShellAction::Runtime(RuntimeAction::SetThinking(false)),
                    );
                }
                UiEvent::AuthOutput(line) => {
                    if !line.trim().is_empty() {
                        reduce(
                            state,
                            ShellAction::Runtime(RuntimeAction::AppendLog(format!(
                                "[meta][auth] {}",
                                line
                            ))),
                        );
                    }
                }
                UiEvent::AuthFinished { provider, success } => {
                    let status = if success { "succeeded" } else { "failed" };
                    reduce(
                        state,
                        ShellAction::Runtime(RuntimeAction::AppendLog(format!(
                            "[meta] {} authentication {}",
                            provider, status
                        ))),
                    );
                }
            }
        }

        if last_sample.elapsed() >= Duration::from_millis(1500) {
            update_system_telemetry(state);
            last_sample = Instant::now();
        }
        if last_gpu_sample.elapsed() >= Duration::from_secs(4) {
            update_gpu_telemetry(state);
            last_gpu_sample = Instant::now();
        }

        terminal.draw(|f| ui(f, state))?;

        if event::poll(Duration::from_millis(16))? {
            let mut effects = Vec::new();
            match event::read()? {
                Event::Key(key) => match handle_key_event(key, state, terminal)? {
                    KeyHandlerResult::Continue(e) => {
                        effects.extend(e);
                    }
                    KeyHandlerResult::Exit => return Ok(()),
                },
                Event::Mouse(mouse) => effects.extend(handle_mouse_event(mouse, state, terminal)?),
                _ => {}
            }

            for effect in effects {
                match effect {
                    DaoEffect::SubmitChat { message, context } => {
                        let tx_clone = tx.clone();
                        let provider = resolved_provider(state).to_string();
                        let model = resolved_model_slug(state).to_string();
                        let response_bytes = Arc::new(AtomicUsize::new(0));
                        let response_bytes_clone = Arc::clone(&response_bytes);
                        let started = Instant::now();
                        state.interaction.live_assistant_preview.clear();
                        reduce(
                            state,
                            ShellAction::Runtime(RuntimeAction::AppendLog(format!(
                                "[meta] Backend: {} | Model: {}",
                                provider, model
                            ))),
                        );
                        dao_exec::ShellAdapter::chat_stream(
                            Some(provider.as_str()),
                            Some(model.as_str()),
                            &message,
                            context.as_deref(),
                            move |event| match event {
                                dao_exec::ChatEvent::Token(msg) => {
                                    response_bytes_clone.fetch_add(msg.len(), Ordering::Relaxed);
                                    let _ = tx_clone.send(UiEvent::Token(msg));
                                }
                                dao_exec::ChatEvent::Meta(msg) => {
                                    let _ = tx_clone.send(UiEvent::StreamMeta(msg));
                                }
                                dao_exec::ChatEvent::Done => {
                                    let _ = tx_clone.send(UiEvent::Finished {
                                        elapsed_ms: started.elapsed().as_millis() as u64,
                                        bytes: response_bytes.load(Ordering::Relaxed),
                                    });
                                }
                            },
                        );
                    }
                    DaoEffect::CopyToClipboard(text) => {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(text);
                        }
                    }
                    DaoEffect::StartProviderAuth { provider } => {
                        let tx_clone = tx.clone();
                        std::thread::spawn(move || {
                            let provider_name = provider.to_ascii_lowercase();
                            let mut cmd = if provider_name == "codex" {
                                let mut c = Command::new("codex");
                                c.arg("login").arg("--device-auth");
                                c
                            } else {
                                let _ = tx_clone.send(UiEvent::AuthOutput(format!(
                                    "Unsupported auth provider '{}'",
                                    provider_name
                                )));
                                let _ = tx_clone.send(UiEvent::AuthFinished {
                                    provider: provider_name,
                                    success: false,
                                });
                                return;
                            };

                            let spawn = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
                            let mut child = match spawn {
                                Ok(child) => child,
                                Err(err) => {
                                    let _ = tx_clone.send(UiEvent::AuthOutput(format!(
                                        "Failed to start auth flow: {}",
                                        err
                                    )));
                                    let _ = tx_clone.send(UiEvent::AuthFinished {
                                        provider: provider_name,
                                        success: false,
                                    });
                                    return;
                                }
                            };

                            let _ = tx_clone.send(UiEvent::AuthOutput(
                                "Waiting for provider output... if prompted, open the verification link and enter the shown code/password."
                                    .to_string(),
                            ));

                            let mut workers = Vec::new();
                            if let Some(stdout) = child.stdout.take() {
                                let tx_out = tx_clone.clone();
                                workers.push(std::thread::spawn(move || {
                                    let reader = BufReader::new(stdout);
                                    for line in reader.lines().map_while(|l| l.ok()) {
                                        let _ = tx_out.send(UiEvent::AuthOutput(line));
                                    }
                                }));
                            }
                            if let Some(stderr) = child.stderr.take() {
                                let tx_err = tx_clone.clone();
                                workers.push(std::thread::spawn(move || {
                                    let reader = BufReader::new(stderr);
                                    for line in reader.lines().map_while(|l| l.ok()) {
                                        let _ = tx_err
                                            .send(UiEvent::AuthOutput(format!("stderr: {}", line)));
                                    }
                                }));
                            }

                            let success = child.wait().map(|s| s.success()).unwrap_or(false);
                            for worker in workers {
                                let _ = worker.join();
                            }
                            let _ = tx_clone.send(UiEvent::AuthFinished {
                                provider: provider_name,
                                success,
                            });
                        });
                    }
                    _ => {}
                }
            }
        }
    }
}

fn get_spinner() -> &'static str {
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        / 100) as usize
        % frames.len();
    frames[idx]
}

fn ui(f: &mut ratatui::Frame, state: &ShellState) {
    let palette = palette_for(state.customization.theme);
    let (header_h, tabs_h) = if state.customization.focus_mode {
        (0, 0)
    } else {
        (3, 3)
    };
    let action_bar_h = if !state.customization.focus_mode && state.customization.show_action_bar {
        2
    } else {
        0
    };

    let mut constraints = vec![
        Constraint::Length(header_h),                         // Header
        Constraint::Length(tabs_h),                           // Tabs
        Constraint::Min(0),                                   // Content
        Constraint::Length(state.customization.input_height), // Input
    ];
    if action_bar_h > 0 {
        constraints.push(Constraint::Length(action_bar_h)); // Action bar
    }
    constraints.push(Constraint::Length(1)); // Footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(f.area());
    let content_idx = 2_usize;
    let input_idx = 3_usize;
    let action_idx = if action_bar_h > 0 {
        Some(4_usize)
    } else {
        None
    };
    let footer_idx = if action_bar_h > 0 { 5_usize } else { 4_usize };

    // Header
    let safety = state.header.safety_mode.label();
    let provider = resolved_provider(state);
    let model = resolved_model_slug(state);
    let journey = state.journey_status.state.label();
    let cpu = state.telemetry.latest.cpu_percent.round() as u64;
    let mem_total = state.telemetry.latest.mem_total_mb.max(1);
    let mem_pct = ((state.telemetry.latest.mem_used_mb as f64 / mem_total as f64) * 100.0).round();
    let thinking = if state.interaction.is_thinking {
        format!("{} thinking", get_spinner())
    } else {
        "idle".to_string()
    };
    let header_text = format!(
        "DAO Cockpit | {} | {} | Journey:{} | Provider:{} | Model:{} | CPU:{}% RAM:{}% | Theme:{} | {}",
        state.header.project_name,
        safety,
        journey,
        provider,
        model,
        cpu,
        mem_pct,
        state.customization.theme.label(),
        thinking
    );
    let header = Paragraph::new(header_text)
        .style(Style::default().fg(palette.accent))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border)),
        );
    f.render_widget(header, chunks[0]);

    // Tabs
    let titles: Vec<Line> = state
        .ordered_tabs()
        .iter()
        .map(|t| {
            let label = t.label();
            Line::from(label)
        })
        .collect();

    let selected_tab_index = state
        .ordered_tabs()
        .iter()
        .position(|t| *t == state.routing.tab)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .title("Views"),
        )
        .select(selected_tab_index)
        .highlight_style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, chunks[1]);

    // Content
    let border_style = if state.journey_status.state == JourneyState::Failed {
        Style::default().fg(palette.danger)
    } else {
        Style::default().fg(palette.border)
    };
    let content_block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(palette.panel_bg))
        .border_style(border_style);

    let mut main_area = chunks[content_idx];
    let show_journey = !state.customization.focus_mode && state.customization.show_journey;
    let show_context = !state.customization.focus_mode && state.customization.show_overview;
    if show_journey || show_context {
        let mut cols = Vec::new();
        if show_journey {
            cols.push(Constraint::Length(28));
        }
        cols.push(Constraint::Min(0));
        if show_context {
            cols.push(Constraint::Length(34));
        }
        let sections = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(cols)
            .split(chunks[content_idx]);
        let mut idx = 0_usize;
        if show_journey {
            render_journey_rail(f, sections[idx], state, palette);
            idx += 1;
        }
        main_area = sections[idx];
        if show_context {
            render_context_rail(f, sections[idx + 1], state, palette);
        }
    }

    if state.routing.tab == ShellTab::Chat {
        let chat_lines = build_chat_lines(state, palette);
        let height = main_area.height.saturating_sub(2);
        let content_height = chat_lines.len() as u16;
        let scroll = if state.selection.log_stick_to_bottom {
            content_height.saturating_sub(height)
        } else {
            state
                .selection
                .log_scroll
                .min(content_height.saturating_sub(height))
        };
        let title = if state.selection.log_search.trim().is_empty() {
            format!("Chat ({} lines)", chat_lines.len())
        } else {
            format!(
                "Chat (filter: '{}' | {} lines)",
                state.selection.log_search.trim(),
                chat_lines.len()
            )
        };
        let title = if state.interaction.is_thinking {
            format!("{} | {} streaming", title, get_spinner())
        } else {
            title
        };
        let p = Paragraph::new(chat_lines)
            .block(content_block.title(title))
            .wrap(Wrap { trim: true })
            .scroll((scroll, 0));
        f.render_widget(p, main_area);
    } else if state.routing.tab == ShellTab::Plan {
        if let Some(plan) = &state.artifacts.plan {
            let items: Vec<ListItem> = plan
                .steps
                .iter()
                .map(|s| {
                    let (symbol, color) = match s.status {
                        StepStatus::Pending => ("○", palette.muted),
                        StepStatus::Running => ("➤", palette.warning),
                        StepStatus::Done => ("●", palette.success),
                        StepStatus::Failed => ("✖", palette.danger),
                    };

                    let mut lines = vec![Line::from(vec![
                        Span::styled(format!("{} ", symbol), Style::default().fg(color)),
                        Span::raw(&s.label),
                    ])];

                    if state.selection.expanded_plan_steps.contains(&s.id) {
                        lines.push(Line::from(vec![
                            Span::raw("      "),
                            Span::styled(
                                format!("ID: {}", s.id),
                                Style::default().fg(palette.muted),
                            ),
                        ]));
                        lines.push(Line::from(vec![
                            Span::raw("      "),
                            Span::styled(
                                format!("Status: {:?}", s.status),
                                Style::default().fg(palette.muted),
                            ),
                        ]));
                    }

                    ListItem::new(lines)
                })
                .collect();
            let title = if state.selection.plan_stick_to_running {
                "Plan (Following)"
            } else {
                "Plan"
            };
            let list = List::new(items)
                .block(content_block.title(title))
                .highlight_style(Style::default().bg(palette.selected_bg));

            let selected_index = state
                .selection
                .selected_plan_step
                .as_ref()
                .and_then(|id| plan.steps.iter().position(|s| s.id == *id));
            let mut list_state = ListState::default();
            list_state.select(selected_index);

            f.render_stateful_widget(list, main_area, &mut list_state);
        } else {
            let p = Paragraph::new("No plan artifact.").block(content_block);
            f.render_widget(p, main_area);
        }
    } else if state.routing.tab == ShellTab::Logs {
        let filter = state.selection.log_level_filter;
        let logs: Vec<Line> = state
            .artifacts
            .logs
            .iter()
            .filter(|l| filter.map_or(true, |f| l.level >= f))
            .map(|l| Line::from(format!("[{:?}] {}", l.level, l.message)))
            .collect();
        let title = if let Some(f) = filter {
            format!("Logs (Filter: {:?}+)", f)
        } else {
            "Logs".to_string()
        };
        let scroll = if state.selection.log_stick_to_bottom {
            let height = main_area.height.saturating_sub(2);
            let content_height = logs.len() as u16;
            content_height.saturating_sub(height)
        } else {
            state.selection.log_scroll
        };
        let p = Paragraph::new(logs)
            .block(content_block.title(title))
            .wrap(Wrap { trim: true })
            .scroll((scroll, 0));
        f.render_widget(p, main_area);
    } else if state.routing.tab == ShellTab::Diff {
        if let Some(diff) = &state.artifacts.diff {
            let ps = get_syntax_set();
            let ts = get_theme_set();
            let theme = &ts.themes[syntect_theme_name(state.customization.theme)];
            let mut lines = Vec::new();

            for file in &diff.files {
                lines.push(Line::from(Span::styled(
                    format!("--- {} ({:?})", file.path, file.status),
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(palette.accent_alt),
                )));

                let syntax = ps
                    .find_syntax_for_file(&file.path)
                    .unwrap_or(None)
                    .unwrap_or_else(|| ps.find_syntax_plain_text());
                let mut h = HighlightLines::new(syntax, theme);

                for hunk in &file.hunks {
                    lines.push(Line::from(Span::styled(
                        &hunk.header,
                        Style::default().fg(palette.accent),
                    )));

                    for line in &hunk.lines {
                        let text = &line.text;
                        let (prefix, content) = if !text.is_empty() {
                            (&text[..1], &text[1..])
                        } else {
                            ("", "")
                        };

                        let ranges: Vec<(syntect::highlighting::Style, &str)> =
                            h.highlight_line(content, ps).unwrap_or_default();
                        let mut spans = Vec::new();

                        let prefix_color = match line.kind {
                            DiffLineKind::Add => palette.success,
                            DiffLineKind::Remove => palette.danger,
                            DiffLineKind::Context => palette.muted,
                        };
                        spans.push(Span::styled(prefix, Style::default().fg(prefix_color)));

                        for (style, text) in ranges {
                            let fg = Color::Rgb(
                                style.foreground.r,
                                style.foreground.g,
                                style.foreground.b,
                            );
                            spans.push(Span::styled(text, Style::default().fg(fg)));
                        }
                        lines.push(Line::from(spans));
                    }
                }
            }
            let p = Paragraph::new(lines)
                .block(content_block)
                .wrap(Wrap { trim: false })
                .scroll((state.selection.log_scroll, 0));
            f.render_widget(p, main_area);
        } else {
            let p = Paragraph::new("No diff artifact.").block(content_block);
            f.render_widget(p, main_area);
        }
    } else if state.routing.tab == ShellTab::Overview {
        render_overview(f, main_area, state, palette);
    } else if state.routing.tab == ShellTab::Telemetry {
        render_telemetry(f, main_area, state, palette);
    } else if state.routing.tab == ShellTab::Explain {
        let text = state
            .artifacts
            .logs
            .iter()
            .rev()
            .find(|l| l.context.as_deref() == Some("explain"))
            .map(|l| l.message.as_str())
            .or_else(|| state.artifacts.diff.as_ref().map(|d| d.summary.as_str()))
            .unwrap_or("No explanation available.");
        let p = Paragraph::new(text)
            .block(content_block)
            .wrap(Wrap { trim: true })
            .scroll((state.selection.log_scroll, 0));
        f.render_widget(p, main_area);
    } else if state.routing.tab == ShellTab::System {
        if let Some(sys) = &state.artifacts.system {
            let mut lines = Vec::new();
            lines.push(Line::from(vec![
                Span::styled("Repo Root: ", Style::default().fg(palette.accent)),
                Span::raw(&sys.repo_root),
            ]));
            lines.push(Line::from(""));

            lines.push(Line::from(Span::styled(
                "Detected Stack:",
                Style::default().fg(palette.accent),
            )));
            if sys.detected_stack.is_empty() {
                lines.push(Line::from("  (none)"));
            } else {
                for stack in &sys.detected_stack {
                    lines.push(Line::from(format!("  - {}", stack)));
                }
            }
            lines.push(Line::from(""));

            lines.push(Line::from(Span::styled(
                "Entrypoints:",
                Style::default().fg(palette.accent),
            )));
            if sys.entrypoints.is_empty() {
                lines.push(Line::from("  (none)"));
            } else {
                for entry in &sys.entrypoints {
                    lines.push(Line::from(format!("  - {}", entry)));
                }
            }
            lines.push(Line::from(""));

            if !sys.risk_flags.is_empty() {
                lines.push(Line::from(Span::styled(
                    "Risk Flags:",
                    Style::default().fg(palette.danger),
                )));
                for risk in &sys.risk_flags {
                    lines.push(Line::from(format!("  - {}", risk)));
                }
                lines.push(Line::from(""));
            }

            lines.push(Line::from(Span::styled(
                "Summary:",
                Style::default().fg(palette.accent),
            )));
            lines.push(Line::from(sys.summary.as_str()));

            let p = Paragraph::new(lines)
                .block(content_block)
                .wrap(Wrap { trim: true });
            f.render_widget(p, main_area);
        } else {
            let p = Paragraph::new("No system artifact.").block(content_block);
            f.render_widget(p, main_area);
        }
    } else {
        let p = Paragraph::new("")
            .block(content_block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, main_area);
    }

    // Input
    let input_block_title = if state.interaction.is_thinking {
        format!("Chat Input {} (Thinking...)", get_spinner())
    } else {
        "Chat Input (Press 'i' to focus, 'Esc' to exit, Enter to send)".to_string()
    };
    let input_border_style = if state.interaction.focus_in_chat {
        Style::default().fg(palette.accent)
    } else {
        Style::default().fg(palette.border)
    };
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(input_block_title)
        .style(Style::default().bg(palette.panel_bg))
        .border_style(input_border_style);
    let input_text = if state.interaction.chat_input.is_empty() {
        if !state.interaction.focus_in_chat {
            Span::styled(
                "Press 'i' to type command...",
                Style::default().fg(palette.muted),
            )
        } else if (now_ms() / 500).is_multiple_of(2) {
            Span::styled("▌", Style::default().fg(palette.accent))
        } else {
            Span::raw("")
        }
    } else if state.interaction.focus_in_chat && (now_ms() / 500).is_multiple_of(2) {
        Span::raw(format!("{}▌", state.interaction.chat_input))
    } else {
        Span::raw(&state.interaction.chat_input)
    };
    let input = Paragraph::new(input_text).block(input_block);
    f.render_widget(input, chunks[input_idx]);

    if state.interaction.focus_in_chat
        && state.interaction.overlay == ShellOverlay::None
        && state.interaction.chat_input.starts_with('/')
    {
        let needle = state.interaction.chat_input.to_ascii_lowercase();
        let matches: Vec<&str> = CHAT_COMMAND_SUGGESTIONS
            .iter()
            .copied()
            .filter(|cmd| cmd.starts_with(&needle))
            .take(5)
            .collect();
        if !matches.is_empty() {
            let popup_h = (matches.len() as u16 + 2).min(7);
            let y = chunks[input_idx].y.saturating_sub(popup_h);
            let area = Rect::new(chunks[input_idx].x, y, chunks[input_idx].width, popup_h);
            let items: Vec<ListItem> = matches
                .into_iter()
                .map(|cmd| ListItem::new(Line::from(cmd)))
                .collect();
            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Command Suggestions")
                    .style(Style::default().bg(palette.panel_bg))
                    .border_style(Style::default().fg(palette.border)),
            );
            f.render_widget(Clear, area);
            f.render_widget(list, area);
        }
    }

    if let Some(action_idx) = action_idx {
        render_action_bar(f, chunks[action_idx], state, palette);
    }

    // Footer
    let footer_text = if state.interaction.focus_in_chat {
        "In Chat: /help /search /streammeta /auth /status /tab /theme /panel /provider /model /copylast /copydiff /copychat /copylogs | Esc exits input"
    } else if state.routing.tab == ShellTab::Chat {
        "Chat: i focus, / commands, /search filter, Up/Down scroll, Home/End/G nav, mouse click focus"
    } else if state.routing.tab == ShellTab::Telemetry {
        "Telemetry refreshes every 500ms | CPU/RAM/Process/Tokens/GPU live"
    } else {
        "Shortcuts: ? help | / palette | [ ] theme | j/o/a rails | 1..9 tabs | arrows+mouse nav | q quit"
    };
    let footer = Paragraph::new(footer_text).style(Style::default().fg(palette.muted));
    f.render_widget(footer, chunks[footer_idx]);

    // Overlays
    if let ShellOverlay::ConfirmReset = state.interaction.overlay {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title("Confirm Reset")
            .borders(Borders::ALL)
            .style(Style::default().bg(palette.panel_bg).fg(Color::White))
            .border_style(Style::default().fg(palette.warning));
        let text = Paragraph::new("Are you sure you want to reset the session?\nThis will clear all artifacts and logs.\n\n[Y] Confirm  [N] Cancel")
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(text, area);
    }

    if let ShellOverlay::Help = state.interaction.overlay {
        let area = centered_rect(60, 60, f.area());
        f.render_widget(Clear, area);
        let block = Block::default()
            .title("Keybindings")
            .borders(Borders::ALL)
            .style(Style::default().bg(palette.panel_bg).fg(Color::White))
            .border_style(Style::default().fg(palette.border));

        let help_text = vec![
            Line::from(Span::styled(
                "General",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  q        Quit"),
            Line::from("  ?        Show this help"),
            Line::from("  Tab/Right Next tab"),
            Line::from("  Left     Previous tab"),
            Line::from("  t        Telemetry view"),
            Line::from("  1..9     Jump to tab"),
            Line::from("  Home/End Jump top/bottom (logs/chat/diff/explain)"),
            Line::from("  z        Toggle focus mode"),
            Line::from("  [ / ]    Previous/next theme"),
            Line::from("  j/o/a    Toggle journey/context/action rails"),
            Line::from("  +/-      Resize input"),
            Line::from("  Ctrl+Up/Down Resize input"),
            Line::from(""),
            Line::from(Span::styled(
                "Chat",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  i        Focus chat input"),
            Line::from("  Esc      Unfocus chat input"),
            Line::from("  Enter    Submit message"),
            Line::from("  v        Review changes"),
            Line::from("  Up/Down  Scroll chat"),
            Line::from("  PgUp/Dn  Scroll chat page"),
            Line::from("  End/G    Jump to latest"),
            Line::from("  /help    Show slash commands"),
            Line::from("  /search  Filter chat history"),
            Line::from("  /streammeta Show provider stream metadata"),
            Line::from("  /auth    Start Codex device login flow"),
            Line::from("  /copylast Copy latest assistant response"),
            Line::from("  /copydiff Copy full diff"),
            Line::from("  /copychat Copy full chat transcript"),
            Line::from("  /copylogs Copy all logs"),
            Line::from("  Mouse    Click input to focus, click plan step to select"),
            Line::from(""),
            Line::from(Span::styled(
                "Logs",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  Up/Down  Scroll logs"),
            Line::from("  PgUp/Dn  Scroll logs page"),
            Line::from("  f        Filter log level"),
            Line::from("  End      Scroll to bottom"),
            Line::from(""),
            Line::from(Span::styled(
                "Session",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  r        Reset session"),
            Line::from(""),
            Line::from(Span::styled(
                "View",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from("  y        Copy Diff (in Diff view)"),
            Line::from("  s        Show System view"),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close",
                Style::default().fg(palette.warning),
            )),
        ];

        let text = Paragraph::new(help_text)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true });
        f.render_widget(text, area);
    }

    if let ShellOverlay::ActionPalette { selected, query } = &state.interaction.overlay {
        let area = centered_rect(60, 40, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title("Action Palette")
            .borders(Borders::ALL)
            .style(Style::default().bg(palette.panel_bg))
            .border_style(Style::default().fg(palette.border));
        f.render_widget(block.clone(), area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Length(1), Constraint::Min(0)].as_ref())
            .split(block.inner(area));

        let input =
            Paragraph::new(format!("> {}", query)).style(Style::default().fg(palette.accent));
        f.render_widget(input, layout[0]);

        let filtered_indices = filtered_palette_indices(query);
        let items: Vec<ListItem> = filtered_indices
            .iter()
            .enumerate()
            .map(|(i, &idx)| {
                let item = &PALETTE_ITEMS[idx];
                let style = if i == *selected {
                    Style::default().fg(Color::Black).bg(palette.accent)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(item.label).style(style)
            })
            .collect();
        let list = List::new(items);
        f.render_widget(list, layout[1]);
    }

    if let ShellOverlay::ModelSelection { selected } = &state.interaction.overlay {
        let area = centered_rect(40, 50, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .title("Select Model")
            .borders(Borders::ALL)
            .style(Style::default().bg(palette.panel_bg))
            .border_style(Style::default().fg(palette.border));
        let inner_area = block.inner(area);
        f.render_widget(block, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([Constraint::Min(0)].as_ref())
            .split(inner_area);

        let items: Vec<ListItem> = AVAILABLE_MODELS
            .iter()
            .enumerate()
            .map(|(i, &model_name)| {
                let style = if i == *selected {
                    Style::default().fg(Color::Black).bg(palette.accent)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(model_name).style(style)
            })
            .collect();
        let list = List::new(items);
        f.render_widget(list, layout[0]);
    }
}

fn render_overview(f: &mut ratatui::Frame, area: Rect, state: &ShellState, palette: UiPalette) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    let info_text = vec![
        Line::from(vec![
            Span::styled("Project: ", Style::default().fg(palette.accent)),
            Span::raw(&state.header.project_name),
        ]),
        Line::from(vec![
            Span::styled("Run ID: ", Style::default().fg(palette.accent)),
            Span::raw(state.current_run_id().to_string()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Journey State: ", Style::default().fg(palette.accent)),
            Span::raw(state.journey_status.state.label()),
        ]),
        Line::from(vec![
            Span::styled("Current Step: ", Style::default().fg(palette.accent)),
            Span::raw(state.journey_status.step.label()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Safety Mode: ", Style::default().fg(palette.accent)),
            Span::raw(state.header.safety_mode.label()),
        ]),
        Line::from(vec![
            Span::styled("Risk Level: ", Style::default().fg(palette.accent)),
            Span::raw(state.header.risk.label()),
        ]),
    ];

    let info_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border))
        .title("Project Status");
    let info_p = Paragraph::new(info_text).block(info_block);
    f.render_widget(info_p, top_chunks[0]);

    let scan_lbl = state.header.scan.label();
    let plan_lbl = if state.artifacts.plan.is_some() {
        "Available"
    } else {
        "Pending"
    };
    let diff_lbl = if state.artifacts.diff.is_some() {
        "Available"
    } else {
        "Pending"
    };
    let verify_lbl = state.header.verify.label();

    let status_text = vec![
        Line::from(vec![
            Span::styled("Scan: ", Style::default().fg(palette.accent)),
            Span::raw(scan_lbl),
        ]),
        Line::from(vec![
            Span::styled("Plan: ", Style::default().fg(palette.accent)),
            Span::raw(plan_lbl),
        ]),
        Line::from(vec![
            Span::styled("Diff: ", Style::default().fg(palette.accent)),
            Span::raw(diff_lbl),
        ]),
        Line::from(vec![
            Span::styled("Verify: ", Style::default().fg(palette.accent)),
            Span::raw(verify_lbl),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Apply Status: ", Style::default().fg(palette.accent)),
            Span::raw(state.header.apply.label()),
        ]),
    ];

    let status_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border))
        .title("Workflow Artifacts");
    let status_p = Paragraph::new(status_text).block(status_block);
    f.render_widget(status_p, top_chunks[1]);

    let logs: Vec<Line> = state
        .artifacts
        .logs
        .iter()
        .rev()
        .take(20)
        .map(|l| {
            let style = match l.level {
                dao_core::state::LogLevel::Error => Style::default().fg(palette.danger),
                dao_core::state::LogLevel::Warn => Style::default().fg(palette.warning),
                _ => Style::default(),
            };
            Line::from(Span::styled(
                format!("[{:?}] {}", l.level, l.message),
                style,
            ))
        })
        .collect();

    let logs_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border))
        .title("Recent Activity");
    let logs_p = Paragraph::new(logs).block(logs_block);
    f.render_widget(logs_p, chunks[1]);
}

fn render_telemetry(f: &mut ratatui::Frame, area: Rect, state: &ShellState, palette: UiPalette) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(area);

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[0]);
    let cpu_pct = state.telemetry.latest.cpu_percent.clamp(0.0, 100.0) as u16;
    let mem_total = state.telemetry.latest.mem_total_mb.max(1);
    let mem_pct = ((state.telemetry.latest.mem_used_mb as f64 / mem_total as f64) * 100.0)
        .clamp(0.0, 100.0) as u16;

    let cpu_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("CPU"))
        .gauge_style(Style::default().fg(palette.accent))
        .percent(cpu_pct)
        .label(format!("{cpu_pct}%"));
    f.render_widget(cpu_gauge, top[0]);

    let mem_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("RAM"))
        .gauge_style(Style::default().fg(palette.success))
        .percent(mem_pct)
        .label(format!(
            "{} / {} MB",
            state.telemetry.latest.mem_used_mb, state.telemetry.latest.mem_total_mb
        ));
    f.render_widget(mem_gauge, top[1]);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    let proc_mem = state.telemetry.latest.process_mem_mb;
    let proc_ratio = ((proc_mem as f64 / mem_total as f64) * 100.0).clamp(0.0, 100.0) as u16;
    let proc_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Process RSS"))
        .gauge_style(Style::default().fg(palette.warning))
        .percent(proc_ratio)
        .label(format!("{} MB", proc_mem));
    f.render_widget(proc_gauge, mid[0]);

    let tps = state.telemetry.latest.tokens_per_second.unwrap_or(0.0);
    let tps_pct = tps.clamp(0.0, 100.0) as u16;
    let tps_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Tokens / sec"))
        .gauge_style(Style::default().fg(palette.accent_alt))
        .percent(tps_pct)
        .label(format!("{tps:.1} tok/s"));
    f.render_widget(tps_gauge, mid[1]);

    let charts = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(rows[2]);
    let cpu_spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("CPU Trend"))
        .data(&state.telemetry.cpu_history)
        .style(Style::default().fg(palette.accent));
    f.render_widget(cpu_spark, charts[0]);

    let mem_spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("RAM % Trend"))
        .data(&state.telemetry.mem_history)
        .style(Style::default().fg(palette.success));
    f.render_widget(mem_spark, charts[1]);

    let tps_spark = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title("TPS Trend"))
        .data(&state.telemetry.tps_history)
        .style(Style::default().fg(palette.accent_alt));
    f.render_widget(tps_spark, charts[2]);

    let details = vec![
        Line::from(vec![
            Span::styled("Provider: ", Style::default().fg(palette.accent)),
            Span::raw(resolved_provider(state)),
            Span::raw("   "),
            Span::styled("Model: ", Style::default().fg(palette.accent)),
            Span::raw(resolved_model_slug(state)),
        ]),
        Line::from(vec![
            Span::styled("Generated Tokens: ", Style::default().fg(palette.accent)),
            Span::raw(
                state
                    .telemetry
                    .latest
                    .tokens_generated
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
            ),
        ]),
        Line::from(vec![
            Span::styled("GPU: ", Style::default().fg(palette.accent)),
            Span::raw(
                state
                    .telemetry
                    .latest
                    .gpu_util_percent
                    .map(|v| format!("{v:.1}%"))
                    .unwrap_or_else(|| "N/A".to_string()),
            ),
        ]),
        Line::from(vec![
            Span::styled("GPU Memory: ", Style::default().fg(palette.accent)),
            Span::raw(
                match (
                    state.telemetry.latest.gpu_mem_used_mb,
                    state.telemetry.latest.gpu_mem_total_mb,
                ) {
                    (Some(used), Some(total)) => format!("{used} / {total} MB"),
                    (Some(used), None) => format!("{used} MB (total unavailable)"),
                    _ => "N/A (unsupported)".to_string(),
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("GPU Status: ", Style::default().fg(palette.accent)),
            Span::raw(
                state
                    .telemetry
                    .latest
                    .gpu_status
                    .clone()
                    .unwrap_or_else(|| "N/A (unsupported)".to_string()),
            ),
        ]),
        Line::from("Tip: press 't' for telemetry from any tab."),
    ];
    let p = Paragraph::new(details)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.border))
                .title("Live Metrics"),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, rows[3]);
}

fn render_journey_rail(f: &mut ratatui::Frame, area: Rect, state: &ShellState, palette: UiPalette) {
    let steps = [
        ("Idea", 0_u8),
        ("Understand", 1),
        ("Plan", 2),
        ("Preview", 3),
        ("Approve", 4),
        ("Verify", 5),
        ("Learn", 6),
    ];
    let current_step = match state.journey_status.step {
        dao_core::state::JourneyStep::Idea => 0_u8,
        dao_core::state::JourneyStep::Understand => 1,
        dao_core::state::JourneyStep::Plan => 2,
        dao_core::state::JourneyStep::Preview => 3,
        dao_core::state::JourneyStep::Approve => 4,
        dao_core::state::JourneyStep::Verify => 5,
        dao_core::state::JourneyStep::Learn => 6,
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Journey: ", Style::default().fg(palette.accent)),
            Span::raw(state.journey_status.state.label()),
        ]),
        Line::from(""),
    ];
    for (label, idx) in steps {
        let (marker, color) = if idx < current_step {
            ("●", palette.success)
        } else if idx == current_step {
            ("➤", palette.accent)
        } else {
            ("○", palette.muted)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), Style::default().fg(color)),
            Span::styled(label, Style::default().fg(color)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "j toggle rail",
        Style::default().fg(palette.muted),
    )));

    let block = Block::default()
        .title("Journey")
        .borders(Borders::ALL)
        .style(Style::default().bg(palette.panel_bg))
        .border_style(Style::default().fg(palette.border));
    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_context_rail(f: &mut ratatui::Frame, area: Rect, state: &ShellState, palette: UiPalette) {
    let chat_lines = chat_line_count(state);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Provider: ", Style::default().fg(palette.accent)),
            Span::raw(resolved_provider(state)),
        ]),
        Line::from(vec![
            Span::styled("Model: ", Style::default().fg(palette.accent)),
            Span::raw(resolved_model_slug(state)),
        ]),
        Line::from(vec![
            Span::styled("Theme: ", Style::default().fg(palette.accent)),
            Span::raw(state.customization.theme.label()),
        ]),
        Line::from(vec![
            Span::styled("Keymap: ", Style::default().fg(palette.accent)),
            Span::raw(state.customization.keymap_preset.label()),
        ]),
        Line::from(vec![
            Span::styled("Chat Lines: ", Style::default().fg(palette.accent)),
            Span::raw(chat_lines.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Input Height: ", Style::default().fg(palette.accent)),
            Span::raw(state.customization.input_height.to_string()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Quick Toggles",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  [ / ]  theme"),
        Line::from("  j      journey rail"),
        Line::from("  o      context rail"),
        Line::from("  a      action bar"),
        Line::from("  z      focus mode"),
        Line::from(""),
        Line::from(Span::styled(
            "Slash Utility",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  /copylast"),
        Line::from("  /copydiff"),
        Line::from("  /copychat"),
        Line::from("  /copylogs"),
        Line::from("  /streammeta <on|off>"),
        Line::from("  /auth <codex>"),
        Line::from("  /search <text|clear>"),
        Line::from("  /panel <name>"),
    ];

    if let Some(thread_id) = &state.thread_id {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Thread: ", Style::default().fg(palette.accent)),
            Span::raw(thread_id.0.as_str()),
        ]));
    }

    let block = Block::default()
        .title("Context")
        .borders(Borders::ALL)
        .style(Style::default().bg(palette.panel_bg))
        .border_style(Style::default().fg(palette.border));
    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_action_bar(f: &mut ratatui::Frame, area: Rect, state: &ShellState, palette: UiPalette) {
    let tab = state.routing.tab.label();
    let text = Line::from(vec![
        Span::styled("Tab ", Style::default().fg(palette.muted)),
        Span::styled(tab, Style::default().fg(palette.accent)),
        Span::styled(" | ", Style::default().fg(palette.muted)),
        Span::styled("i", Style::default().fg(palette.accent)),
        Span::styled(" chat ", Style::default().fg(palette.muted)),
        Span::styled("/", Style::default().fg(palette.accent)),
        Span::styled(" palette ", Style::default().fg(palette.muted)),
        Span::styled("?", Style::default().fg(palette.accent)),
        Span::styled(" help ", Style::default().fg(palette.muted)),
        Span::styled("[ ]", Style::default().fg(palette.accent)),
        Span::styled(" theme ", Style::default().fg(palette.muted)),
        Span::styled("q", Style::default().fg(palette.warning)),
        Span::styled(" quit", Style::default().fg(palette.muted)),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette.border))
        .style(Style::default().bg(palette.panel_bg))
        .title("Action Bar");
    let p = Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(p, area);
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    r: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
