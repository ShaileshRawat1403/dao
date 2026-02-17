use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPolicy {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub mode: PolicyMode,
    #[serde(default)]
    pub precedence: PolicyPrecedence,
    pub applies_to: PolicyScope,
    pub defaults: PolicyDefaults,
    pub rules: Vec<PolicyRule>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    AllowByDefault,
    #[default]
    DenyByDefault,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyPrecedence {
    #[default]
    FirstMatch,
    BestScore,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyScope {
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub environments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyDefaults {
    pub approval: ApprovalConfig,
    #[serde(default)]
    pub evidence_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub when: String, // Expression string (e.g., "diff.files_changed > 10")
    pub then: RuleAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RuleAction {
    Allow {
        message: Option<String>,
    },
    Block {
        message: String,
        remediation: Option<Vec<String>>,
    },
    RequireApproval {
        message: String,
        #[serde(flatten)]
        approval: ApprovalConfig,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalConfig {
    #[serde(default = "default_approval_count")]
    pub required: u8,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default = "default_true")]
    pub justification_required: bool,
    pub justification_prompt: Option<String>,
}

fn default_approval_count() -> u8 {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub policy_id: String,
    pub decision: DecisionOutcome,
    pub matched_rule_id: Option<String>,
    pub message: String,
    pub requirements: Option<ApprovalConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Allowed,
    Blocked,
    ApprovalRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Signals {
    pub diff_files_changed: usize,
    pub diff_lines_added: usize,
    pub diff_lines_deleted: usize,
    pub risk_class: String,
    pub diff_file_names: String,
    pub commit_message: String,
    pub diff_added_content: String,
    pub new_file_contents: Vec<String>,
    pub new_file_paths: Vec<String>,
}

impl ReviewPolicy {
    pub fn evaluate(&self, signals: &Signals) -> PolicyDecision {
        for rule in &self.rules {
            if self.evaluate_condition(&rule.when, signals) {
                return PolicyDecision {
                    policy_id: self.id.clone(),
                    decision: rule.then.to_decision_outcome(),
                    matched_rule_id: Some(rule.id.clone()),
                    message: rule.then.message(),
                    requirements: rule.then.approval_config(),
                };
            }
        }

        match self.mode {
            PolicyMode::AllowByDefault => PolicyDecision {
                policy_id: self.id.clone(),
                decision: DecisionOutcome::Allowed,
                matched_rule_id: None,
                message: "Allowed by default".to_string(),
                requirements: None,
            },
            PolicyMode::DenyByDefault => PolicyDecision {
                policy_id: self.id.clone(),
                decision: DecisionOutcome::ApprovalRequired,
                matched_rule_id: None,
                message: "Approval required by default".to_string(),
                requirements: Some(self.defaults.approval.clone()),
            },
        }
    }

    fn evaluate_condition(&self, condition: &str, signals: &Signals) -> bool {
        use evalexpr::*;
        let mut context = HashMapContext::new();
        context
            .set_value(
                "diff_files_changed".into(),
                Value::Int(signals.diff_files_changed as i64),
            )
            .ok();
        context
            .set_value(
                "diff_lines_added".into(),
                Value::Int(signals.diff_lines_added as i64),
            )
            .ok();
        context
            .set_value(
                "diff_lines_deleted".into(),
                Value::Int(signals.diff_lines_deleted as i64),
            )
            .ok();
        context
            .set_value(
                "risk_class".into(),
                Value::String(signals.risk_class.clone()),
            )
            .ok();
        context
            .set_value(
                "diff_file_names".into(),
                Value::String(signals.diff_file_names.clone()),
            )
            .ok();
        context
            .set_value(
                "commit_message".into(),
                Value::String(signals.commit_message.clone()),
            )
            .ok();
        context
            .set_value(
                "diff_added_content".into(),
                Value::String(signals.diff_added_content.clone()),
            )
            .ok();
        context
            .set_value(
                "new_file_contents".into(),
                Value::Tuple(
                    signals
                        .new_file_contents
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            )
            .ok();
        context
            .set_value(
                "new_file_paths".into(),
                Value::Tuple(
                    signals
                        .new_file_paths
                        .iter()
                        .map(|s| Value::String(s.clone()))
                        .collect(),
                ),
            )
            .ok();

        context
            .set_function(
                "contains".into(),
                Function::new(|argument| {
                    let arguments = argument.as_tuple()?;
                    if arguments.len() != 2 {
                        return Err(EvalexprError::CustomMessage(
                            "contains() expects exactly 2 arguments".to_string(),
                        ));
                    }
                    if let (Value::String(haystack), Value::String(needle)) =
                        (&arguments[0], &arguments[1])
                    {
                        Ok(Value::Boolean(haystack.contains(needle)))
                    } else {
                        Err(EvalexprError::CustomMessage(
                            "contains() expects string arguments".to_string(),
                        ))
                    }
                }),
            )
            .ok();

        context
            .set_function(
                "regex_match".into(),
                Function::new(|argument| {
                    let arguments = argument.as_tuple()?;
                    if arguments.len() != 2 {
                        return Err(EvalexprError::CustomMessage(
                            "regex_match() expects exactly 2 arguments".to_string(),
                        ));
                    }
                    if let (Value::String(haystack), Value::String(pattern)) =
                        (&arguments[0], &arguments[1])
                    {
                        let re = regex::Regex::new(pattern).map_err(|e| {
                            EvalexprError::CustomMessage(format!("Invalid regex: {}", e))
                        })?;
                        Ok(Value::Boolean(re.is_match(haystack)))
                    } else {
                        Err(EvalexprError::CustomMessage(
                            "regex_match() expects string arguments".to_string(),
                        ))
                    }
                }),
            )
            .ok();

        context
            .set_function(
                "all_match".into(),
                Function::new(|argument| {
                    let arguments = argument.as_tuple()?;
                    if arguments.len() != 2 {
                        return Err(EvalexprError::CustomMessage(
                            "all_match() expects exactly 2 arguments".to_string(),
                        ));
                    }
                    let list = match &arguments[0] {
                        Value::Tuple(t) => t,
                        _ => {
                            return Err(EvalexprError::CustomMessage(
                                "all_match() first argument must be a list (tuple)".to_string(),
                            ))
                        }
                    };
                    let pattern = match &arguments[1] {
                        Value::String(s) => s,
                        _ => {
                            return Err(EvalexprError::CustomMessage(
                                "all_match() second argument must be a regex string".to_string(),
                            ))
                        }
                    };
                    let re = regex::Regex::new(pattern).map_err(|e| {
                        EvalexprError::CustomMessage(format!("Invalid regex: {}", e))
                    })?;

                    Ok(Value::Boolean(list.iter().all(
                        |item| matches!(item, Value::String(s) if re.is_match(s)),
                    )))
                }),
            )
            .ok();

        context
            .set_function(
                "missing_tests".into(),
                Function::new(|argument| {
                    let arguments = argument.as_tuple()?;

                    let files: Vec<String> = if arguments.len() == 1 {
                        match &arguments[0] {
                            Value::Tuple(list) => list
                                .iter()
                                .filter_map(|v| match v {
                                    Value::String(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .collect(),
                            Value::String(s) => vec![s.clone()],
                            _ => {
                                return Err(EvalexprError::CustomMessage(
                                    "missing_tests() argument must be a list or string".to_string(),
                                ))
                            }
                        }
                    } else {
                        arguments
                            .iter()
                            .filter_map(|v| match v {
                                Value::String(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect()
                    };

                    let source_exts = ["rs", "py", "js", "ts", "go", "java", "c", "cpp"];

                    for file in &files {
                        let path = std::path::Path::new(file);
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        let is_source = source_exts.contains(&ext);
                        let is_test = file.to_lowercase().contains("test")
                            || file.to_lowercase().contains("spec");

                        if is_source && !is_test {
                            let has_match = files.iter().any(|f| {
                                let f_lower = f.to_lowercase();
                                (f_lower.contains("test") || f_lower.contains("spec"))
                                    && f_lower.contains(stem)
                            });
                            if !has_match {
                                return Ok(Value::Boolean(true));
                            }
                        }
                    }
                    Ok(Value::Boolean(false))
                }),
            )
            .ok();

        match eval_boolean_with_context(condition, &context) {
            Ok(result) => result,
            Err(e) => {
                eprintln!(
                    "Policy evaluation error for condition '{}': {}",
                    condition, e
                );
                false
            }
        }
    }
}

impl RuleAction {
    pub fn to_decision_outcome(&self) -> DecisionOutcome {
        match self {
            RuleAction::Allow { .. } => DecisionOutcome::Allowed,
            RuleAction::Block { .. } => DecisionOutcome::Blocked,
            RuleAction::RequireApproval { .. } => DecisionOutcome::ApprovalRequired,
        }
    }

    pub fn message(&self) -> String {
        match self {
            RuleAction::Allow { message } => message.clone().unwrap_or_default(),
            RuleAction::Block { message, .. } => message.clone(),
            RuleAction::RequireApproval { message, .. } => message.clone(),
        }
    }

    pub fn approval_config(&self) -> Option<ApprovalConfig> {
        match self {
            RuleAction::RequireApproval { approval, .. } => Some(approval.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_yaml_loading_and_logic() {
        let yaml = r#"
id: "security-guardrails-v1"
version: "1.0"
mode: "deny_by_default"
applies_to:
  branches: ["main", "production"]
  environments: ["prod"]
defaults:
  approval:
    required: 1
    roles: ["maintainer", "security-lead"]
rules:
  - id: "allow-documentation"
    when: "diff_files_changed < 5"
    then:
      action: "allow"
      message: "Small documentation changes are auto-approved."
  - id: "block-secrets"
    when: 'risk_class == "destructive"'
    then:
      action: "block"
      message: "Destructive changes require manual override."
  - id: "block-large-changes"
    when: "diff_files_changed > 10"
    then:
      action: "block"
      message: "Too many files changed."
  - id: "approval-large-additions"
    when: "diff_lines_added > 500"
    then:
      action: "require_approval"
      message: "Large additions require approval."
  - id: "allow-refactor"
    when: 'risk_class == "refactor"'
    then:
      action: "allow"
      message: "Refactors are auto-approved."
  - id: "block-net-deletion"
    when: 'diff_lines_deleted > diff_lines_added && risk_class != "refactor"'
    then:
      action: "block"
      message: "Net deletion is blocked."
  - id: "auth-security-check"
    when: 'contains(diff_file_names, "auth/")'
    then:
      action: "require_approval"
      message: "Changes to auth/ require double approval."
      required: 2
      roles: ["security-team", "tech-lead"]
  - id: "block-secret-files"
    when: 'contains(diff_file_names, ".env") || contains(diff_file_names, ".pem") || contains(diff_file_names, ".key")'
    then:
      action: "block"
      message: "Committing secret files is forbidden."
  - id: "wip-check"
    when: 'contains(commit_message, "WIP")'
    then:
      action: "require_approval"
      message: "Work in progress (WIP) requires approval."
      required: 1
      roles: ["maintainer"]
  - id: "conventional-commit-check"
    when: 'regex_match(commit_message, "^feat:.*")'
    then:
      action: "allow"
      message: "Feature commits are allowed."
  - id: "sync-cargo-lock"
    when: 'contains(diff_file_names, "Cargo.toml") && !contains(diff_file_names, "Cargo.lock")'
    then:
      action: "block"
      message: "Cargo.toml changed but Cargo.lock did not. Please update the lockfile."
  - id: "block-unsafe"
    when: 'contains(diff_added_content, "unsafe")'
    then:
      action: "block"
      message: "Unsafe code detected. Explicit approval required."
  - id: "check-unwrap"
    when: 'contains(diff_added_content, "unwrap()")'
    then:
      action: "require_approval"
      message: "Usage of unwrap() detected. Please use expect() or handle errors properly."
      required: 1
      roles: ["maintainer"]
  - id: "block-todo"
    when: 'contains(diff_added_content, "todo!(")'
    then:
      action: "block"
      message: "Production code cannot contain todo!() macros."
  - id: "block-dbg"
    when: 'contains(diff_added_content, "dbg!(")'
    then:
      action: "block"
      message: "Debug prints (dbg!) are not allowed in production."
  - id: "block-panic"
    when: 'contains(diff_added_content, "panic!(")'
    then:
      action: "block"
      message: "Explicit panic!() calls are forbidden. Use Result/Option instead."
  - id: "protect-policy-file"
    when: 'contains(diff_file_names, "review.policy.yaml")'
    then:
      action: "require_approval"
      message: "Changes to the governance policy require Admin approval."
      required: 1
      roles: ["admin"]
  - id: "database-migration-check"
    when: 'contains(diff_file_names, "migrations/") && contains(diff_file_names, ".sql")'
    then:
      action: "require_approval"
      message: "Database migrations require DBA approval."
      required: 1
      roles: ["dba"]
  - id: "block-absolute-paths"
    when: 'contains(diff_added_content, "/Users/") || contains(diff_added_content, "/home/")'
    then:
      action: "block"
      message: "Absolute paths detected."
  - id: "block-private-keys"
    when: 'contains(diff_added_content, "BEGIN PRIVATE KEY")'
    then:
      action: "block"
      message: "Private keys detected."
  - id: "block-aws-keys"
    when: 'contains(diff_added_content, "AKIA")'
    then:
      action: "block"
      message: "AWS keys detected."
  - id: "enforce-license-header"
    when: '!all_match(new_file_contents, "Copyright 202.")'
    then:
      action: "block"
      message: "All new files must contain a Copyright 202x header."
  - id: "enforce-test-coverage"
    when: 'missing_tests(new_file_paths)'
    then:
      action: "block"
      message: "New source files must have a corresponding test file."
  - id: "block-empty-message"
    when: 'commit_message == ""'
    then:
      action: "block"
      message: "Commit message cannot be empty."
"#;

        // Ensure serde_yaml is in your Cargo.toml [dev-dependencies]
        let policy: ReviewPolicy = serde_yaml::from_str(yaml).expect("Failed to parse YAML");

        // Case 1: Default deny (conditions don't match)
        let signals_safe = Signals {
            diff_files_changed: 10,
            diff_lines_added: 100,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "Fix documentation".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_safe = policy.evaluate(&signals_safe);
        assert_eq!(decision_safe.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(decision_safe.matched_rule_id, None);

        // Case 2: Block rule matches (expression evaluation)
        let signals_risky = Signals {
            diff_files_changed: 10,
            diff_lines_added: 1,
            diff_lines_deleted: 1,
            risk_class: "destructive".to_string(),
            diff_file_names: String::new(),
            commit_message: "Nuke database".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_risky = policy.evaluate(&signals_risky);
        assert_eq!(decision_risky.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_risky.matched_rule_id,
            Some("block-secrets".to_string())
        );
        assert!(decision_risky.message.contains("Destructive changes"));

        // Case 3: Allow rule matches (expression evaluation: 3 < 5)
        let signals_small = Signals {
            diff_files_changed: 3,
            diff_lines_added: 10,
            diff_lines_deleted: 2,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "Small fix".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_small = policy.evaluate(&signals_small);
        assert_eq!(decision_small.decision, DecisionOutcome::Allowed);
        assert_eq!(
            decision_small.matched_rule_id,
            Some("allow-documentation".to_string())
        );

        // Case 4: Large change set
        let signals_large = Signals {
            diff_files_changed: 12,
            diff_lines_added: 10,
            diff_lines_deleted: 2,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "Big change".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_large = policy.evaluate(&signals_large);
        assert_eq!(decision_large.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_large.matched_rule_id,
            Some("block-large-changes".to_string())
        );

        // Case 5: Large line additions
        let signals_lines = Signals {
            diff_files_changed: 6,
            diff_lines_added: 600,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "Add lots of lines".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_lines = policy.evaluate(&signals_lines);
        assert_eq!(decision_lines.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(
            decision_lines.matched_rule_id,
            Some("approval-large-additions".to_string())
        );

        // Case 6: Net deletion
        let signals_deletion = Signals {
            diff_files_changed: 6,
            diff_lines_added: 10,
            diff_lines_deleted: 20,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "Delete stuff".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_deletion = policy.evaluate(&signals_deletion);
        assert_eq!(decision_deletion.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_deletion.matched_rule_id,
            Some("block-net-deletion".to_string())
        );

        // Case 7: Net deletion with refactor (should bypass block)
        let signals_refactor = Signals {
            diff_files_changed: 6,
            diff_lines_added: 10,
            diff_lines_deleted: 20,
            risk_class: "refactor".to_string(),
            diff_file_names: String::new(),
            commit_message: "Refactor core".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_refactor = policy.evaluate(&signals_refactor);
        assert_eq!(decision_refactor.decision, DecisionOutcome::Allowed);
        assert_eq!(
            decision_refactor.matched_rule_id,
            Some("allow-refactor".to_string())
        );

        // Case 8: Auth directory changes
        let signals_auth = Signals {
            diff_files_changed: 6,
            diff_lines_added: 5,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/auth/login.rs".to_string(),
            commit_message: "Update login".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_auth = policy.evaluate(&signals_auth);
        assert_eq!(decision_auth.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(
            decision_auth.matched_rule_id,
            Some("auth-security-check".to_string())
        );
        assert_eq!(decision_auth.requirements.unwrap().required, 2);

        // Case 9: Secret file changes
        let signals_secrets = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: ".env".to_string(),
            commit_message: "Add secrets".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_secrets = policy.evaluate(&signals_secrets);
        assert_eq!(decision_secrets.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_secrets.matched_rule_id,
            Some("block-secret-files".to_string())
        );

        // Case 10: WIP in commit message
        let signals_wip = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "WIP: initial implementation".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_wip = policy.evaluate(&signals_wip);
        assert_eq!(decision_wip.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(decision_wip.matched_rule_id, Some("wip-check".to_string()));

        // Case 11: Regex match for conventional commits
        let signals_feat = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: String::new(),
            commit_message: "feat: add regex support".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_feat = policy.evaluate(&signals_feat);
        assert_eq!(decision_feat.decision, DecisionOutcome::Allowed);
        assert_eq!(
            decision_feat.matched_rule_id,
            Some("conventional-commit-check".to_string())
        );

        // Case 12: Lockfile sync check
        let signals_lock = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "Cargo.toml".to_string(), // Missing Cargo.lock
            commit_message: "Update deps".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_lock = policy.evaluate(&signals_lock);
        assert_eq!(decision_lock.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_lock.matched_rule_id,
            Some("sync-cargo-lock".to_string())
        );

        // Case 13: Unsafe code check
        let signals_unsafe = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/lib.rs".to_string(),
            commit_message: "Optimize".to_string(),
            diff_added_content: "fn fast() { unsafe { ... } }".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_unsafe = policy.evaluate(&signals_unsafe);
        assert_eq!(decision_unsafe.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_unsafe.matched_rule_id,
            Some("block-unsafe".to_string())
        );

        // Case 14: Unwrap usage check
        let signals_unwrap = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/main.rs".to_string(),
            commit_message: "Quick fix".to_string(),
            diff_added_content: "let val = option.unwrap();".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_unwrap = policy.evaluate(&signals_unwrap);
        assert_eq!(decision_unwrap.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(
            decision_unwrap.matched_rule_id,
            Some("check-unwrap".to_string())
        );

        // Case 15: Todo macro check
        let signals_todo = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/lib.rs".to_string(),
            commit_message: "Implement feature".to_string(),
            diff_added_content: "fn foo() { todo!() }".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_todo = policy.evaluate(&signals_todo);
        assert_eq!(decision_todo.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_todo.matched_rule_id,
            Some("block-todo".to_string())
        );

        // Case 16: Dbg macro check
        let signals_dbg = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/lib.rs".to_string(),
            commit_message: "Debug info".to_string(),
            diff_added_content: "dbg!(x);".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_dbg = policy.evaluate(&signals_dbg);
        assert_eq!(decision_dbg.decision, DecisionOutcome::Blocked);
        assert_eq!(decision_dbg.matched_rule_id, Some("block-dbg".to_string()));

        // Case 17: Panic macro check
        let signals_panic = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/lib.rs".to_string(),
            commit_message: "Crash handler".to_string(),
            diff_added_content: "if err { panic!(\"boom\") }".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_panic = policy.evaluate(&signals_panic);
        assert_eq!(decision_panic.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_panic.matched_rule_id,
            Some("block-panic".to_string())
        );

        // Case 18: Policy file protection
        let signals_policy = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "review.policy.yaml".to_string(),
            commit_message: "Update policy".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_policy = policy.evaluate(&signals_policy);
        assert_eq!(decision_policy.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(
            decision_policy.matched_rule_id,
            Some("protect-policy-file".to_string())
        );

        // Case 19: Database migration check
        let signals_db = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "migrations/001_init.sql".to_string(),
            commit_message: "Add users table".to_string(),
            diff_added_content: "CREATE TABLE users...".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_db = policy.evaluate(&signals_db);
        assert_eq!(decision_db.decision, DecisionOutcome::ApprovalRequired);
        assert_eq!(
            decision_db.matched_rule_id,
            Some("database-migration-check".to_string())
        );

        // Case 20: Absolute paths check
        let signals_abs = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "config.rs".to_string(),
            commit_message: "Add config".to_string(),
            diff_added_content: "let path = \"/Users/shailesh/project\";".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_abs = policy.evaluate(&signals_abs);
        assert_eq!(decision_abs.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_abs.matched_rule_id,
            Some("block-absolute-paths".to_string())
        );

        // Case 21: Private key check
        let signals_key = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "secrets.rs".to_string(),
            commit_message: "Add key".to_string(),
            diff_added_content: "-----BEGIN PRIVATE KEY-----".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_key = policy.evaluate(&signals_key);
        assert_eq!(decision_key.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_key.matched_rule_id,
            Some("block-private-keys".to_string())
        );

        // Case 22: AWS key check
        let signals_aws = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "aws.rs".to_string(),
            commit_message: "Add aws".to_string(),
            diff_added_content: "let key = \"AKIAIOSFODNN7EXAMPLE\";".to_string(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_aws = policy.evaluate(&signals_aws);
        assert_eq!(decision_aws.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_aws.matched_rule_id,
            Some("block-aws-keys".to_string())
        );

        // Case 23: Missing license header in new file
        let signals_license = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "new_file.rs".to_string(),
            commit_message: "Add file".to_string(),
            diff_added_content: "fn main() {}".to_string(),
            new_file_contents: vec!["fn main() {}".to_string()],
            new_file_paths: vec!["new_file.rs".to_string()],
        };
        let decision_license = policy.evaluate(&signals_license);
        assert_eq!(decision_license.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_license.matched_rule_id,
            Some("enforce-license-header".to_string())
        );

        // Case 24: Missing test file
        let signals_tests = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "src/logic.rs".to_string(),
            commit_message: "Add logic".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: vec!["src/logic.rs".to_string()], // No corresponding test file
        };
        let decision_tests = policy.evaluate(&signals_tests);
        assert_eq!(decision_tests.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_tests.matched_rule_id,
            Some("enforce-test-coverage".to_string())
        );

        // Case 25: Empty commit message
        let signals_empty_msg = Signals {
            diff_files_changed: 6,
            diff_lines_added: 1,
            diff_lines_deleted: 0,
            risk_class: "patch-only".to_string(),
            diff_file_names: "file.rs".to_string(),
            commit_message: "".to_string(),
            diff_added_content: String::new(),
            new_file_contents: Vec::new(),
            new_file_paths: Vec::new(),
        };
        let decision_empty_msg = policy.evaluate(&signals_empty_msg);
        assert_eq!(decision_empty_msg.decision, DecisionOutcome::Blocked);
        assert_eq!(
            decision_empty_msg.matched_rule_id,
            Some("block-empty-message".to_string())
        );
    }
}
