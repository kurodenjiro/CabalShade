//! The reputation score — derived from real demonstrated behaviour.
//!
//! # From mock to signal (replaces ticket 03's mock, closes ticket 39)
//!
//! The prototype shipped a constant and later a hash of the node identifier
//! (ticket 03) so the screens read as designed before any signal existed. That
//! was honest about being a placeholder but the number it showed was not a
//! claim the system could back: a peer with zero relays scored the same as a
//! peer with a hundred.
//!
//! Reputation now starts from **measured behaviour**, all of which already
//! existed in the app:
//!
//! - transactions this node relayed for peers (relay history, persisted)
//! - bytes relayed through the mesh topic (the real counter)
//! - intents settled on-chain (the intent ledger's terminal states)
//! - the best observed peer latency (from the ping behaviour)
//!
//! # Why the score still does not move between polls
//!
//! Home polls every five seconds. A score that changed because the counters
//! changed a little between polls would read as a bug — the brand's copy rules
//! treat exact figures as a trust signal. The score is therefore derived from
//! the *cumulative* counters, and the delta is measured against a baseline the
//! first reading persists for this node. The value is stable within a session,
//! moves only when behaviour actually changes, and never fabricates a
//! period-over-period number the app has not observed.

use serde::{Deserialize, Serialize};

/// A reputation reading: the score and its change from the node's baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reputation {
    /// `60.0` to `99.9`.
    pub score: f64,
    /// `-9.9` to `+9.9`, as a percentage.
    pub delta_percent: f64,
}

/// What one node has actually done, so reputation can be a claim about
/// behaviour rather than an identifier hash.
#[derive(Debug, Clone, Copy, Default)]
pub struct Signals {
    /// Transactions relayed to the chain for other peers.
    pub relayed_tx_count: u64,
    /// Bytes relayed through the mesh topic.
    pub relay_bytes: u64,
    /// Intents that reached a terminal settled state.
    pub settled_deals: u64,
    /// Best observed peer round-trip time. `None` before the first ping.
    pub best_peer_latency_ms: Option<u16>,
}

impl Reputation {
    /// The score for a node's demonstrated behaviour, or `None` when there is
    /// no mesh to measure.
    ///
    /// An empty or placeholder identifier means the mesh is not up yet, which
    /// is a genuinely unmeasured state rather than one to fill in.
    #[must_use]
    pub fn of(peer_id: &str, signals: Signals) -> Option<Self> {
        if peer_id.is_empty() || peer_id == "—" {
            return None;
        }

        // Base + demonstrated behaviour, each contribution capped so no single
        // activity can dominate the band.
        let mut score = 60.0
            + (signals.relayed_tx_count.saturating_mul(6)).min(24) as f64
            + (signals.settled_deals.saturating_mul(4)).min(12) as f64
            + (signals.relay_bytes.saturating_div(1024)).min(6) as f64;

        // Latency is a penalty, not a reward: a slow link reduces the score but
        // an absent reading is unmeasured, not perfect.
        if let Some(ms) = signals.best_peer_latency_ms {
            match ms {
                0..=100 => {}
                101..=300 => score -= 2.0,
                301..=500 => score -= 4.0,
                _ => score -= 6.0,
            }
        }

        let score = score.clamp(60.0, 99.9);
        Some(Self { score, delta_percent: 0.0 })
    }

    /// `87.6` — the score alone, for the home tile that carries its delta in a
    /// separate field.
    #[must_use]
    pub fn value(&self) -> String {
        format!("{:.1}", self.score)
    }

    /// `87.6 (+5.3%)` — score and delta in one string, the shape the profile
    /// row renders.
    #[must_use]
    pub fn combined(&self) -> String {
        format!("{:.1} ({:+.1}%)", self.score, self.delta_percent)
    }
}

/// The persisted baseline a delta is measured against.
///
/// Written once per node identity when the score is first read, so the delta
/// stays stable across five-second polls and only moves when behaviour does.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReputationBaseline {
    /// The node this baseline belongs to.
    pub node_id: String,
    /// The score first observed for that node.
    pub score: f64,
}

impl ReputationBaseline {
    /// Reads the baseline for `node_id`, or writes a fresh one when absent.
    #[must_use]
    pub fn load_or_establish(
        node_id: &str,
        current_score: f64,
        store: &cabal_store::JsonStore,
    ) -> Option<Self> {
        let stored: Option<ReputationBaseline> = store.load().ok();
        match stored {
            // Same node, keep the original baseline so the delta is against
            // first sight, not the last poll.
            Some(baseline) if baseline.node_id == node_id => Some(baseline),
            // Different node (identity changed): re-anchor at the current score.
            Some(_) => {
                let baseline = Self { node_id: node_id.into(), score: current_score };
                let _ = store.save(&baseline);
                Some(baseline)
            }
            // No baseline yet: establish one at the current score, delta 0.
            None => {
                let baseline = Self { node_id: node_id.into(), score: current_score };
                let _ = store.save(&baseline);
                Some(baseline)
            }
        }
    }

    /// The delta of `current_score` against this baseline, clamped to the
    /// display band.
    #[must_use]
    pub fn delta_percent(&self, current_score: f64) -> f64 {
        let delta = ((current_score - self.score) / self.score) * 100.0;
        delta.clamp(-9.9, 9.9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals() -> Signals {
        Signals::default()
    }

    #[test]
    fn no_node_means_no_score() {
        // Not a fallback constant: every device without a mesh would otherwise
        // show the same number in every screenshot.
        assert!(Reputation::of("", signals()).is_none());
        assert!(Reputation::of("—", signals()).is_none());
    }

    #[test]
    fn zero_activity_scores_at_the_floor() {
        let reading = Reputation::of("12D3KooWIdle", signals()).unwrap();
        assert_eq!(reading.score, 60.0);
    }

    #[test]
    fn demonstrated_activity_raises_the_score() {
        let idle = Reputation::of("12D3KooWAlpha", signals()).unwrap().score;
        let active = Reputation::of(
            "12D3KooWAlpha",
            Signals {
                relayed_tx_count: 4,
                relay_bytes: 10 * 1024,
                settled_deals: 3,
                best_peer_latency_ms: None,
            },
        )
        .unwrap()
        .score;
        assert!(active > idle, "activity must improve the score: {idle} -> {active}");
    }

    #[test]
    fn contributions_are_capped_so_no_single_activity_dominates() {
        // 1_000 relays would be +6000 without the cap; the band keeps it sane.
        let reading = Reputation::of(
            "12D3KooWWhale",
            Signals {
                relayed_tx_count: 1_000,
                relay_bytes: 0,
                settled_deals: 0,
                best_peer_latency_ms: None,
            },
        )
        .unwrap();
        assert!((60.0..=99.9).contains(&reading.score));
        // 60 + capped 24 = 84.
        assert_eq!(reading.score, 84.0);
    }

    #[test]
    fn slow_peers_are_penalised_not_rewarded() {
        let fast = Reputation::of(
            "12D3KooWFast",
            Signals { relayed_tx_count: 2, ..Signals::default() },
        )
        .unwrap()
        .score;
        let slow = Reputation::of(
            "12D3KooWSlow",
            Signals {
                relayed_tx_count: 2,
                best_peer_latency_ms: Some(600),
                ..Signals::default()
            },
        )
        .unwrap()
        .score;
        assert!(fast > slow);
    }

    #[test]
    fn readings_stay_inside_their_bands() {
        for index in 0..2_000_u64 {
            let reading = Reputation::of(
                &format!("12D3KooW{index}"),
                Signals {
                    relayed_tx_count: index % 10,
                    relay_bytes: (index % 5) * 1024,
                    settled_deals: index % 3,
                    best_peer_latency_ms: Some((index % 700) as u16),
                },
            )
            .unwrap();
            assert!(
                (60.0..=99.9).contains(&reading.score),
                "score out of band: {}",
                reading.score
            );
            assert!(
                (-9.9..=9.9).contains(&reading.delta_percent),
                "delta out of band: {}",
                reading.delta_percent
            );
        }
    }

    #[test]
    fn formatting_matches_the_two_places_it_renders() {
        let reading = Reputation { score: 87.6, delta_percent: 5.3 };
        assert_eq!(reading.value(), "87.6");
        assert_eq!(reading.combined(), "87.6 (+5.3%)");

        // The sign is always explicit — a bare "3.0%" leaves the direction to
        // the colour, which reaches no screen reader.
        let falling = Reputation { score: 64.0, delta_percent: -3.0 };
        assert_eq!(falling.combined(), "64.0 (-3.0%)");
    }

    #[test]
    fn baseline_keeps_the_delta_stable_against_first_sight() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = cabal_store::JsonStore::new(dir.path().join("reputation.json"));

        let baseline = ReputationBaseline::load_or_establish("node-a", 80.0, &store).unwrap();
        assert_eq!(baseline.score, 80.0);

        // Later polls at a different score measure against the original 80.
        let later = ReputationBaseline::load_or_establish("node-a", 84.0, &store).unwrap();
        assert_eq!(later.score, 80.0);
        assert_eq!(later.delta_percent(84.0), 5.0);
    }

    #[test]
    fn a_new_identity_reanchors_the_baseline() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = cabal_store::JsonStore::new(dir.path().join("reputation.json"));

        ReputationBaseline::load_or_establish("node-a", 80.0, &store).unwrap();
        // Identity changed: re-anchor at the current score rather than inherit
        // another node's baseline.
        let rebased = ReputationBaseline::load_or_establish("node-b", 90.0, &store).unwrap();
        assert_eq!(rebased.score, 90.0);
    }
}
