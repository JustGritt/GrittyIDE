//! Who initiated something: attribution, not event sourcing (addendum §5).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::ids::AgentRunId;

/// Serialized as a tagged union:
/// `{ "type": "USER" } | { "type": "SYSTEM" } | { "type": "AGENT", "runId": "..." } | { "type": "EXTERNAL" }`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Actor {
    /// The person using Gritty.
    User,
    /// Gritty itself (maintenance, recovery, scheduled work).
    System,
    /// A supervised agent run.
    Agent {
        #[serde(rename = "runId")]
        run_id: AgentRunId,
    },
    /// Something outside Gritty (another program touching the workspace).
    External,
}

impl Actor {
    pub fn agent(run_id: AgentRunId) -> Self {
        Self::Agent { run_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_tagged_union() {
        assert_eq!(
            serde_json::to_value(Actor::User).unwrap(),
            serde_json::json!({ "type": "USER" })
        );
        let run = AgentRunId::from_raw("run-18");
        assert_eq!(
            serde_json::to_value(Actor::agent(run)).unwrap(),
            serde_json::json!({ "type": "AGENT", "runId": "run-18" })
        );
        let back: Actor =
            serde_json::from_value(serde_json::json!({ "type": "EXTERNAL" })).unwrap();
        assert_eq!(back, Actor::External);
    }
}
