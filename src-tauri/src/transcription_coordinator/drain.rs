use super::{Command, CoordinatorState, Stage};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

/// Keep the times and order of physical edges while the recorder returns its
/// sample buffer. Even a complete second hold must reach the recording path.
#[derive(Default)]
pub(super) struct DrainInputs(VecDeque<(Command, Instant)>);

impl DrainInputs {
    pub(super) fn receive(
        &mut self,
        rx: &Receiver<Command>,
        state: &CoordinatorState,
    ) -> Option<(Command, Instant)> {
        if let (Some(deadline), Some((_, at))) = (state.grace_deadline(), self.0.front()) {
            if deadline <= *at {
                return Some((Command::GraceExpired, deadline));
            }
        }
        if let Some(edge) = self.next(&state.stage) {
            return Some(edge);
        }
        let cmd = match state.grace_deadline() {
            Some(deadline) => {
                match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(cmd) => cmd,
                    Err(RecvTimeoutError::Timeout) => Command::GraceExpired,
                    Err(RecvTimeoutError::Disconnected) => return None,
                }
            }
            None => rx.recv().ok()?,
        };
        let at = match &cmd {
            Command::Input(_, at, ..) => *at,
            _ => Instant::now(),
        };
        if let Some(deadline) = state.grace_deadline() {
            if matches!(cmd, Command::Input(..)) && deadline <= at {
                self.0.push_front((cmd, at));
                return Some((Command::GraceExpired, deadline));
            }
        }
        Some((cmd, at))
    }

    pub(super) fn press_waiting(&self) -> bool {
        self.0
            .iter()
            .any(|(cmd, _)| matches!(cmd, Command::Input(input, ..) if input.is_pressed))
    }

    pub(super) fn next(&mut self, stage: &Stage) -> Option<(Command, Instant)> {
        if matches!(stage, Stage::Processing) {
            None
        } else {
            self.0.pop_front()
        }
    }

    pub(super) fn accept(
        &mut self,
        cmd: Command,
        at: Instant,
        stage: &Stage,
        interrupt: impl FnOnce(),
    ) -> Option<Command> {
        match &cmd {
            Command::Input(input, ..) if matches!(stage, Stage::Processing) => {
                if input.is_pressed {
                    interrupt();
                }
                self.0.push_back((cmd, at));
                None
            }
            Command::Cancel => {
                self.0.clear();
                Some(cmd)
            }
            _ => Some(cmd),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ShortcutActivation;
    use crate::transcription_coordinator::InputEvent;
    use std::time::Duration;

    #[test]
    fn a_complete_hold_during_drain_replays_both_physical_edges() {
        let mut drain = DrainInputs::default();
        let press = Instant::now();
        let release = press + Duration::from_millis(650);
        let mut cuts = 0;
        for (is_pressed, at) in [(true, press), (false, release)] {
            let edge = Command::Input(
                InputEvent {
                    binding_id: "transcribe".into(),
                    hotkey_string: "test".into(),
                    is_pressed,
                    mode: ShortcutActivation::PushToTalk,
                    hold_threshold: Duration::ZERO,
                    external: false,
                },
                at,
                crate::input::PasteDestination::focused(),
                None,
            );
            assert!(drain
                .accept(edge, at, &Stage::Processing, || cuts += 1)
                .is_none());
        }
        assert_eq!(cuts, 1);
        for (expected, time) in [(true, press), (false, release)] {
            let Some((Command::Input(edge, ..), at)) = drain.next(&Stage::Idle) else {
                panic!("lost edge")
            };
            assert_eq!((edge.is_pressed, at), (expected, time));
        }
    }
}
