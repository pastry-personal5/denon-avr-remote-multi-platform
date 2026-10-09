//! Tests of the Agent path through the gate: evaluation, refusal, the caps, and
//! the policy and ledger it depends on. They run on a paused clock with the
//! fakes of `tests.rs`, a fake audit log, a fake policy source, and a clock the
//! test sets.

mod decisions;
mod dispatch;
mod fakes;
mod fixtures;
mod review;
mod tokens;

use super::tests::{
    advance, living_room, settle, Effect as SessionEffect, FakeConfig, FakeConnector,
    FakeDiscovery, OrderLog,
};
use super::*;
use crate::audit::{AuditEntry, AuditError, AuditLog, AuditPage, AuditQuery, Durability};
use crate::clock::Clock;
use crate::control::{AgentLabel, ApprovalHealth, AuditHealth, DryRunDecision, PolicyHealth};
use crate::policy_source::{LoadedPolicy, PolicyDigest, PolicyLoadError, PolicySource};
use crate::tokens::SharedTokenStore;
use crate::{AuditDecision, AuditEvent, AuditRecord};
use denon_avr_domain::{
    CoreField, Epoch, FrameSeq, MasterVolume, MonotonicMillis, MuteState, ObservationOrigin,
    OperationOutcome, ReceiverObservation, ReceiverState, RejectionCause, WallTime,
};
use denon_avr_policy::{
    Effect as Verdict, IntentKind, IntentValue, Level, PolicyConfig, Rule, Span,
};
use std::collections::BTreeMap;

use dispatch::{counted, dispatching_record};
use fakes::{event_name, AuditMode, FakeAudit, FakeClock, FakePolicy};
use fixtures::{
    ask, ask_as_operator, label, level, master, mute_on, observation, operate_calls, owners_rules,
    policy_of, set_volume, start, start_default, turn_dial, AgentHarness, Setup,
};
