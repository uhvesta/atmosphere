//! A turn's usage as Copilot's session metrics count it, for the calls it
//! didn't report one by one. Copilot reports each model call's usage as it
//! happens, but those reports aren't kept, so any that arrive after the turn
//! has ended, or not at all, are made up for from the totals.

use std::collections::HashMap;

use serde_json::Value;

use crate::event::{Event, EventKind};

/// Usage by the call that started the subagent that used it, None for the
/// main agent, and by model.
pub(super) type Ledger = HashMap<(Option<String>, String), Tally>;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Tally {
    /// Prompt cache reads and writes included.
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) cache_write_tokens: u64,
    pub(super) premium_requests: f64,
}

impl Tally {
    fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_write_tokens += other.cache_write_tokens;
        self.premium_requests += other.premium_requests;
    }

    /// What `self` has beyond `other`, none of it below 0.
    fn beyond(&self, other: &Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_sub(other.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(other.output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_sub(other.cache_read_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_sub(other.cache_write_tokens),
            premium_requests: (self.premium_requests - other.premium_requests).max(0.0),
        }
    }

    fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_read_tokens == 0
            && self.cache_write_tokens == 0
            && self.premium_requests < 0.0005
    }

    pub(super) fn kind(&self, model: String) -> EventKind {
        EventKind::Usage {
            model,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            premium_requests: self.premium_requests,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
        }
    }
}

/// Copilot's `session.usage.getMetrics` answer as a ledger; `call_of` gives
/// the call that started a subagent from its id. False with it if the answer
/// splits usage only by model, not by agent.
pub(super) fn metrics(answer: &Value, call_of: impl Fn(&str) -> String) -> (Ledger, bool) {
    let mut ledger = Ledger::new();
    let agents = answer["agentMetrics"]
        .as_object()
        .filter(|agents| !agents.is_empty());
    let by_agent = agents.is_some();
    let mut add = |parent: Option<String>, models: &Value| {
        for (model, metric) in models.as_object().into_iter().flatten() {
            let usage = &metric["usage"];
            let count = |field: &str| usage[field].as_u64().unwrap_or(0);
            ledger
                .entry((parent.clone(), model.clone()))
                .or_default()
                .add(&Tally {
                    input_tokens: count("inputTokens"),
                    output_tokens: count("outputTokens"),
                    cache_read_tokens: count("cacheReadTokens"),
                    cache_write_tokens: count("cacheWriteTokens"),
                    premium_requests: metric["requests"]["cost"].as_f64().unwrap_or(0.0),
                });
        }
    };
    match agents {
        Some(agents) => {
            for (agent, metrics) in agents {
                let parent = (agent != "main").then(|| call_of(agent));
                add(parent, &metrics["modelMetrics"]);
            }
        }
        None => add(None, &answer["modelMetrics"]),
    }
    (ledger, by_agent)
}

/// The usage recorded in `events` since the last turn started.
pub(super) fn recorded(events: &[Event]) -> Ledger {
    let start = events
        .iter()
        .rposition(|event| matches!(event.kind, EventKind::TurnStarted))
        .unwrap_or(0);
    let mut ledger = Ledger::new();
    for event in &events[start..] {
        if let EventKind::Usage {
            model,
            input_tokens,
            output_tokens,
            premium_requests,
            cache_read_tokens,
            cache_write_tokens,
        } = &event.kind
        {
            ledger
                .entry((event.parent_call_id.clone(), model.clone()))
                .or_default()
                .add(&Tally {
                    input_tokens: *input_tokens,
                    output_tokens: *output_tokens,
                    cache_read_tokens: *cache_read_tokens,
                    cache_write_tokens: *cache_write_tokens,
                    premium_requests: *premium_requests,
                });
        }
    }
    ledger
}

/// What the metrics counted between `before` and `after` that `recorded`
/// lacks, in a steady order. With metrics `by_agent` false, everything
/// recorded counts toward the main agent's, as the metrics do.
pub(super) fn missing(
    before: &Ledger,
    after: &Ledger,
    by_agent: bool,
    recorded: &Ledger,
) -> Vec<((Option<String>, String), Tally)> {
    let mut seen = Ledger::new();
    for ((parent, model), tally) in recorded {
        let parent = if by_agent { parent.clone() } else { None };
        seen.entry((parent, model.clone())).or_default().add(tally);
    }
    let mut missing: Vec<_> = after
        .iter()
        .filter_map(|(key, total)| {
            let used = total.beyond(&before.get(key).copied().unwrap_or_default());
            let left = used.beyond(&seen.get(key).copied().unwrap_or_default());
            (!left.is_empty()).then(|| (key.clone(), left))
        })
        .collect();
    missing.sort_by(|(a, _), (b, _)| a.cmp(b));
    missing
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key(parent: Option<&str>, model: &str) -> (Option<String>, String) {
        (parent.map(str::to_owned), model.to_owned())
    }

    fn tally(input_tokens: u64, output_tokens: u64, premium_requests: f64) -> Tally {
        Tally {
            input_tokens,
            output_tokens,
            premium_requests,
            ..Tally::default()
        }
    }

    fn model(input: u64, output: u64, cost: f64) -> Value {
        json!({"requests": {"count": 1, "cost": cost},
            "usage": {"inputTokens": input, "outputTokens": output, "cacheReadTokens": 0, "cacheWriteTokens": 0}})
    }

    #[test]
    fn usage_copilot_counted_but_didnt_report_is_made_up_for() {
        let call_of = |agent: &str| format!("call-of-{agent}");
        let (before, _) = metrics(
            &json!({"agentMetrics": {"main": {"modelMetrics": {"gpt-5": model(100, 10, 1.0)}}}}),
            call_of,
        );
        let (after, by_agent) = metrics(
            &json!({"agentMetrics": {
                "main": {"modelMetrics": {"gpt-5": model(400, 40, 2.0)}},
                "sub-1": {"modelMetrics": {"gpt-5-mini": model(50, 5, 0.0)}},
            }}),
            call_of,
        );
        let mut recorded = Ledger::new();
        recorded.insert(key(None, "gpt-5"), tally(200, 20, 1.0));

        assert!(by_agent);
        assert_eq!(
            missing(&before, &after, by_agent, &recorded),
            [
                (key(None, "gpt-5"), tally(100, 10, 0.0)),
                (key(Some("call-of-sub-1"), "gpt-5-mini"), tally(50, 5, 0.0)),
            ]
        );
    }

    #[test]
    fn nothing_is_made_up_when_every_call_was_reported() {
        let models = json!({"modelMetrics": {"gpt-5": model(300, 30, 1.0)}});
        let (after, by_agent) = metrics(&models, str::to_owned);
        let mut recorded = Ledger::new();
        recorded.insert(key(None, "gpt-5"), tally(250, 25, 1.0));
        recorded.insert(key(Some("call-1"), "gpt-5"), tally(50, 5, 0.0));

        assert!(!by_agent);
        assert!(missing(&Ledger::new(), &after, by_agent, &recorded).is_empty());
    }

    #[test]
    fn recorded_usage_is_this_turns() {
        let usage = |input_tokens| {
            Event::now(EventKind::Usage {
                model: "gpt-5".into(),
                input_tokens,
                output_tokens: 0,
                premium_requests: 0.0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            })
        };
        let mut sub = usage(7);
        sub.parent_call_id = Some("call-1".into());
        let events = [
            usage(1),
            Event::now(EventKind::TurnStarted),
            usage(2),
            usage(3),
            sub,
        ];
        let ledger = recorded(&events);
        assert_eq!(ledger[&key(None, "gpt-5")].input_tokens, 5);
        assert_eq!(ledger[&key(Some("call-1"), "gpt-5")].input_tokens, 7);
        assert_eq!(ledger.len(), 2);
    }
}
