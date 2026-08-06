//! Composer draft rewrite for the Optimize-input sparkle (Qoder / Roo / Cline style).
//!
//! One tool-free model call expands a rough draft into clear natural-language
//! prose the user can edit and send. Industry references:
//! - **Roo Code / Cline ENHANCE**: single-completion rewrite, output-only enhanced
//!   prompt (no lead-in, no bullet scaffolding).
//! - **Qoder Prompt Enhancement**: natural multi-sentence expansion + Undo.
//! - **PromptDC Simple mode**: one-paragraph coding-first rewrite (not ROLE/PLAN
//!   structured templates).
//!
//! Design goals beyond those:
//! - Preserve intent; never pivot the topic to git status/diff unless the draft
//!   is about branch/changes/commits.
//! - Light always-on workspace identity (cwd, branch, package); heavy git stats
//!   only when the draft is change-related.
//! - Cover common coding-agent scenarios (ask, implement, debug, review, explain).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Hard cap on the rewritten prompt returned to the client.
pub const OPTIMIZED_MAX_CHARS: usize = 2_000;

/// Cap on workspace context injected into the model request.
const CONTEXT_MAX_CHARS: usize = 2_400;

/// Cap on the raw draft sent to the model.
const DRAFT_MAX_CHARS: usize = 4_000;

/// System prompt: natural-prose enhancer, not a Goal/Requirements templater.
///
/// Informed by Roo/Cline's ENHANCE template (output-only rewrite) and Qoder's
/// natural expansion, with explicit anti-hijack rules for workspace context.
pub const OPTIMIZE_SYSTEM_PROMPT: &str = concat!(
    "You enhance rough drafts into clearer prompts for a coding agent (CLI/IDE).\n\n",
    "OUTPUT (strict):\n",
    "- Reply with ONLY the enhanced prompt — no conversation, no explanations, no lead-in, ",
    "no surrounding quotes, no code fences.\n",
    "- Natural language only: 1–3 short paragraphs or a clear multi-sentence question/instruction.\n",
    "- Do NOT use markdown section headers (## Goal, ## Context, ## Requirements, ",
    "## Acceptance criteria, ROLE, OBJECTIVE, SCOPE, PLAN, etc.).\n",
    "- Do NOT paste boilerplate requirement lists or checkbox acceptance criteria.\n",
    "- Do NOT invent file paths, APIs, function names, tickets, or commits not present in the draft ",
    "or in workspace context you are allowed to use.\n\n",
    "INTENT (highest priority):\n",
    "- Preserve the user's goal, constraints, tone, and scope.\n",
    "- Expand vague wording for clarity; do not change what they are asking for.\n",
    "- If the draft is already clear and specific, only lightly polish (fix grammar, ",
    "tighten wording). Do not bloat it.\n",
    "- If the draft is a short question, keep it a question; if it is a task, keep it a task.\n\n",
    "WORKSPACE CONTEXT (critical):\n",
    "- Workspace context is BACKGROUND identity only unless the draft is clearly about ",
    "repo/branch/diff/commits/local changes/PR review.\n",
    "- NEVER rewrite an unrelated draft into \"summarize local changes / git status / ",
    "working tree under crates/...\" just because status or diff is present.\n",
    "- When the draft IS about branch or local changes, you may name the current branch and ",
    "default base (main/master) from context and ask for features, fixes, additions, deletions, ",
    "and functional impact — like a natural Qoder-style expansion, not a raw file dump.\n",
    "- Do not list every dirty path from git status unless the user asked for file-level detail.\n\n",
    "SCENARIOS (match and enhance accordingly):\n",
    "1) Question / explore (\"what is this project\", \"how does X work\"): clearer question; ",
    "optional ask for architecture overview from the codebase — not an implementation plan.\n",
    "2) Implement / feature (\"add login\", \"wire optimize sparkle\"): clear task with expected ",
    "behavior and constraints; stay surgical; match existing patterns when useful.\n",
    "3) Bug / fix (\"broken\", \"doesn't work\"): restate symptom, expected vs actual if implied, ",
    "ask to diagnose and fix without inventing root causes.\n",
    "4) Review / branch / changes (\"what changed on this branch\", \"diff vs main\"): natural ",
    "request to summarize modifications vs the default base — features, fixes, functional impact.\n",
    "5) Refactor / cleanup: goals (readability, structure) and what not to break.\n",
    "6) Explain / docs: ask for clear explanation or doc update scoped to the topic.\n",
    "7) Multi-line or already structured drafts: preserve structure and content; fix only clarity.\n",
    "8) Non-English or mixed language: keep the user's language unless they mixed casually; ",
    "do not force English.\n\n",
    "STYLE:\n",
    "- Concise, specific, developer-to-agent voice.\n",
    "- Prefer one coherent paragraph or a few sentences over long templates.\n",
    "- Prefer actionable detail over filler (\"please\", \"kindly\", generic acceptance checklists).\n",
);

/// Keywords / patterns that mean the draft is about repo changes — safe to attach
/// git status / diff stats for code-aware expansion.
pub fn draft_wants_change_context(draft: &str) -> bool {
    let lower = draft.to_ascii_lowercase();
    // Multi-word phrases (substring OK — long enough to avoid false positives).
    const PHRASES: &[&str] = &[
        "this branch",
        "current branch",
        "my branch",
        "on the branch",
        "on this branch",
        "vs main",
        "vs master",
        "versus main",
        "compared to main",
        "compare to main",
        "against main",
        "origin/main",
        "origin/master",
        "pull request",
        "code review",
        "what changed",
        "what are the changes",
        "what's changed",
        "whats changed",
        "changes in this",
        "changes on this",
        "local changes",
        "working tree",
        "working directory",
        "staged changes",
        "git status",
        "git diff",
        "diff stat",
        "commit message",
        "release notes",
        "what did i change",
        "what have i changed",
        "summarize changes",
        "summarise changes",
        "dirty files",
        "modified files",
    ];
    if PHRASES.iter().any(|n| lower.contains(n)) {
        return true;
    }
    // Token match with whole-token equality (never match "pr" inside "project").
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '/')
        .filter(|t| !t.is_empty())
        .collect();
    for t in &tokens {
        match *t {
            "diff" | "diffs" | "pr" | "prs" | "commits" | "commit" | "branch" | "branches"
            | "changelog" | "uncommitted" | "staged" | "unstaged" => return true,
            _ => {}
        }
    }
    // "review the/my/this …" without matching "preview".
    if tokens.windows(2).any(|w| {
        w[0] == "review" && matches!(w[1], "the" | "my" | "this" | "our" | "code" | "changes")
    }) {
        return true;
    }
    false
}

/// Build the user message: draft + optional workspace context with anti-hijack rules.
pub fn build_user_message(draft: &str, workspace_context: &str) -> String {
    let draft = truncate_chars(draft.trim(), DRAFT_MAX_CHARS);
    if workspace_context.trim().is_empty() {
        format!(
            "Generate an enhanced version of this prompt. Reply with only the enhanced prompt.\n\n\
             <draft>\n{draft}\n</draft>"
        )
    } else {
        let ctx = truncate_chars(workspace_context.trim(), CONTEXT_MAX_CHARS);
        format!(
            "Generate an enhanced version of this prompt. Reply with only the enhanced prompt.\n\n\
             Rules for context:\n\
             - <workspace_context> is optional background (identity / code-aware hints).\n\
             - Do not change the draft's topic to git status, working-tree files, or \"summarize local changes\" \
             unless the draft itself is about branch/changes/diffs/commits/PR/review.\n\
             - When the draft is about changes, you may use branch and base names from context; \
             do not dump raw path lists.\n\
             - Never invent files or APIs not in the draft or context.\n\n\
             <draft>\n{draft}\n</draft>\n\n\
             <workspace_context>\n{ctx}\n</workspace_context>"
        )
    }
}

/// Gather a bounded snapshot of cwd + identity, and optionally git change stats
/// when `draft` indicates change/branch/PR intent.
pub fn gather_workspace_context(cwd: &Path, draft: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("cwd: {}", cwd.display()));

    if let Some(branch) = git_stdout(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        parts.push(format!("branch: {}", branch.trim()));
    }
    if let Some(base) = detect_default_base(cwd) {
        parts.push(format!("default_base: {base}"));
    }
    if let Some(pkg) = read_package_hint(cwd) {
        parts.push(pkg);
    }

    // Recent commits: small signal for branch work without dumping the tree.
    if let Some(log) = git_stdout(cwd, &["log", "-5", "--oneline", "--no-decorate"]) {
        let log = truncate_chars(log.trim(), 400);
        if !log.is_empty() {
            parts.push(format!("recent commits (HEAD):\n{log}"));
        }
    }

    if draft_wants_change_context(draft) {
        parts.push(
            "note: draft appears change/branch-related — status and diff stats included for grounding."
                .to_string(),
        );
        if let Some(status) = git_stdout(cwd, &["status", "-sb"]) {
            let status = truncate_chars(status.trim(), 600);
            if !status.is_empty() {
                parts.push(format!("git status (abbrev):\n{status}"));
            }
        }
        let base = detect_default_base(cwd);
        let diff_args: Vec<&str> = if let Some(ref b) = base {
            vec!["diff", "--stat", b.as_str(), "HEAD"]
        } else {
            vec!["diff", "--stat", "HEAD"]
        };
        if let Some(stat) = git_stdout(cwd, &diff_args) {
            let stat = truncate_chars(stat.trim(), 900);
            if !stat.is_empty() {
                let label = base
                    .as_deref()
                    .map(|b| format!("git diff --stat {b}...HEAD"))
                    .unwrap_or_else(|| "git diff --stat HEAD".to_string());
                parts.push(format!("{label}:\n{stat}"));
            }
        }
        // Also unstaged working tree summary vs index (common for "local changes").
        if let Some(wt) = git_stdout(cwd, &["diff", "--stat"]) {
            let wt = truncate_chars(wt.trim(), 600);
            if !wt.is_empty() {
                parts.push(format!("git diff --stat (working tree):\n{wt}"));
            }
        }
    } else {
        parts.push(
            "note: draft is NOT change/branch-related — omit status/diff; do not pivot topic to git."
                .to_string(),
        );
    }

    truncate_chars(&parts.join("\n\n"), CONTEXT_MAX_CHARS)
}

fn detect_default_base(cwd: &Path) -> Option<String> {
    for candidate in ["main", "master", "origin/main", "origin/master"] {
        if git_stdout(cwd, &["rev-parse", "--verify", candidate]).is_some() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn read_package_hint(cwd: &Path) -> Option<String> {
    let cargo = cwd.join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&cargo) {
        for line in text.lines().take(40) {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("name") {
                let rest = rest.trim().trim_start_matches('=').trim();
                let name = rest.trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    return Some(format!("package (Cargo.toml name): {name}"));
                }
            }
        }
    }
    let pkg = cwd.join("package.json");
    if let Ok(text) = std::fs::read_to_string(&pkg) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(name) = v.get("name").and_then(|n| n.as_str())
        {
            return Some(format!("package (package.json name): {name}"));
        }
    }
    let readme = ["README.md", "README", "readme.md"]
        .iter()
        .map(|n| cwd.join(n))
        .find(|p| p.is_file());
    if let Some(path) = readme
        && let Ok(text) = std::fs::read_to_string(path)
    {
        let head: String = text.lines().take(6).collect::<Vec<_>>().join("\n");
        let head = truncate_chars(head.trim(), 280);
        if !head.is_empty() {
            return Some(format!("README head:\n{head}"));
        }
    }
    None
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // Cheap bound: if git hangs, drop after a short wait.
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = String::new();
                if let Some(mut stdout) = child.stdout.take() {
                    use std::io::Read;
                    let _ = stdout.read_to_string(&mut out);
                }
                return if out.trim().is_empty() {
                    None
                } else {
                    Some(out)
                };
            }
            Ok(Some(_)) => return None,
            Ok(None) if start.elapsed() > Duration::from_millis(800) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
}

/// Clean model output: strip fences, accidental Goal/Requirements sections,
/// leading meta lines; cap length.
pub fn sanitize_optimized(raw: &str) -> Result<String, &'static str> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return Err("empty model response");
    }

    // Strip a single outer ``` fence.
    if s.starts_with("```") {
        if let Some(rest) = s.strip_prefix("```") {
            let rest = rest
                .strip_prefix("markdown")
                .or_else(|| rest.strip_prefix("md"))
                .or_else(|| rest.strip_prefix("text"))
                .unwrap_or(rest)
                .trim_start_matches('\n');
            if let Some(end) = rest.rfind("```") {
                s = rest[..end].trim().to_string();
            } else {
                s = rest.trim().to_string();
            }
        }
    }

    // Drop common meta openers.
    for prefix in [
        "Here is the optimized prompt:",
        "Here's the optimized prompt:",
        "Here is the enhanced prompt:",
        "Here's the enhanced prompt:",
        "Enhanced prompt:",
        "Optimized prompt:",
        "Sure,",
        "Certainly,",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start().to_string();
        }
    }

    // If the model still emitted the forbidden template, strip those sections
    // and keep the first non-empty Goal/body line expanded as prose when present.
    if looks_like_goal_template(&s) {
        s = strip_goal_template(&s);
    }

    // Strip ROLE/OBJECTIVE style structured enhancers (PromptDC structured mode).
    if looks_like_structured_role_template(&s) {
        s = strip_structured_role_template(&s);
    }

    s = s.trim().to_string();
    // Strip surrounding quotes if the whole answer is quoted.
    if (s.starts_with('"') && s.ends_with('"') && s.len() > 1)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() > 1)
    {
        s = s[1..s.len() - 1].trim().to_string();
    }

    if s.is_empty() {
        return Err("empty after sanitize");
    }
    Ok(truncate_chars(&s, OPTIMIZED_MAX_CHARS))
}

fn looks_like_goal_template(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("## goal")
        || lower.contains("### goal")
        || (lower.contains("## requirements") && lower.contains("acceptance"))
}

fn looks_like_structured_role_template(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.contains("**role**") || lower.contains("role:") || lower.starts_with("role\n"))
        && (lower.contains("objective") || lower.contains("scope") || lower.contains("plan"))
}

fn strip_structured_role_template(text: &str) -> String {
    // Prefer OBJECTIVE body if present (inline "Objective: foo" or following lines).
    let mut objective = String::new();
    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        // Strip markdown bold markers for header detection.
        let stripped = lower.replace("**", "");
        let (header_name, rest_after_header) = if let Some((h, r)) = stripped.split_once(':') {
            (h.trim(), r.trim())
        } else {
            (stripped.trim(), "")
        };
        match header_name {
            "role" | "objective" | "scope" | "plan" | "constraints" | "context" => {
                section = match header_name {
                    "objective" => "objective",
                    "role" | "plan" | "constraints" => "skip",
                    "scope" | "context" => "extra",
                    _ => "skip",
                };
                if section == "objective" && !rest_after_header.is_empty() {
                    // Preserve original casing for the body after the colon.
                    if let Some((_, orig_rest)) = trimmed.split_once(':') {
                        let body = orig_rest.trim().trim_matches('*').trim();
                        if !body.is_empty() {
                            if !objective.is_empty() {
                                objective.push(' ');
                            }
                            objective.push_str(body);
                        }
                    }
                }
                continue;
            }
            _ => {}
        }
        if section == "skip" {
            continue;
        }
        if section == "objective" {
            if !objective.is_empty() {
                objective.push(' ');
            }
            objective.push_str(trimmed);
        }
    }
    let out = objective.trim().to_string();
    if out.is_empty() {
        text.lines()
            .map(str::trim)
            .filter(|l| {
                let lower = l.to_ascii_lowercase().replace("**", "");
                let head = lower.split(':').next().unwrap_or("").trim();
                !matches!(
                    head,
                    "role" | "objective" | "scope" | "plan" | "constraints"
                ) && !l.is_empty()
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        out
    }
}

fn strip_goal_template(text: &str) -> String {
    // Prefer content under ## Goal / ## Context as prose; drop Requirements boilerplate.
    let mut goal = String::new();
    let mut context = String::new();
    let mut section = "";
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("## goal") || lower.starts_with("### goal") {
            section = "goal";
            continue;
        }
        if lower.starts_with("## context") || lower.starts_with("### context") {
            section = "context";
            continue;
        }
        if lower.starts_with("## requirements")
            || lower.starts_with("## acceptance")
            || lower.starts_with("## non-goals")
            || lower.starts_with("## approach")
        {
            section = "skip";
            continue;
        }
        if section == "skip" {
            continue;
        }
        if section == "goal" {
            if !goal.is_empty() {
                goal.push(' ');
            }
            goal.push_str(trimmed);
        } else if section == "context" {
            if !context.is_empty() {
                context.push(' ');
            }
            context.push_str(trimmed);
        }
    }
    let mut out = goal.trim().to_string();
    let ctx = context.trim();
    if !ctx.is_empty() && ctx != out {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(ctx);
    }
    if out.is_empty() {
        // Fall back: drop header lines only.
        text.lines()
            .filter(|l| {
                let lower = l.trim().to_ascii_lowercase();
                !lower.starts_with("## ")
                    && !lower.starts_with("### ")
                    && !lower.starts_with("- implement the change surgically")
                    && !lower.starts_with("- match existing project")
                    && !lower.starts_with("- handle error cases")
                    && !lower.starts_with("- [ ]")
            })
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        out
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_fence_and_meta() {
        let raw = "```markdown\nHere is the optimized prompt:\nWhat changed on this branch?\n```";
        let out = sanitize_optimized(raw).unwrap();
        assert_eq!(out, "What changed on this branch?");
    }

    #[test]
    fn sanitize_strips_goal_template() {
        let raw = r#"## Goal
what is this project

## Context
what is this project

## Requirements
- Implement the change surgically; avoid unrelated refactors.

## Acceptance criteria
- [ ] Behavior matches the goal above
"#;
        let out = sanitize_optimized(raw).unwrap();
        assert!(!out.to_ascii_lowercase().contains("## goal"));
        assert!(!out.contains("Implement the change surgically"));
        assert!(out.to_ascii_lowercase().contains("what is this project"));
    }

    #[test]
    fn sanitize_strips_enhanced_meta_and_quotes() {
        let raw = "\"Here's the enhanced prompt:\nFix the login button on mobile.\"";
        // After fence/meta — whole string may still have leading quote variants
        let out = sanitize_optimized("Here's the enhanced prompt:\nFix the login button on mobile.")
            .unwrap();
        assert_eq!(out, "Fix the login button on mobile.");
        let _ = raw;
    }

    #[test]
    fn empty_sanitize_errors() {
        assert!(sanitize_optimized("   ").is_err());
    }

    #[test]
    fn build_user_message_includes_draft_and_anti_hijack() {
        let msg = build_user_message("fix login", "branch: main");
        assert!(msg.contains("fix login"));
        assert!(msg.contains("branch: main"));
        assert!(msg.contains("Do not change the draft's topic to git status"));
        assert!(msg.contains("only the enhanced prompt"));
    }

    #[test]
    fn build_user_message_without_context() {
        let msg = build_user_message("what is this project", "");
        assert!(msg.contains("what is this project"));
        assert!(!msg.contains("workspace_context"));
    }

    #[test]
    fn draft_wants_change_context_branch_questions() {
        assert!(draft_wants_change_context(
            "What are the changes in this branch"
        ));
        assert!(draft_wants_change_context("diff vs main"));
        assert!(draft_wants_change_context("summarize my PR"));
        assert!(draft_wants_change_context("local changes"));
        assert!(draft_wants_change_context("review the commits"));
    }

    #[test]
    fn draft_does_not_want_change_context_for_unrelated() {
        assert!(!draft_wants_change_context("what is this project"));
        assert!(!draft_wants_change_context("fix the login button"));
        assert!(!draft_wants_change_context("how does auth work"));
        assert!(!draft_wants_change_context("add unit tests for parser"));
        // "different" must not trip "diff" token
        assert!(!draft_wants_change_context(
            "explain the different authentication strategies"
        ));
    }

    #[test]
    fn system_prompt_forbids_goal_template_and_git_hijack() {
        let p = OPTIMIZE_SYSTEM_PROMPT.to_ascii_lowercase();
        assert!(p.contains("only the enhanced prompt") || p.contains("only the enhanced"));
        assert!(p.contains("goal"));
        assert!(p.contains("never rewrite") || p.contains("summarize local changes"));
        assert!(p.contains("preserve"));
    }

    #[test]
    fn gather_workspace_context_gates_git_status_on_draft() {
        let cwd = std::env::current_dir().unwrap();
        // Unrelated draft: must not include full status path dump instruction with status
        let light = gather_workspace_context(&cwd, "what is this project");
        assert!(
            light.contains("NOT change/branch-related") || light.contains("omit status"),
            "light context should mark non-change drafts: {light}"
        );
        assert!(
            !light.contains("git status (abbrev)"),
            "must not attach status for unrelated draft: {light}"
        );

        let heavy = gather_workspace_context(&cwd, "What are the changes in this branch");
        assert!(
            heavy.contains("change/branch-related") || heavy.contains("git status"),
            "change drafts should include change context: {heavy}"
        );
        // Identity always present
        assert!(light.contains("cwd:") || light.contains("branch:"));
        assert!(heavy.contains("cwd:") || heavy.contains("branch:"));
    }

    #[test]
    fn sanitize_strips_role_objective_template() {
        let raw = "**Role**: Senior engineer\n**Objective**: Fix the flaky test in auth\n**Plan**:\n1. Reproduce\n2. Fix";
        let out = sanitize_optimized(raw).unwrap();
        assert!(
            out.to_ascii_lowercase().contains("fix the flaky test")
                || out.to_ascii_lowercase().contains("flaky test"),
            "got: {out}"
        );
        assert!(!out.to_ascii_lowercase().contains("**role**"));
    }
}
