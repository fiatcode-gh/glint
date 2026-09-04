use std::fmt;

/// Where the session is. Every transition emits a D-Bus signal, so these names
/// are user-visible: the `Display` text is part of the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Idle,
    Scanning,
    Connecting,
    Pairing,
    Negotiating,
    Streaming,
    Reconnecting,
}

impl State {
    pub const ALL: [State; 7] = [
        State::Idle,
        State::Scanning,
        State::Connecting,
        State::Pairing,
        State::Negotiating,
        State::Streaming,
        State::Reconnecting,
    ];
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            State::Idle => "Idle",
            State::Scanning => "Scanning",
            State::Connecting => "Connecting",
            State::Pairing => "Pairing",
            State::Negotiating => "Negotiating",
            State::Streaming => "Streaming",
            State::Reconnecting => "Reconnecting",
        };
        f.write_str(name)
    }
}

/// What happened. Events come from NetworkManager, the RTSP channel, the
/// pipeline, and D-Bus method calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    ScanRequested,
    ScanFinished,
    ConnectRequested,
    LinkUp,
    PinRequired,
    PinEntered,
    LinkFailed,
    NegotiationDone,
    StreamStarted,
    LinkLost,
    RetryTimeout,
    DisconnectRequested,
}

impl Event {
    pub const ALL: [Event; 12] = [
        Event::ScanRequested,
        Event::ScanFinished,
        Event::ConnectRequested,
        Event::LinkUp,
        Event::PinRequired,
        Event::PinEntered,
        Event::LinkFailed,
        Event::NegotiationDone,
        Event::StreamStarted,
        Event::LinkLost,
        Event::RetryTimeout,
        Event::DisconnectRequested,
    ];
}

/// What the caller must do as a result. Only `State`'s `Display` text is
/// pinned by tests — `Action`'s `Debug` formatting is deliberately not part of
/// the contract (mutation control C2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    StartScan,
    StartLink,
    AskPin,
    StartRtsp,
    StartPipeline,
    StopPipeline,
    TearDownLink,
    ScheduleRetry,
    EmitSignal(State),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_displays_as_its_variant_name() {
        // arrange
        let cases = [
            (State::Idle, "Idle"),
            (State::Scanning, "Scanning"),
            (State::Connecting, "Connecting"),
            (State::Pairing, "Pairing"),
            (State::Negotiating, "Negotiating"),
            (State::Streaming, "Streaming"),
            (State::Reconnecting, "Reconnecting"),
        ];
        for (state, expected) in cases {
            // act & assert
            assert_eq!(state.to_string(), expected);
        }
    }

    #[test]
    fn all_seven_states_are_covered_by_the_display_test() {
        // A guard against a new State variant slipping past the table above.
        // act & assert
        assert_eq!(State::ALL.len(), 7);
    }
}
