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
/// the contract, so renaming an `Action` variant breaks no promise the crate
/// makes to its callers.
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
    fn state_all_lists_every_variant_exactly_once_in_order() {
        // An exhaustive match: adding a State variant makes this fail to
        // COMPILE, which is the only thing that can force a new variant to be
        // noticed by everything that sweeps State::ALL.
        fn index_of(state: State) -> usize {
            match state {
                State::Idle => 0,
                State::Scanning => 1,
                State::Connecting => 2,
                State::Pairing => 3,
                State::Negotiating => 4,
                State::Streaming => 5,
                State::Reconnecting => 6,
            }
        }
        // The number of variants the match above enumerates.
        const VARIANTS: usize = 7;

        // act & assert
        assert_eq!(State::ALL.len(), VARIANTS);
        for (index, state) in State::ALL.iter().enumerate() {
            assert_eq!(
                index_of(*state),
                index,
                "State::ALL is missing or misorders {state}"
            );
            assert_eq!(State::ALL[index_of(*state)], *state);
        }
    }

    #[test]
    fn event_all_lists_every_variant_exactly_once_in_order() {
        // Same guard for Event: tests/state_machine.rs sweeps Event::ALL, so a
        // variant left out of ALL would never be swept and nothing would fail.
        fn index_of(event: Event) -> usize {
            match event {
                Event::ScanRequested => 0,
                Event::ScanFinished => 1,
                Event::ConnectRequested => 2,
                Event::LinkUp => 3,
                Event::PinRequired => 4,
                Event::PinEntered => 5,
                Event::LinkFailed => 6,
                Event::NegotiationDone => 7,
                Event::StreamStarted => 8,
                Event::LinkLost => 9,
                Event::RetryTimeout => 10,
                Event::DisconnectRequested => 11,
            }
        }
        // The number of variants the match above enumerates.
        const VARIANTS: usize = 12;

        // act & assert
        assert_eq!(Event::ALL.len(), VARIANTS);
        for (index, event) in Event::ALL.iter().enumerate() {
            assert_eq!(
                index_of(*event),
                index,
                "Event::ALL is missing or misorders {event:?}"
            );
            assert_eq!(Event::ALL[index_of(*event)], *event);
        }
    }
}
