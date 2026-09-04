//! The session state machine as one pure function.
//!
//! No clock, no I/O, no interior mutability: `step` maps a `(State, Event)`
//! pair to the next state plus an ordered action list. Action order is part of
//! the contract, and `EmitSignal(new_state)` is always last.

use std::fmt;

use crate::session::state::{Action, Event, State};

/// The pair had no transition. Every `(State, Event)` outside the table lands
/// here rather than silently self-transitioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    pub state: State,
    pub event: Event,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no transition from {} on {:?}", self.state, self.event)
    }
}

impl std::error::Error for InvalidTransition {}

/// Appends the signal every valid transition ends with.
///
/// Private, and named for what it does rather than where it is called from.
/// The tests pin `step`'s observable behaviour, never this helper's name, so
/// renaming it changes nothing a caller can see.
fn with_signal(next: State, mut actions: Vec<Action>) -> (State, Vec<Action>) {
    actions.push(Action::EmitSignal(next));
    (next, actions)
}

/// Advance the session.
///
/// `NegotiationDone` self-transitions to `Negotiating` rather than entering
/// `Streaming`: in Wi-Fi Display the pipeline is started once the parameters
/// are agreed, but the stream only truly runs when the sink sends RTSP PLAY,
/// which arrives later as its own `StreamStarted` event. Calling the state
/// `Streaming` in between would be a lie the D-Bus signal would repeat.
///
/// `Pairing + PinEntered` re-issues `StartLink` rather than a distinct
/// "resume" verb. With the NetworkManager P2P path the two are the same call —
/// activation stalls waiting on a secret, and supplying it re-drives the same
/// activation. If Milestone 3 finds that resuming a half-established link
/// genuinely differs from starting one, that is where a `ResumeLink` action
/// would be added; it is not invented here for a caller that does not exist.
pub fn step(state: State, event: Event) -> Result<(State, Vec<Action>), InvalidTransition> {
    use Action::*;
    use Event::*;
    use State::*;

    let (next, actions) = match (state, event) {
        (Idle, ScanRequested) => (Scanning, vec![StartScan]),
        (Scanning, ScanFinished) => (Idle, vec![]),
        (Idle, ConnectRequested) | (Scanning, ConnectRequested) => (Connecting, vec![StartLink]),
        (Connecting, LinkUp) | (Reconnecting, LinkUp) => (Negotiating, vec![StartRtsp]),
        (Connecting, PinRequired) => (Pairing, vec![AskPin]),
        (Pairing, PinEntered) => (Connecting, vec![StartLink]),
        (Connecting, LinkFailed) | (Pairing, LinkFailed) => (Idle, vec![TearDownLink]),
        (Negotiating, NegotiationDone) => (Negotiating, vec![StartPipeline]),
        (Negotiating, StreamStarted) => (Streaming, vec![]),
        (Negotiating, LinkLost) | (Streaming, LinkLost) => {
            (Reconnecting, vec![StopPipeline, ScheduleRetry])
        }
        (Reconnecting, LinkFailed) => (Reconnecting, vec![ScheduleRetry]),
        (Reconnecting, RetryTimeout) => (Idle, vec![TearDownLink]),
        (Connecting, DisconnectRequested)
        | (Pairing, DisconnectRequested)
        | (Reconnecting, DisconnectRequested) => (Idle, vec![TearDownLink]),
        (Negotiating, DisconnectRequested) | (Streaming, DisconnectRequested) => {
            (Idle, vec![StopPipeline, TearDownLink])
        }
        _ => return Err(InvalidTransition { state, event }),
    };

    Ok(with_signal(next, actions))
}
