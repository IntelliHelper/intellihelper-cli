//! `/token-usage` — day / week / month token spend by model and totals.
//!
//! Available with or without an active session (not session-scoped). Reads the
//! durable local store written on each model call.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct TokenUsageCommand;

impl SlashCommand for TokenUsageCommand {
    fn name(&self) -> &str {
        "token-usage"
    }

    fn aliases(&self) -> &[&str] {
        &["tokens"]
    }

    fn description(&self) -> &str {
        "View token usage by day, week, and month"
    }

    fn usage(&self) -> &str {
        "/token-usage"
    }

    fn session_scoped(&self) -> bool {
        false
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::ShowTokenUsage)
    }
}
