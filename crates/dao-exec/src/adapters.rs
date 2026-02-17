use crate::executor::ToolExecutionPayload;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub struct ShellAdapter;

pub enum ChatEvent {
    Token(String),
    Meta(String),
    Done,
}

fn build_chat_prompt(provider: &str, model: &str, message: &str, context: Option<&str>) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "System:\n\
You are DAO's assistant running through the configured local CLI provider on the user's machine.\n\
When asked about identity or model, answer with this exact runtime model: ",
    );
    prompt.push_str(model);
    prompt.push_str(" and provider: ");
    prompt.push_str(provider);
    prompt.push_str(
        ".\n\
Do not claim to be a provider-specific hosted assistant.\n\
Keep responses factual and concise.\n\n",
    );

    if let Some(ctx) = context {
        prompt.push_str("Context:\n");
        prompt.push_str(ctx);
        prompt.push_str("\n\n");
    }

    prompt.push_str("User Request: ");
    prompt.push_str(message);
    prompt
}

fn resolve_provider(provider: Option<&str>) -> &str {
    match provider.unwrap_or("ollama").to_ascii_lowercase().as_str() {
        "codex" => "codex",
        "gemini" => "gemini",
        _ => "ollama",
    }
}

fn default_model_for_provider(provider: &str) -> &'static str {
    match provider {
        "codex" => "gpt-5",
        "gemini" => "gemini-2.5-pro",
        _ => "phi3:mini-128k",
    }
}

fn stream_command_output<F>(mut cmd: Command, provider_label: &str, callback: &F)
where
    F: Fn(ChatEvent),
{
    let spawn = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(err) => {
            callback(ChatEvent::Token(format!(
                "Failed to start {} CLI: {}",
                provider_label, err
            )));
            callback(ChatEvent::Done);
            return;
        }
    };

    let stderr_handle = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut stderr_text = String::new();
            let _ = stderr.read_to_string(&mut stderr_text);
            stderr_text
        })
    });

    let mut emitted = false;
    if let Some(mut stdout) = child.stdout.take() {
        let mut buf = [0_u8; 2048];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = strip_ansi_sequences(&String::from_utf8_lossy(&buf[..n]));
                    if !chunk.is_empty() {
                        emitted = true;
                        callback(ChatEvent::Token(chunk));
                    }
                }
                Err(_) => break,
            }
        }
    }

    let status = child.wait().ok();
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    if !status.is_some_and(|s| s.success()) {
        let msg = if stderr_text.is_empty() {
            format!("{} CLI exited with a non-zero status.", provider_label)
        } else {
            format!("{} CLI error: {}", provider_label, stderr_text)
        };
        callback(ChatEvent::Meta(msg));
    } else if !emitted {
        callback(ChatEvent::Token("[assistant] (empty response)".to_string()));
    }

    callback(ChatEvent::Done);
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                while let Some(n) = chars.next() {
                    if ('@'..='~').contains(&n) {
                        break;
                    }
                }
                continue;
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }
    out
}

fn push_delta_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if val.is_string() {
                    let key = k.to_ascii_lowercase();
                    if key.contains("delta") || key == "text" || key == "content" {
                        if let Some(s) = val.as_str() {
                            if !s.is_empty() {
                                out.push(s.to_string());
                            }
                        }
                    }
                }
                push_delta_strings(val, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                push_delta_strings(item, out);
            }
        }
        _ => {}
    }
}

fn emit_chunked_text<F>(text: &str, callback: &F)
where
    F: Fn(ChatEvent),
{
    // Fallback "progressive render" for providers that only emit final text events.
    // Keeps perceived responsiveness without changing semantic content.
    const CHUNK: usize = 24;
    const SLEEP_MS: u64 = 12;
    if text.chars().count() <= CHUNK {
        callback(ChatEvent::Token(text.to_string()));
        return;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let end = (i + CHUNK).min(chars.len());
        let piece: String = chars[i..end].iter().collect();
        callback(ChatEvent::Token(piece));
        i = end;
        if i < chars.len() {
            thread::sleep(Duration::from_millis(SLEEP_MS));
        }
    }
}

fn stream_gemini_json<F>(mut cmd: Command, callback: &F)
where
    F: Fn(ChatEvent),
{
    let spawn = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(err) => {
            callback(ChatEvent::Token(format!(
                "Failed to start Gemini CLI: {}",
                err
            )));
            callback(ChatEvent::Done);
            return;
        }
    };

    let stderr_handle = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut stderr_text = String::new();
            let _ = stderr.read_to_string(&mut stderr_text);
            stderr_text
        })
    });

    let mut emitted = false;
    let mut last_assistant = String::new();
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if !trimmed.starts_with('{') {
                if !trimmed.is_empty() {
                    callback(ChatEvent::Meta(trimmed.to_string()));
                }
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
                callback(ChatEvent::Meta(trimmed.to_string()));
                continue;
            };
            let t = v.get("type").and_then(Value::as_str).unwrap_or_default();
            if t != "message" {
                callback(ChatEvent::Meta(format!("gemini event: {}", t)));
                continue;
            }
            if v.get("role").and_then(Value::as_str) != Some("assistant") {
                continue;
            }
            if let Some(content) = v.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    let chunk = if v.get("delta").and_then(Value::as_bool).unwrap_or(false) {
                        if let Some(rest) = content.strip_prefix(&last_assistant) {
                            rest.to_string()
                        } else {
                            content.to_string()
                        }
                    } else {
                        content.to_string()
                    };
                    if !chunk.is_empty() {
                        emitted = true;
                        callback(ChatEvent::Token(chunk));
                    }
                    last_assistant = content.to_string();
                }
            }
        }
    }

    let status = child.wait().ok();
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    if !status.is_some_and(|s| s.success()) {
        let msg = if stderr_text.is_empty() {
            "Gemini CLI exited with a non-zero status.".to_string()
        } else {
            format!("Gemini CLI error: {}", stderr_text)
        };
        callback(ChatEvent::Meta(msg));
    } else if !emitted {
        callback(ChatEvent::Token("[assistant] (empty response)".to_string()));
    }
    callback(ChatEvent::Done);
}

fn stream_codex_json<F>(mut cmd: Command, callback: &F)
where
    F: Fn(ChatEvent),
{
    let spawn = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(err) => {
            callback(ChatEvent::Token(format!(
                "Failed to start Codex CLI: {}",
                err
            )));
            callback(ChatEvent::Done);
            return;
        }
    };

    let stderr_handle = child.stderr.take().map(|mut stderr| {
        thread::spawn(move || {
            let mut stderr_text = String::new();
            let _ = stderr.read_to_string(&mut stderr_text);
            stderr_text
        })
    });

    let mut emitted = false;
    let mut saw_delta = false;
    let mut assistant_so_far = String::new();
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if !trimmed.starts_with('{') {
                if !trimmed.is_empty() {
                    // Suppress noisy runtime warnings from Codex internals in normal stream logs.
                    let is_noisy_warn = trimmed.contains("WARN codex_protocol::openai_models")
                        || trimmed.contains("model personality requested");
                    if !is_noisy_warn {
                        callback(ChatEvent::Meta(trimmed.to_string()));
                    }
                }
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
                callback(ChatEvent::Meta(trimmed.to_string()));
                continue;
            };
            let event_type = v.get("type").and_then(Value::as_str).unwrap_or_default();
            if event_type == "item.completed" {
                let item = v.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("agent_message") {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            // If we already streamed deltas, avoid duplicating full final message.
                            let suffix = if let Some(rest) = text.strip_prefix(&assistant_so_far) {
                                rest
                            } else if saw_delta {
                                ""
                            } else {
                                text
                            };
                            if !suffix.is_empty() {
                                emitted = true;
                                // Codex `exec --json` may be final-only; progressively emit to improve UX.
                                if saw_delta {
                                    callback(ChatEvent::Token(suffix.to_string()));
                                } else {
                                    emit_chunked_text(suffix, callback);
                                }
                            }
                            assistant_so_far = text.to_string();
                        }
                    }
                }
            } else if event_type.contains("delta") {
                let mut deltas = Vec::new();
                push_delta_strings(&v, &mut deltas);
                for delta in deltas {
                    if !delta.is_empty() {
                        saw_delta = true;
                        assistant_so_far.push_str(&delta);
                        emitted = true;
                        callback(ChatEvent::Token(delta));
                    }
                }
            } else {
                let noisy = matches!(
                    event_type,
                    "thread.started" | "turn.started" | "turn.completed"
                );
                if !noisy {
                    callback(ChatEvent::Meta(format!("codex event: {}", event_type)));
                }
            }
        }
    }

    let status = child.wait().ok();
    let stderr_text = stderr_handle
        .and_then(|h| h.join().ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    if !status.is_some_and(|s| s.success()) {
        let msg = if stderr_text.is_empty() {
            "Codex CLI exited with a non-zero status.".to_string()
        } else {
            format!("Codex CLI error: {}", stderr_text)
        };
        callback(ChatEvent::Meta(msg));
    } else if !emitted {
        callback(ChatEvent::Token("[assistant] (empty response)".to_string()));
    }
    callback(ChatEvent::Done);
}

impl ShellAdapter {
    pub fn generate_plan(
        cwd: &std::path::Path,
        task: &str,
        model: Option<&str>,
    ) -> ToolExecutionPayload {
        // 1. Try running a local script first (e.g., .dao/plan.sh)
        // This allows project-specific overrides for planning logic.
        let script_path = cwd.join(".dao/plan.sh");
        if script_path.exists() {
            eprintln!("> Running local plan script: {}", script_path.display());
            if let Ok(mut child) = Command::new(&script_path)
                .arg(task)
                .current_dir(cwd)
                .stdout(Stdio::piped())
                .spawn()
            {
                if let Some(stdout) = child.stdout.take() {
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines().map_while(|l| l.ok()) {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                    });

                    let mut steps = Vec::new();
                    let mut timeout = Duration::from_secs(10);

                    loop {
                        match rx.recv_timeout(timeout) {
                            Ok(line) => {
                                let trimmed = line.trim().to_string();
                                if !trimmed.is_empty() {
                                    steps.push(trimmed);
                                }
                                timeout = Duration::from_secs(5);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                eprintln!("> Local script timed out.");
                                let _ = child.kill();
                                break;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }

                    if let Ok(status) = child.wait() {
                        if status.success() && !steps.is_empty() {
                            return ToolExecutionPayload::Plan { steps };
                        }
                    }
                }
            }
        }

        // 2. Try using Ollama (local LLM)
        let model = model.unwrap_or("phi3:mini-128k");
        let prompt = format!(
            "You are a senior software engineer. \
            Create a concise, step-by-step execution plan for the following task: '{}'. \
            Return ONLY the steps as a list, one per line. Do not include numbering, bullets, or preamble.",
            task
        );

        eprintln!("> Generating plan with Ollama ({})...", model);
        if let Ok(mut child) = Command::new("ollama")
            .args(&["run", model, &prompt])
            .stdout(Stdio::piped())
            .spawn()
        {
            if let Some(stdout) = child.stdout.take() {
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(|l| l.ok()) {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                });

                let mut steps = Vec::new();
                // Allow 60s for model loading/first token, then 10s per line
                let mut timeout = Duration::from_secs(60);

                loop {
                    match rx.recv_timeout(timeout) {
                        Ok(line) => {
                            let trimmed = line.trim().to_string();
                            if !trimmed.is_empty() {
                                eprintln!("  • {}", trimmed);
                                steps.push(trimmed);
                            }
                            timeout = Duration::from_secs(10);
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            eprintln!("> Ollama request timed out.");
                            let _ = child.kill();
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }

                let _ = child.wait();

                if !steps.is_empty() {
                    return ToolExecutionPayload::Plan { steps };
                }
            }
        }

        // 3. Fallback default plan
        ToolExecutionPayload::Plan {
            steps: vec![
                format!("Analyze request: {}", task),
                "Check existing files".to_string(),
                "Implement changes".to_string(),
                "Verify results".to_string(),
            ],
        }
    }

    pub fn chat(provider: Option<&str>, model: Option<&str>, message: &str) {
        let provider = resolve_provider(provider);
        let model = model.unwrap_or(default_model_for_provider(provider));
        eprintln!("> Chatting with {} ({})...", provider, model);

        let prompt = if message.is_empty() {
            String::new()
        } else {
            build_chat_prompt(provider, model, message, None)
        };

        let mut cmd = match provider {
            "codex" => {
                let mut c = Command::new("codex");
                c.arg("exec").arg("--skip-git-repo-check");
                if !message.is_empty() {
                    if !model.is_empty() {
                        c.arg("-m").arg(model);
                    }
                    c.arg(prompt);
                }
                c
            }
            "gemini" => {
                let mut c = Command::new("gemini");
                if !message.is_empty() {
                    c.arg("-p").arg(prompt);
                    if !model.is_empty() {
                        c.arg("-m").arg(model);
                    }
                }
                c
            }
            _ => {
                let mut c = Command::new("ollama");
                c.arg("run").arg(model);
                if !message.is_empty() {
                    c.arg(prompt);
                }
                c
            }
        };

        let mut child = cmd
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn chat backend");

        let _ = child.wait();
    }

    pub fn chat_stream<F>(
        provider: Option<&str>,
        model: Option<&str>,
        message: &str,
        context: Option<&str>,
        callback: F,
    ) where
        F: Fn(ChatEvent) + Send + 'static,
    {
        let provider = resolve_provider(provider).to_string();
        let model = model
            .unwrap_or(default_model_for_provider(&provider))
            .to_string();
        let message = build_chat_prompt(&provider, &model, message, context);

        thread::spawn(move || {
            if provider == "ollama" {
                let mut cmd = Command::new("ollama");
                cmd.args(["run", "--nowordwrap", &model, &message]);
                stream_command_output(cmd, "Ollama", &callback);
                return;
            }

            if provider == "codex" {
                let mut cmd = Command::new("codex");
                cmd.arg("exec").arg("--skip-git-repo-check").arg("--json");
                if !model.is_empty() {
                    cmd.arg("-m").arg(&model);
                }
                cmd.arg(&message);
                stream_codex_json(cmd, &callback);
                return;
            }

            if provider == "gemini" {
                let mut cmd = Command::new("gemini");
                cmd.arg("-p")
                    .arg(&message)
                    .arg("--output-format")
                    .arg("stream-json");
                if !model.is_empty() {
                    cmd.arg("-m").arg(&model);
                }
                stream_gemini_json(cmd, &callback);
                return;
            }

            callback(ChatEvent::Token(format!(
                "Unsupported provider: {}",
                provider
            )));
            callback(ChatEvent::Done);
        });
    }
}
