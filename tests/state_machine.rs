//! The full transition table from the spec, pinned row by row. Anything not in
//! the table is `InvalidTransition`.

use glint::session::machine::{InvalidTransition, step};
use glint::session::state::{Action, Event, State};
use glint::session::state::{Action::*, Event::*, State::*};

/// (from, event, to, actions before the trailing EmitSignal)
const TABLE: &[(State, Event, State, &[Action])] = &[
    (Idle, ScanRequested, Scanning, &[StartScan]),
    (Scanning, ScanFinished, Idle, &[]),
    (Idle, ConnectRequested, Connecting, &[StartLink]),
    (Scanning, ConnectRequested, Connecting, &[StartLink]),
    (Connecting, LinkUp, Negotiating, &[StartRtsp]),
    (Connecting, PinRequired, Pairing, &[AskPin]),
    (Pairing, PinEntered, Connecting, &[StartLink]),
    (Connecting, LinkFailed, Idle, &[TearDownLink]),
    (Pairing, LinkFailed, Idle, &[TearDownLink]),
    (Negotiating, NegotiationDone, Negotiating, &[StartPipeline]),
    (Negotiating, StreamStarted, Streaming, &[]),
    (
        Negotiating,
        LinkLost,
        Reconnecting,
        &[StopPipeline, ScheduleRetry],
    ),
    (
        Streaming,
        LinkLost,
        Reconnecting,
        &[StopPipeline, ScheduleRetry],
    ),
    (Reconnecting, LinkUp, Negotiating, &[StartRtsp]),
    (Reconnecting, LinkFailed, Reconnecting, &[ScheduleRetry]),
    (Reconnecting, RetryTimeout, Idle, &[TearDownLink]),
    (Connecting, DisconnectRequested, Idle, &[TearDownLink]),
    (Pairing, DisconnectRequested, Idle, &[TearDownLink]),
    (
        Negotiating,
        DisconnectRequested,
        Idle,
        &[StopPipeline, TearDownLink],
    ),
    (
        Streaming,
        DisconnectRequested,
        Idle,
        &[StopPipeline, TearDownLink],
    ),
    (Reconnecting, DisconnectRequested, Idle, &[TearDownLink]),
];

#[test]
fn every_row_of_the_transition_table_holds() {
    for &(from, event, to, before_signal) in TABLE {
        // act
        let (next, actions) = step(from, event)
            .unwrap_or_else(|_| panic!("{from} + {event:?} should be a valid transition"));

        // assert
        assert_eq!(next, to, "wrong target state for {from} + {event:?}");

        let mut expected: Vec<Action> = before_signal.to_vec();
        expected.push(EmitSignal(to));
        assert_eq!(
            actions, expected,
            "wrong action list for {from} + {event:?}"
        );
    }
}

#[test]
fn the_table_covers_exactly_twenty_one_transitions() {
    // A new arm in step() that nobody added to TABLE would otherwise go unpinned.
    assert_eq!(TABLE.len(), 21);

    // The sweep proves step's arms are all in TABLE, and the row test proves
    // TABLE's rows are all in step. Set equality needs distinctness too: a
    // duplicated (from, event) pair would let a missing one hide in the count.
    for (index, &(from, event, _, _)) in TABLE.iter().enumerate() {
        assert!(
            !TABLE[..index]
                .iter()
                .any(|&(f, e, _, _)| f == from && e == event),
            "{from} + {event:?} appears twice in TABLE"
        );
    }
}

#[test]
fn emit_signal_is_always_the_last_action_and_names_the_new_state() {
    for &(from, event, to, _) in TABLE {
        // act
        let (_, actions) = step(from, event).unwrap();
        // assert
        assert_eq!(
            actions.last(),
            Some(&EmitSignal(to)),
            "{from} + {event:?} must end with EmitSignal({to})"
        );
    }
}

#[test]
fn transitions_outside_the_table_are_rejected() {
    // arrange
    let invalid = [
        (Idle, LinkUp),
        (Streaming, PinEntered),
        (Idle, RetryTimeout),
        (Scanning, StreamStarted),
        (Streaming, ScanRequested),
    ];
    for (from, event) in invalid {
        // act
        let result = step(from, event);
        // assert
        assert_eq!(
            result,
            Err(InvalidTransition { state: from, event }),
            "{from} + {event:?} should be rejected"
        );
    }
}

#[test]
fn every_state_event_pair_is_either_in_the_table_or_rejected() {
    // Exhaustive sweep: 7 states x 12 events. Nothing may panic, and anything
    // that succeeds must be a row of TABLE.
    for state in State::ALL {
        for event in Event::ALL {
            match step(state, event) {
                Ok((next, _)) => {
                    assert!(
                        TABLE
                            .iter()
                            .any(|&(f, e, t, _)| f == state && e == event && t == next),
                        "{state} + {event:?} succeeded but is not in TABLE"
                    );
                }
                Err(err) => assert_eq!(err, InvalidTransition { state, event }),
            }
        }
    }
}
