//! Helpers for the composer "Optimize input" action.
//!
//! The rewrite itself is an async LLM side-call (`x.ai/optimize_prompt`).
//! This module only validates empty drafts and sanitizes model output so a
//! misbehaving model cannot leave Goal/Requirements boilerplate in the box.

/// Reject empty drafts before issuing a network call.
pub fn validate_draft(draft: &str) -> Result<(), &'static str> {
    if draft.trim().is_empty() {
        Err("Nothing to optimize — type a draft first.")
    } else {
        Ok(())
    }
}

/// Clean model output: strip fences, meta openers, and accidental Goal templates.
pub fn sanitize_model_output(raw: &str) -> Result<String, &'static str> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return Err("Empty optimized text");
    }

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

    if looks_like_goal_template(&s) {
        s = strip_goal_template(&s);
    }

    s = s.trim().to_string();
    if (s.starts_with('"') && s.ends_with('"') && s.len() > 1)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() > 1)
    {
        s = s[1..s.len() - 1].trim().to_string();
    }
    if s.is_empty() {
        return Err("Empty optimized text");
    }
    // Keep composer usable: hard cap.
    if s.chars().count() > 2_000 {
        s = s.chars().take(1_999).collect();
        s.push('…');
    }
    Ok(s)
}

fn looks_like_goal_template(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("## goal")
        || lower.contains("### goal")
        || (lower.contains("## requirements") && lower.contains("acceptance"))
}

fn strip_goal_template(text: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_draft_is_error() {
        assert!(validate_draft("   ").is_err());
        assert!(validate_draft("hello").is_ok());
    }

    #[test]
    fn sanitize_strips_goal_boilerplate() {
        let raw = r#"## Goal
what is this project

## Context
what is this project

## Requirements
- Implement the change surgically; avoid unrelated refactors.

## Acceptance criteria
- [ ] Behavior matches the goal above
"#;
        let out = sanitize_model_output(raw).unwrap();
        assert!(!out.to_ascii_lowercase().contains("## goal"));
        assert!(!out.contains("Implement the change surgically"));
        assert!(out.to_ascii_lowercase().contains("what is this project"));
    }

    #[test]
    fn sanitize_natural_prose_passthrough() {
        let raw = "What are the changes made in this branch compared to main?";
        assert_eq!(sanitize_model_output(raw).unwrap(), raw);
    }
}
