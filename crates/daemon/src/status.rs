//! Pure status transitions. See spec section 3.4. This milestone implements the
//! "all" rows and the Shell rows; every runtime uses the Shell rows as its fallback.

use proto::{Runtime, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    /// Bytes arrived from the PTY.
    Output,
    /// No output for `manager::QUIET_AFTER`.
    Quiet,
    /// The program rang the terminal bell.
    Bell,
    /// A client started viewing this window.
    Focused,
    /// A client typed into this window.
    InputSent,
    /// The child process ended.
    Exited,
}

pub fn next(current: Status, event: StatusEvent, _runtime: Runtime) -> Status {
    use Status::*;
    use StatusEvent as E;
    match (current, event) {
        (Exited, _) | (_, E::Exited) => Exited,
        (_, E::Bell) => Attention,
        (Done, E::Focused) => Idle,
        (Attention, E::InputSent) => Working,
        (Starting | Idle | Done, E::Output) => Working,
        (Working, E::Quiet) => Idle,
        (s, _) => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Runtime::{Claude, Shell};
    use proto::Status::*;

    #[test]
    fn output_makes_a_window_working_and_quiet_makes_it_idle() {
        assert_eq!(next(Starting, StatusEvent::Output, Shell), Working);
        assert_eq!(next(Idle, StatusEvent::Output, Shell), Working);
        assert_eq!(next(Done, StatusEvent::Output, Shell), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Shell), Idle);
        assert_eq!(next(Idle, StatusEvent::Quiet, Shell), Idle);
    }

    #[test]
    fn bell_demands_attention_until_input_is_sent() {
        assert_eq!(next(Working, StatusEvent::Bell, Shell), Attention);
        assert_eq!(next(Attention, StatusEvent::Output, Shell), Attention);
        assert_eq!(next(Attention, StatusEvent::InputSent, Shell), Working);
        assert_eq!(next(Idle, StatusEvent::InputSent, Shell), Idle);
    }

    #[test]
    fn focusing_clears_done() {
        assert_eq!(next(Done, StatusEvent::Focused, Shell), Idle);
        assert_eq!(next(Working, StatusEvent::Focused, Shell), Working);
    }

    #[test]
    fn exited_is_terminal() {
        assert_eq!(next(Working, StatusEvent::Exited, Shell), Exited);
        assert_eq!(next(Exited, StatusEvent::Output, Shell), Exited);
        assert_eq!(next(Exited, StatusEvent::Bell, Shell), Exited);
    }

    #[test]
    fn fallback_rules_apply_to_every_runtime_in_this_milestone() {
        assert_eq!(next(Starting, StatusEvent::Output, Claude), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Claude), Idle);
    }
}
