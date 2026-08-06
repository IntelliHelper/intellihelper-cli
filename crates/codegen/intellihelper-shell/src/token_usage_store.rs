//! Durable per-user token usage history for day / week / month reports.
//!
//! Append-only JSONL at `$INTELLIHELPER_HOME/token-usage.jsonl`. Each line is one model
//! call (main-loop). Incomplete bills must not invent tokens; callers only
//! append when real usage was observed.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};

const STORE_FILE: &str = "token-usage.jsonl";
/// Keep roughly 400 days so month windows always have room after restarts.
const RETENTION_DAYS: i64 = 400;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageEvent {
    /// Unix seconds (UTC).
    pub ts: i64,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cached_read_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageBucket {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_read_tokens: u64,
    pub reasoning_tokens: u64,
    pub model_calls: u64,
    pub by_model: BTreeMap<String, ModelBucket>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelBucket {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_read_tokens: u64,
    pub reasoning_tokens: u64,
    pub model_calls: u64,
}

impl UsageBucket {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    fn fold_event(&mut self, e: &TokenUsageEvent) {
        self.input_tokens = self.input_tokens.saturating_add(e.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(e.output_tokens);
        self.cached_read_tokens = self
            .cached_read_tokens
            .saturating_add(e.cached_read_tokens);
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(e.reasoning_tokens);
        self.model_calls = self.model_calls.saturating_add(1);
        let m = self.by_model.entry(e.model.clone()).or_default();
        m.input_tokens = m.input_tokens.saturating_add(e.input_tokens);
        m.output_tokens = m.output_tokens.saturating_add(e.output_tokens);
        m.cached_read_tokens = m.cached_read_tokens.saturating_add(e.cached_read_tokens);
        m.reasoning_tokens = m.reasoning_tokens.saturating_add(e.reasoning_tokens);
        m.model_calls = m.model_calls.saturating_add(1);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsageReport {
    pub day: UsageBucket,
    pub week: UsageBucket,
    pub month: UsageBucket,
    /// Local-timezone labels for the report header.
    pub day_label: String,
    pub week_label: String,
    pub month_label: String,
}

fn store_path() -> PathBuf {
    intellihelper_config::intellihelper_home().join(STORE_FILE)
}

/// Append one model-call event. Best-effort; failures are logged and ignored.
pub fn record_model_call(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached_read_tokens: u64,
    reasoning_tokens: u64,
) {
    if model.is_empty() && input_tokens == 0 && output_tokens == 0 {
        return;
    }
    let event = TokenUsageEvent {
        ts: Utc::now().timestamp(),
        model: if model.is_empty() {
            "unknown".into()
        } else {
            model.to_owned()
        },
        input_tokens,
        output_tokens,
        cached_read_tokens,
        reasoning_tokens,
    };
    if let Err(e) = append_event(&store_path(), &event) {
        tracing::debug!(error = %e, "token usage store: append failed");
    }
}

fn append_event(path: &Path, event: &TokenUsageEvent) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut f, event).map_err(std::io::Error::other)?;
    f.write_all(b"\n")?;
    Ok(())
}

/// Load and aggregate events for local calendar day / week / month.
pub fn load_report() -> TokenUsageReport {
    load_report_at(Local::now(), &store_path())
}

fn load_report_at(now_local: chrono::DateTime<Local>, path: &Path) -> TokenUsageReport {
    let (day_start, week_start, month_start, day_label, week_label, month_label) =
        window_bounds(now_local);
    let day_start_ts = day_start.timestamp();
    let week_start_ts = week_start.timestamp();
    let month_start_ts = month_start.timestamp();
    let retention_ts = (now_local - chrono::Duration::days(RETENTION_DAYS)).timestamp();

    let mut report = TokenUsageReport {
        day_label,
        week_label,
        month_label,
        ..Default::default()
    };

    let Ok(file) = File::open(path) else {
        return report;
    };
    let reader = BufReader::new(file);
    let mut keep_lines: Vec<String> = Vec::new();
    let mut needs_prune = false;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<TokenUsageEvent>(trimmed) else {
            needs_prune = true;
            continue;
        };
        if event.ts < retention_ts {
            needs_prune = true;
            continue;
        }
        keep_lines.push(trimmed.to_string());
        if event.ts >= month_start_ts {
            report.month.fold_event(&event);
        }
        if event.ts >= week_start_ts {
            report.week.fold_event(&event);
        }
        if event.ts >= day_start_ts {
            report.day.fold_event(&event);
        }
    }

    if needs_prune {
        let _ = rewrite_store(path, &keep_lines);
    }

    report
}

fn rewrite_store(path: &Path, lines: &[String]) -> std::io::Result<()> {
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut f = File::create(&tmp)?;
        for line in lines {
            f.write_all(line.as_bytes())?;
            f.write_all(b"\n")?;
        }
    }
    std::fs::rename(tmp, path)
}

/// Local calendar windows: day = today midnight→now; week = Monday 00:00→now;
/// month = first of month 00:00→now.
fn window_bounds(
    now: chrono::DateTime<Local>,
) -> (
    chrono::DateTime<Local>,
    chrono::DateTime<Local>,
    chrono::DateTime<Local>,
    String,
    String,
    String,
) {
    let date = now.date_naive();
    let day_start = Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap_or(now);
    let weekday = date.weekday().num_days_from_monday() as i64;
    let week_date = date - chrono::Duration::days(weekday);
    let week_start = Local
        .from_local_datetime(&week_date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap_or(now);
    let month_date = chrono::NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date);
    let month_start = Local
        .from_local_datetime(&month_date.and_hms_opt(0, 0, 0).unwrap())
        .single()
        .unwrap_or(now);

    let day_label = date.format("%Y-%m-%d (local)").to_string();
    let week_label = format!(
        "{} – {} (local week, Mon start)",
        week_date.format("%Y-%m-%d"),
        date.format("%Y-%m-%d")
    );
    let month_label = date.format("%Y-%m (local)").to_string();

    (
        day_start,
        week_start,
        month_start,
        day_label,
        week_label,
        month_label,
    )
}

/// Human-readable multi-window report for `/token-usage`.
pub fn format_report(report: &TokenUsageReport) -> String {
    let mut out = String::new();
    out.push_str("Token usage (local calendar windows)\n");
    out.push_str(&format_window("Day", &report.day_label, &report.day));
    out.push('\n');
    out.push_str(&format_window("Week", &report.week_label, &report.week));
    out.push('\n');
    out.push_str(&format_window("Month", &report.month_label, &report.month));
    out
}

fn format_window(name: &str, label: &str, bucket: &UsageBucket) -> String {
    // Bullet list (·) matches session `/usage` and reads like a compact tree —
    // easy to scan for Input / Output / cached / reasoning.
    let mut rows = Vec::new();
    rows.push(format!("{name} — {label}"));
    if bucket.model_calls == 0 {
        rows.push("  · Input:    0 (0 cached)".to_string());
        rows.push("  · Output:   0 (0 reasoning)".to_string());
        rows.push("  · Total:    0 · calls: 0".to_string());
        rows.push("  No model calls recorded.".to_string());
        return rows.join("\n");
    }
    rows.push(format!(
        "  · Input:    {} ({} cached)",
        group_thousands(bucket.input_tokens),
        group_thousands(bucket.cached_read_tokens)
    ));
    rows.push(format!(
        "  · Output:   {} ({} reasoning)",
        group_thousands(bucket.output_tokens),
        group_thousands(bucket.reasoning_tokens)
    ));
    rows.push(format!(
        "  · Total:    {} · calls: {}",
        group_thousands(bucket.total_tokens()),
        group_thousands(bucket.model_calls)
    ));
    if !bucket.by_model.is_empty() {
        rows.push("  By model:".to_string());
        for (model, m) in &bucket.by_model {
            rows.push(format!(
                "    · {model} — {} in / {} out · {} total · {} calls",
                group_thousands(m.input_tokens),
                group_thousands(m.output_tokens),
                group_thousands(m.input_tokens.saturating_add(m.output_tokens)),
                group_thousands(m.model_calls)
            ));
        }
    }
    rows.join("\n")
}

fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_path() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("intellihelper-token-usage-test-{n}-{}.jsonl", std::process::id()))
    }

    #[test]
    fn append_and_aggregate_windows() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let now = Local::now();
        let day_ts = now.timestamp();
        let old_ts = (now - chrono::Duration::days(40)).timestamp();

        append_event(
            &path,
            &TokenUsageEvent {
                ts: day_ts,
                model: "a".into(),
                input_tokens: 100,
                output_tokens: 10,
                cached_read_tokens: 5,
                reasoning_tokens: 2,
            },
        )
        .unwrap();
        append_event(
            &path,
            &TokenUsageEvent {
                ts: day_ts,
                model: "b".into(),
                input_tokens: 50,
                output_tokens: 5,
                cached_read_tokens: 0,
                reasoning_tokens: 0,
            },
        )
        .unwrap();
        append_event(
            &path,
            &TokenUsageEvent {
                ts: old_ts,
                model: "a".into(),
                input_tokens: 999,
                output_tokens: 999,
                cached_read_tokens: 0,
                reasoning_tokens: 0,
            },
        )
        .unwrap();

        let report = load_report_at(now, &path);
        assert_eq!(report.day.input_tokens, 150);
        assert_eq!(report.day.output_tokens, 15);
        assert_eq!(report.day.model_calls, 2);
        assert_eq!(report.day.by_model["a"].input_tokens, 100);
        assert_eq!(report.day.by_model["b"].input_tokens, 50);
        // 40-day-old event is outside day/week but may be in a long month? No, 40 days > month.
        assert_eq!(report.month.input_tokens, 150);

        let text = format_report(&report);
        assert!(text.contains("By model:"));
        assert!(text.contains("a —"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_store_is_empty_report() {
        let path = temp_path();
        let _ = std::fs::remove_file(&path);
        let report = load_report_at(Local::now(), &path);
        assert_eq!(report.day.model_calls, 0);
        let text = format_report(&report);
        assert!(text.contains("No model calls recorded"));
    }
}
