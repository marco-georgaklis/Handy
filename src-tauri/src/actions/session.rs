use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tokio::sync::oneshot;

use crate::tray::{set_tray_state, TrayIconState};
use crate::utils;

#[derive(Default)]
pub(super) struct DictationSession {
    cancelled: AtomicBool,
}

static CURRENT: Lazy<Mutex<Arc<DictationSession>>> = Lazy::new(Mutex::default);
static OUTPUT_TAIL: Mutex<Option<oneshot::Receiver<()>>> = Mutex::new(None);

impl DictationSession {
    pub(super) fn begin() {
        *CURRENT.lock().unwrap() = Arc::default();
    }

    pub(super) fn current() -> Arc<Self> {
        Arc::clone(&CURRENT.lock().unwrap())
    }

    pub(super) fn is_current(self: &Arc<Self>) -> bool {
        Arc::ptr_eq(self, &CURRENT.lock().unwrap())
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(super) fn hide_if_current(self: &Arc<Self>, app: &AppHandle) {
        if self.is_current() {
            utils::hide_recording_overlay(app);
            set_tray_state(app, TrayIconState::Idle);
        }
    }
}

pub(crate) fn cancel_current_dictation() {
    CURRENT
        .lock()
        .unwrap()
        .cancelled
        .store(true, Ordering::Release);
}

/// Dropping the sender on any exit releases the following output turn.
pub(super) fn reserve_output() -> (Option<oneshot::Receiver<()>>, oneshot::Sender<()>) {
    let (finished, wait) = oneshot::channel();
    (OUTPUT_TAIL.lock().unwrap().replace(wait), finished)
}

pub(super) async fn wait_for_output(previous: Option<oneshot::Receiver<()>>) {
    if let Some(previous) = previous {
        // Both sending and dropping the predecessor signal that its turn is over.
        let _ = previous.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_during_the_next_hold_keeps_the_preceding_output() {
        DictationSession::begin();
        let previous = DictationSession::current();
        DictationSession::begin();
        let recording = DictationSession::current();
        cancel_current_dictation();
        assert!(recording.is_cancelled());
        assert!(!previous.is_cancelled());
        assert!(!previous.is_current());
    }

    #[tokio::test]
    async fn outputs_wait_in_release_order_even_if_a_turn_ends_without_a_paste() {
        let (first, finish_first) = reserve_output();
        wait_for_output(first).await;
        let (second, finish_second) = reserve_output();
        let (third, _finish_third) = reserve_output();
        let mut second = Box::pin(wait_for_output(second));
        let mut third = Box::pin(wait_for_output(third));
        assert!(futures_util::poll!(&mut second).is_pending());
        drop(finish_first);
        second.await;
        assert!(futures_util::poll!(&mut third).is_pending());
        drop(finish_second);
        third.await;
    }
}
