//! Reconnect backoff policy. Pure arithmetic: no clock, no sleeping.

/// The backoff never grows past this, however many attempts have failed.
const BACKOFF_CAP_SECS: u32 = 8;

/// The attempt index at which the doubling reaches the cap (1, 2, 4, then 8).
const CAP_ATTEMPT: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Wait this many seconds, then try again.
    Delay(u32),
    /// The retry budget is spent — stop and tear the link down.
    GiveUp,
}

/// The delay this attempt would use, ignoring the budget.
fn delay_for(attempt: u32) -> u32 {
    if attempt >= CAP_ATTEMPT {
        BACKOFF_CAP_SECS
    } else {
        1 << attempt
    }
}

/// Total seconds spent on attempts `0..=attempt`.
///
/// The doubling part sums in closed form (1 + 2 + ... + 2^(n-1) = 2^n - 1);
/// every attempt from `CAP_ATTEMPT` onward costs `BACKOFF_CAP_SECS`. Saturating
/// arithmetic keeps an absurd attempt number from wrapping back into a small sum.
fn cumulative_through(attempt: u32) -> u32 {
    let doubling_total = (1u32 << CAP_ATTEMPT) - 1;
    if attempt < CAP_ATTEMPT {
        (1u32 << (attempt + 1)) - 1
    } else {
        let capped_attempts = attempt - CAP_ATTEMPT + 1;
        doubling_total.saturating_add(capped_attempts.saturating_mul(BACKOFF_CAP_SECS))
    }
}

/// Decide the next reconnect delay.
///
/// `attempt` is 0-based: it counts the retries already scheduled, so
/// `next_delay(0, _)` yields the first retry's delay.
///
/// Boundary semantics: give up when the cumulative sum of delays **including
/// the candidate delay** would exceed `retry_timeout_secs`. "Exceed" is
/// strict, so a sum landing exactly on the timeout is allowed — with a timeout
/// of 15 the delays 1, 2, 4, 8 sum to exactly 15 and all four are scheduled.
pub fn next_delay(attempt: u32, retry_timeout_secs: u32) -> Retry {
    if cumulative_through(attempt) > retry_timeout_secs {
        Retry::GiveUp
    } else {
        Retry::Delay(delay_for(attempt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_schedule_is_one_two_four_then_capped_at_eight() {
        // arrange: a timeout large enough that nothing gives up
        let timeout = 10_000;
        // act
        let delays: Vec<Retry> = (0..6).map(|a| next_delay(a, timeout)).collect();
        // assert
        assert_eq!(
            delays,
            vec![
                Retry::Delay(1),
                Retry::Delay(2),
                Retry::Delay(4),
                Retry::Delay(8),
                Retry::Delay(8),
                Retry::Delay(8),
            ]
        );
    }

    #[test]
    fn timeout_fifteen_allows_exactly_one_two_four_eight_then_gives_up() {
        // This is the boundary the spec pins: the cumulative sum INCLUDING the
        // candidate delay is 1+2+4+8 = 15, which does not exceed 15, so the
        // fourth retry is allowed. The fifth would reach 23 and gives up.
        // act & assert
        assert_eq!(next_delay(0, 15), Retry::Delay(1));
        assert_eq!(next_delay(1, 15), Retry::Delay(2));
        assert_eq!(next_delay(2, 15), Retry::Delay(4));
        assert_eq!(next_delay(3, 15), Retry::Delay(8));
        assert_eq!(next_delay(4, 15), Retry::GiveUp);
    }

    #[test]
    fn one_second_short_of_the_boundary_gives_up_one_retry_earlier() {
        // At timeout 14 the fourth candidate reaches 15 > 14, so it is refused.
        // act & assert
        assert_eq!(next_delay(2, 14), Retry::Delay(4));
        assert_eq!(next_delay(3, 14), Retry::GiveUp);
    }

    #[test]
    fn a_zero_timeout_gives_up_immediately() {
        // act & assert
        assert_eq!(next_delay(0, 0), Retry::GiveUp);
    }

    #[test]
    fn the_default_thirty_second_timeout_allows_four_retries() {
        // 1+2+4+8 = 15 <= 30; the fifth reaches 23 <= 30; the sixth reaches 31.
        // act & assert
        assert_eq!(next_delay(4, 30), Retry::Delay(8));
        assert_eq!(next_delay(5, 30), Retry::GiveUp);
    }

    #[test]
    fn give_up_is_stable_for_every_later_attempt() {
        // Once the budget is spent it stays spent — no attempt number wraps
        // back into a delay.
        for attempt in 5..40 {
            // act & assert
            assert_eq!(next_delay(attempt, 15), Retry::GiveUp, "attempt {attempt}");
        }
    }

    #[test]
    fn a_very_large_attempt_number_does_not_overflow() {
        // act & assert
        assert_eq!(next_delay(u32::MAX, 10_000), Retry::GiveUp);
    }
}
