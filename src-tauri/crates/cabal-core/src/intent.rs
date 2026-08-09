//! The intent model: what the user asks for, and where it is in its life.

use crate::ids::ProofHash;
use crate::money::{TokenAmount, UsdPrice};
use serde::{Deserialize, Serialize};

#[cfg(feature = "ts-rs")]
use ts_rs::TS;

/// What the user is trying to do — the `I WANT TO` segmented control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "UPPERCASE")]
pub enum Action {
    Buy,
    Sell,
}

impl Action {
    /// Every variant, in the order the segmented control shows them.
    /// The MVP supports buying and selling only. Swap/stake require separate
    /// token-program adapters and are intentionally not exposed by the UI.
    pub const ALL: [Self; 2] = [Self::Buy, Self::Sell];
}

/// Execution strategy — the `MODE` selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionMode {
    Shark,
    Ghost,
    Patient,
}

impl ExecutionMode {
    pub const ALL: [Self; 3] = [Self::Shark, Self::Ghost, Self::Patient];

    /// The label shown in the selector, e.g. `SHARK MODE`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Shark => "SHARK MODE",
            Self::Ghost => "GHOST MODE",
            Self::Patient => "PATIENT MODE",
        }
    }

    /// The copy shown beneath the selector. Lives here rather than in the
    /// frontend so the mode and its description cannot drift apart.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Shark => "Aggressive execution. Best price. Higher risk.",
            Self::Ghost => "Maximum privacy. Longer route. Slower fill.",
            Self::Patient => "Waits for the condition. No slippage tolerance.",
        }
    }
}

/// How much routing privacy to buy, at the cost of speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "UPPERCASE")]
pub enum PrivacyLevel {
    Low,
    Medium,
    High,
}

impl PrivacyLevel {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];
}

/// The `CONDITION` row.
///
/// `Any` carries no price, which is the point: the type makes a priceless
/// `Under` unconstructable, so no downstream code has to handle a
/// "price-under-nothing" case that should never exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    Under { price: UsdPrice },
    Above { price: UsdPrice },
    Any,
}

impl Condition {
    /// The price this condition tests against, if it tests one.
    #[must_use]
    pub const fn price(&self) -> Option<UsdPrice> {
        match self {
            Self::Under { price } | Self::Above { price } => Some(*price),
            Self::Any => None,
        }
    }

    /// Whether `candidate` satisfies the condition.
    ///
    /// Strict on both sides, which is what makes two *opposing orders* at the
    /// same number refuse to match: neither "under 95" nor "above 95" accepts
    /// 95, so there is no price both sides agreed to. See [`Self::ceiling`] for
    /// the question a buyer asks about a price it is being quoted, which is a
    /// different question with a different answer.
    #[must_use]
    pub fn is_satisfied_by(&self, candidate: UsdPrice) -> bool {
        match self {
            Self::Under { price } => candidate < *price,
            Self::Above { price } => candidate > *price,
            Self::Any => true,
        }
    }

    /// The most a buyer holding this condition will pay, inclusive.
    ///
    /// Deliberately *not* [`Self::is_satisfied_by`]. That method answers "did
    /// two orders agree on a number", where the boundary belongs to neither
    /// side. This one answers "will I pay what I am being asked", and a limit
    /// price fills at the limit — every exchange treats "buy up to 95" as
    /// including 95, and a user who types the asking price means to buy at it.
    ///
    /// `None` means no upper limit was stated: `Any` accepts any price by
    /// definition, and `Above` bounds the wrong end.
    #[must_use]
    pub const fn ceiling(&self) -> Option<UsdPrice> {
        match self {
            Self::Under { price } => Some(*price),
            Self::Above { .. } | Self::Any => None,
        }
    }
}

/// Why an intent failed. A variant rather than a message, so the UI can render
/// on-voice copy instead of echoing an internal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureReason {
    /// No route through the mesh satisfied the condition before expiry.
    NoRoute,
    /// A node on the chosen route failed liveness and was slashed.
    NodeFailure,
    /// The condition never became true.
    ConditionUnmet,
    /// Settlement was attempted and rejected on-chain.
    SettlementRejected,
    /// Balance no longer covers the intent.
    InsufficientBalance,
}

/// Where an intent is in its life.
///
/// Matched exhaustively everywhere — no catch-all arm — so adding a variant
/// surfaces every place that has to decide what it means, rather than silently
/// falling through to a default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS), ts(export, export_to = "../../src/types/bindings.ts"))]
#[serde(tag = "status", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IntentStatus {
    /// Composed but not yet broadcast. Nothing has left the device.
    Draft,
    /// Dispatched to the mesh.
    Broadcast { route_len: u8 },
    /// Nodes are bidding.
    Negotiating { bids: u8, best: Option<UsdPrice> },
    /// Accepted, searching for a path.
    FindingRoute,
    /// Waiting for the condition to become true.
    Waiting,
    /// Settled on-chain. Terminal.
    Settled {
        proof: ProofHash,
        filled_at: UsdPrice,
        elapsed_ms: u32,
    },
    /// Failed. Terminal.
    Failed { reason: FailureReason },
    /// Cancelled by the user. Terminal. Escrow released, nothing written.
    Cancelled,
}

impl IntentStatus {
    /// Whether this is an end state. Terminal states accept no successor.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Settled { .. } | Self::Failed { .. } | Self::Cancelled)
    }

    /// Whether the intent is live on the mesh and can still be cancelled.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Broadcast { .. } | Self::Negotiating { .. } | Self::FindingRoute | Self::Waiting
        )
    }

    /// Whether `next` is a legal successor of `self`.
    ///
    /// The rules, in one place so they cannot be reimplemented differently by
    /// each caller:
    ///
    /// - A terminal state accepts nothing. Once settled, always settled.
    /// - Nothing returns to `Draft`; broadcasting is irreversible.
    /// - A draft may only be broadcast or cancelled — it cannot jump straight
    ///   to settled, which would mean settling something never sent.
    /// - Any live state may fail or be cancelled.
    #[must_use]
    pub fn can_transition_to(&self, next: &Self) -> bool {
        use IntentStatus as S;

        if self.is_terminal() {
            return false;
        }
        if matches!(next, S::Draft) {
            return false;
        }
        if matches!(next, S::Failed { .. } | S::Cancelled) {
            return true;
        }

        match (self, next) {
            (S::Draft, S::Broadcast { .. }) => true,
            (S::Draft, _) => false,

            (S::Broadcast { .. }, S::Negotiating { .. } | S::FindingRoute | S::Waiting) => true,
            // Back to the open book: a negotiation whose counterparty cancels
            // has to leave the surviving order matchable again, and nothing has
            // been written on-chain at this point to undo.
            (S::Negotiating { .. }, S::Broadcast { .. }) => true,
            (S::Negotiating { .. }, S::Negotiating { .. } | S::FindingRoute | S::Waiting) => true,
            (S::FindingRoute, S::Negotiating { .. } | S::Waiting | S::Settled { .. }) => true,
            (S::Waiting, S::Negotiating { .. } | S::FindingRoute | S::Settled { .. }) => true,

            // Settlement has to be routed first — otherwise there is nothing
            // to settle through.
            (S::Broadcast { .. } | S::Negotiating { .. }, S::Settled { .. }) => false,

            _ => false,
        }
    }
}

/// A composed but unsent intent — what the `new` screen produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentDraft {
    pub action: Action,
    pub asset: Box<str>,
    pub condition: Condition,
    pub amount: TokenAmount,
    pub mode: ExecutionMode,
    pub privacy: PrivacyLevel,
}

/// What two matched sides have agreed, derived from their conditions alone.
///
/// Carried separately from the drafts because it is a property of the *pair*:
/// neither order names this price on its own, and both must be able to
/// recompute it identically without talking to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchTerms {
    /// The price both conditions accept. `None` when neither side named one —
    /// two `Any` orders match at whatever the market gives, with no number to
    /// show.
    pub price: Option<UsdPrice>,
}

impl IntentDraft {
    /// The closed band of prices this draft's condition accepts, in cents.
    ///
    /// The bounds are inclusive, which is why `Under`/`Above` step one cent
    /// inward: [`Condition::is_satisfied_by`] is strict, so `UNDER 95.00` does
    /// not accept 95.00 and the band must not claim it does.
    fn price_band(&self) -> (u64, u64) {
        match self.condition {
            Condition::Under { price } => (0, price.cents().saturating_sub(1)),
            Condition::Above { price } => (price.cents().saturating_add(1), u64::MAX),
            Condition::Any => (0, u64::MAX),
        }
    }

    /// Whether `other` is the opposite side of *this* trade, and on what terms.
    ///
    /// Deterministic and symmetric: both devices run this over the same two
    /// drafts and reach the same answer, which is what lets a match be agreed
    /// without a round trip. The rules are the whole agreement:
    ///
    /// - opposite actions — a buy fills against a sell, never another buy;
    /// - the same asset;
    /// - the same amount, because the escrow settles one whole leg (partial
    ///   fills would need a fill ledger this MVP does not have);
    /// - overlapping price bands, priced at the midpoint of the overlap — the
    ///   one number neither side can call the other's.
    #[must_use]
    pub fn match_with(&self, other: &Self) -> Option<MatchTerms> {
        if self.action == other.action || self.asset != other.asset || self.amount != other.amount {
            return None;
        }

        let (own_low, own_high) = self.price_band();
        let (their_low, their_high) = other.price_band();
        let low = own_low.max(their_low);
        let high = own_high.min(their_high);
        if low > high {
            return None;
        }

        let price = match (low, high) {
            // Neither side bounded the price.
            (0, u64::MAX) => None,
            // Only one side did: its own bound is the agreed price.
            (low, u64::MAX) => Some(UsdPrice::from_cents(low)),
            (0, high) => Some(UsdPrice::from_cents(high)),
            // Both did: split the overlap.
            (low, high) => Some(UsdPrice::from_cents(low + (high - low) / 2)),
        };
        Some(MatchTerms { price })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled() -> IntentStatus {
        IntentStatus::Settled {
            proof: ProofHash::new("0xa4f2c9e1b70d5533"),
            filled_at: UsdPrice::from_cents(9421),
            elapsed_ms: 11_400,
        }
    }

    #[test]
    fn terminal_states_accept_nothing() {
        for terminal in [
            settled(),
            IntentStatus::Failed { reason: FailureReason::NoRoute },
            IntentStatus::Cancelled,
        ] {
            assert!(!terminal.can_transition_to(&IntentStatus::Waiting));
            assert!(!terminal.can_transition_to(&IntentStatus::Cancelled));
            assert!(!terminal.can_transition_to(&settled()));
        }
    }

    #[test]
    fn a_collapsed_negotiation_returns_to_the_open_book() {
        let negotiating = IntentStatus::Negotiating { bids: 1, best: None };
        assert!(negotiating.can_transition_to(&IntentStatus::Broadcast { route_len: 2 }));
        // Only from negotiation. Routing has committed to a path, and settled
        // is settled.
        assert!(!IntentStatus::FindingRoute.can_transition_to(&IntentStatus::Broadcast { route_len: 2 }));
        assert!(!settled().can_transition_to(&IntentStatus::Broadcast { route_len: 2 }));
    }

    #[test]
    fn nothing_returns_to_draft() {
        assert!(!IntentStatus::Waiting.can_transition_to(&IntentStatus::Draft));
        assert!(!IntentStatus::Broadcast { route_len: 3 }.can_transition_to(&IntentStatus::Draft));
    }

    #[test]
    fn a_draft_cannot_settle_without_being_broadcast() {
        assert!(!IntentStatus::Draft.can_transition_to(&settled()));
    }

    #[test]
    fn settlement_requires_a_route() {
        // Settling straight from broadcast would mean settling through a route
        // that was never found.
        assert!(!IntentStatus::Broadcast { route_len: 3 }.can_transition_to(&settled()));
        assert!(IntentStatus::FindingRoute.can_transition_to(&settled()));
        assert!(IntentStatus::Waiting.can_transition_to(&settled()));
    }

    #[test]
    fn any_live_state_can_fail_or_be_cancelled() {
        for live in [
            IntentStatus::Draft,
            IntentStatus::Broadcast { route_len: 3 },
            IntentStatus::Negotiating { bids: 3, best: None },
            IntentStatus::FindingRoute,
            IntentStatus::Waiting,
        ] {
            assert!(live.can_transition_to(&IntentStatus::Cancelled), "{live:?}");
            assert!(
                live.can_transition_to(&IntentStatus::Failed { reason: FailureReason::NodeFailure }),
                "{live:?}"
            );
        }
    }

    #[test]
    fn active_and_terminal_are_mutually_exclusive() {
        for status in [
            IntentStatus::Draft,
            IntentStatus::Broadcast { route_len: 1 },
            IntentStatus::Negotiating { bids: 0, best: None },
            IntentStatus::FindingRoute,
            IntentStatus::Waiting,
            settled(),
            IntentStatus::Failed { reason: FailureReason::NoRoute },
            IntentStatus::Cancelled,
        ] {
            assert!(!(status.is_active() && status.is_terminal()), "{status:?}");
        }
    }

    #[test]
    fn conditions_test_the_price_they_carry() {
        let ninety_five = UsdPrice::from_cents(9500);
        let filled = UsdPrice::from_cents(9421);

        assert!(Condition::Under { price: ninety_five }.is_satisfied_by(filled));
        assert!(!Condition::Above { price: ninety_five }.is_satisfied_by(filled));
        assert!(Condition::Any.is_satisfied_by(filled));
    }

    #[test]
    fn any_condition_carries_no_price() {
        assert_eq!(Condition::Any.price(), None);
    }

    #[test]
    fn a_limit_buys_at_its_own_limit() {
        // The exact case that broke the Boost flow: the compose form defaults
        // to a 0.0001 SOL ceiling and the marketplace lists at exactly that,
        // so a strict reading refused the app's own default trade.
        let limit = UsdPrice::from_cents(100_000);
        let condition = Condition::Under { price: limit };

        assert_eq!(condition.ceiling(), Some(limit));
        // Matching still treats the boundary as agreed by neither side.
        assert!(!condition.is_satisfied_by(limit));
    }

    #[test]
    fn an_unbounded_condition_states_no_ceiling() {
        assert_eq!(Condition::Any.ceiling(), None);
        // A floor is not a ceiling: `Above` bounds the other end entirely.
        assert_eq!(
            Condition::Above { price: UsdPrice::from_cents(9400) }.ceiling(),
            None
        );
    }

    #[test]
    fn status_serializes_with_the_tag_the_ui_switches_on() {
        let json = serde_json::to_string(&IntentStatus::FindingRoute).unwrap();
        assert_eq!(json, r#"{"status":"FINDING_ROUTE"}"#);
    }

    fn draft(action: Action, condition: Condition, amount: &str) -> IntentDraft {
        IntentDraft {
            action,
            asset: "SOL".into(),
            condition,
            amount: TokenAmount::parse(amount, 9).unwrap(),
            mode: ExecutionMode::Shark,
            privacy: PrivacyLevel::Medium,
        }
    }

    #[test]
    fn a_buy_never_fills_against_another_buy() {
        let one = draft(Action::Buy, Condition::Any, "1");
        let two = draft(Action::Buy, Condition::Any, "1");
        assert_eq!(one.match_with(&two), None);
    }

    #[test]
    fn sides_must_want_the_same_asset_and_amount() {
        let buy = draft(Action::Buy, Condition::Any, "1");
        let mut other_asset = draft(Action::Sell, Condition::Any, "1");
        other_asset.asset = "BOOST NFT".into();
        assert_eq!(buy.match_with(&other_asset), None);
        assert_eq!(buy.match_with(&draft(Action::Sell, Condition::Any, "2")), None);
    }

    #[test]
    fn a_buy_ceiling_below_a_sell_floor_does_not_match() {
        let buy = draft(Action::Buy, Condition::Under { price: UsdPrice::from_cents(9000) }, "1");
        let sell = draft(Action::Sell, Condition::Above { price: UsdPrice::from_cents(9500) }, "1");
        assert_eq!(buy.match_with(&sell), None);
        // Touching bounds are still no overlap: `UNDER 95` and `ABOVE 95` are
        // both strict, so 95.00 satisfies neither.
        let at = draft(Action::Sell, Condition::Above { price: UsdPrice::from_cents(9000) }, "1");
        assert_eq!(buy.match_with(&at), None);
    }

    #[test]
    fn an_overlap_prices_at_its_midpoint_and_both_conditions_accept_it() {
        let buy = draft(Action::Buy, Condition::Under { price: UsdPrice::from_cents(9600) }, "1");
        let sell = draft(Action::Sell, Condition::Above { price: UsdPrice::from_cents(9400) }, "1");
        let price = buy.match_with(&sell).unwrap().price.unwrap();
        assert_eq!(price, UsdPrice::from_cents(9500));
        assert!(buy.condition.is_satisfied_by(price));
        assert!(sell.condition.is_satisfied_by(price));
    }

    #[test]
    fn one_bounded_side_sets_the_price_alone() {
        let buy = draft(Action::Buy, Condition::Any, "1");
        let sell = draft(Action::Sell, Condition::Above { price: UsdPrice::from_cents(9400) }, "1");
        assert_eq!(
            buy.match_with(&sell).unwrap().price,
            Some(UsdPrice::from_cents(9401))
        );
    }

    #[test]
    fn two_unbounded_sides_match_with_no_agreed_number() {
        let buy = draft(Action::Buy, Condition::Any, "1");
        let sell = draft(Action::Sell, Condition::Any, "1");
        assert_eq!(buy.match_with(&sell).unwrap().price, None);
    }

    #[test]
    fn matching_is_symmetric_so_both_devices_agree() {
        let buy = draft(Action::Buy, Condition::Under { price: UsdPrice::from_cents(9600) }, "1");
        let sell = draft(Action::Sell, Condition::Above { price: UsdPrice::from_cents(9401) }, "1");
        assert_eq!(buy.match_with(&sell), sell.match_with(&buy));
    }

    #[test]
    fn mode_labels_match_the_prototype() {
        assert_eq!(ExecutionMode::Shark.label(), "SHARK MODE");
        assert_eq!(
            ExecutionMode::Ghost.description(),
            "Maximum privacy. Longer route. Slower fill."
        );
    }
}
