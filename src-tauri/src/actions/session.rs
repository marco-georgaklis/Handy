use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tokio::sync::watch;

use crate::tray::{set_tray_state, TrayIconState};
use crate::utils;

#[derive(Default)]
pub(super) struct DictationSession {
    cancelled: AtomicBool,
    decoding: AtomicBool,
}

static CURRENT: Lazy<Mutex<Arc<DictationSession>>> = Lazy::new(Mutex::default);
static OUTPUT_TAIL: Mutex<Option<watch::Receiver<()>>> = Mutex::new(None);

impl DictationSession {
    pub(super) fn begin() {
        *CURRENT.lock().unwrap() = Arc::default();
    }

    pub(super) fn current() -> Arc<Self> {
        Arc::clone(&CURRENT.lock().unwrap())
    }

    #[cfg(test)]
    pub(super) fn is_current(self: &Arc<Self>) -> bool {
        Arc::ptr_eq(self, &CURRENT.lock().unwrap())
    }

    pub(super) fn when_current(self: &Arc<Self>, f: impl FnOnce()) {
        let owner = CURRENT.lock().unwrap();
        if Arc::ptr_eq(self, &owner) && !self.is_cancelled() {
            f();
        }
    }

    pub(super) fn notice(self: &Arc<Self>, app: &AppHandle, notice: crate::notices::Notice) {
        let owner = CURRENT.lock().unwrap();
        if self.is_cancelled() {
            return;
        }
        if Arc::ptr_eq(self, &owner) {
            crate::notices::raise(app, notice);
        } else {
            crate::notices::raise_previous_dictation(app, notice);
        }
    }

    pub(super) fn begin_decode(&self) -> bool {
        self.decoding.store(true, Ordering::SeqCst);
        !self.is_cancelled()
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(super) fn hide_if_current(self: &Arc<Self>, app: &AppHandle) {
        self.when_current(|| {
            utils::hide_recording_overlay(app);
            set_tray_state(app, TrayIconState::Idle);
        });
    }
}

pub(crate) fn output_pending() -> bool {
    OUTPUT_TAIL
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|last| last.has_changed().is_ok())
}

/// A younger take captured during an older decode has no engine work to cancel.
pub(crate) fn cancel_current_dictation() -> bool {
    let current = CURRENT.lock().unwrap();
    current.cancelled.store(true, Ordering::SeqCst);
    current.decoding.load(Ordering::SeqCst) || !output_pending()
}

/// Dropping the sender on any exit releases the following output turn.
pub(super) fn reserve_output() -> (Option<watch::Receiver<()>>, watch::Sender<()>) {
    let (finished, wait) = watch::channel(());
    (OUTPUT_TAIL.lock().unwrap().replace(wait), finished)
}

pub(super) async fn wait_for_output(previous: Option<watch::Receiver<()>>) {
    if let Some(mut previous) = previous {
        // The predecessor drops its only sender when it finishes or is cancelled.
        let _ = previous.changed().await;
    }
}

pub(super) struct PasteOutput {
    pub app: AppHandle,
    pub destination: crate::input::PasteDestination,
    pub session: Arc<DictationSession>,
    pub sample_count: usize,
    pub stop_time: std::time::Instant,
    pub transcribed_at: std::time::Duration,
    pub processed_at: std::time::Duration,
    pub post_process_failed: bool,
}

impl PasteOutput {
    pub(super) fn paste(self, text: String) {
        use super::TranscriptionOutcome;
        use crate::notices::{self, Notice, NoticeCode, NoticeSource};
        if self.session.is_cancelled() {
            return;
        }
        crate::overlay::set_last_output(&text);
        let started = std::time::Instant::now();
        let (outcome, notice) = match utils::paste_to(text, self.app.clone(), self.destination) {
            Ok(()) => {
                log::info!(
                    "dictation timing: audio={:.2}s transcribe={}ms post_process={}ms paste={}ms total={}ms",
                    self.sample_count as f64 / crate::audio_toolkit::constants::WHISPER_SAMPLE_RATE as f64,
                    self.transcribed_at.as_millis(), (self.processed_at-self.transcribed_at).as_millis(),
                    started.elapsed().as_millis(), self.stop_time.elapsed().as_millis()
                );
                (
                    TranscriptionOutcome::Typed,
                    self.post_process_failed
                        .then_some(NoticeCode::PostProcessFailed),
                )
            }
            Err(error) => {
                log::error!("Failed to paste transcription: {error}");
                (TranscriptionOutcome::Failed, Some(NoticeCode::PasteFailed))
            }
        };
        if let Some(code) = notice {
            self.session
                .notice(&self.app, Notice::new(code, NoticeSource::Dictation));
        }
        self.session.when_current(|| {
            if notice.is_none() {
                notices::clear(&self.app, NoticeSource::Dictation);
                crate::overlay::show_done_overlay(&self.app);
            }
            set_tray_state(&self.app, TrayIconState::Idle);
        });
        self.session
            .complete(&self.app, outcome, self.sample_count, self.stop_time);
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
