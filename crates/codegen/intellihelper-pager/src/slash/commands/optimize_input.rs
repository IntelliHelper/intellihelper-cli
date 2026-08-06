//! `/optimize-input` — rewrite the composer draft for a clearer coding prompt.
//!
//! Does not send a turn. Replaces the draft in place.

use crate::app::actions::Action;
use crate::slash::command::{CommandExecCtx, CommandResult, SlashCommand};

pub struct OptimizeInputCommand;

impl SlashCommand for OptimizeInputCommand {
    fn name(&self) -> &str {
        "optimize-input"
    }

    fn aliases(&self) -> &[&str] {
        &["optimize"]
    }

    fn description(&self) -> &str {
        "Optimize the current draft (does not send)"
    }

    fn usage(&self) -> &str {
        "/optimize-input"
    }

    fn session_scoped(&self) -> bool {
        // Available on welcome (after session create) and in-session; the
        // action itself operates on the active composer.
        false
    }

    fn run(&self, _ctx: &mut CommandExecCtx, _args: &str) -> CommandResult {
        CommandResult::Action(Action::OptimizeInput)
    }
}
