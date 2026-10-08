//! The policy engine: a pure function from a request, the receiver's state, and
//! the recent volume changes to a decision.
//!
//! It depends on `domain` alone. It reads no clock, performs no I/O, and holds
//! no state: the caller supplies the time and the ledger of recent changes. The
//! rules are in [`config`], the decision in [`evaluate`].

pub mod config;
pub mod evaluate;
pub mod intent;
pub mod level;

pub use config::{
    Alternative, Budget, Conditions, Effect, Filter, PolicyConfig, PolicyError, Rule, Scope,
};
pub use evaluate::{evaluate, Baseline, Decision, PolicyInput, Reason, RecentChange};
pub use intent::{IntentKind, IntentValue};
pub use level::{Level, LevelError, Span};
