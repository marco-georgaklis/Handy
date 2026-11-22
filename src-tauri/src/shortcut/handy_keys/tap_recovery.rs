//! Recover the handy-keys event tap after macOS switches it off silently.
//!
//! macOS disables an event tap whose callback is slow to return, and is
//! supposed to tell the tap with a `TapDisabledByTimeout` pseudo-event.
//! handy-keys re-enables its tap from that pseudo-event and nothing else (its
//! old 100ms `CGEventTapIsEnabled` poll leaked kernel IPC vouchers until the
//! machine panicked, cjpais/Handy#1827). Across sleep and the lock screen the
//! pseudo-event can be lost: an event in flight when the Mac went to sleep was
//! charged hours of latency, the tap was switched off, the callback was never
//! told, and every hotkey stayed dead until Handy was restarted.
//!
//! Such a tap can only be turned back on by the process that owns its port,
//! which handy-keys keeps private, so recovery rebuilds the listener. It runs
//! only after the events that precede this failure (wake, unlock, returning to
//! the session), with a few spaced-out checks each, so there is no periodic
//! WindowServer traffic.

use log::{debug, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::AppHandle;

/// When to look at the tap after a wake or unlock. The stale event that gets
/// the tap disabled is delivered around wake, but unlock can come first or
/// later, so a few checks cover both orders.
const CHECK_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(10),
    Duration::from_secs(60),
];

/// Distributed notification the system posts when the screen is unlocked.
const SCREEN_UNLOCKED: &str = "com.apple.screenIsUnlocked";

static STARTED: AtomicBool = AtomicBool::new(false);

/// A tap this process owns, reduced to what recovery needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedTap {
    pub tapping_process: i32,
    pub enabled: bool,
}

/// Whether any tap owned by `pid` is disabled.
///
/// Every tap Handy owns is a handy-keys listener: the shortcut manager's, and
/// briefly the shortcut recorder's or the keyboard diagnostic's. A disabled
/// short-lived one causes a needless but harmless rebuild.
pub fn any_disabled_for(taps: &[OwnedTap], pid: i32) -> bool {
    taps.iter()
        .any(|tap| tap.tapping_process == pid && !tap.enabled)
}

/// Lists every event tap in the session. `None` if WindowServer refused.
fn list_taps() -> Option<Vec<OwnedTap>> {
    use objc2_core_graphics::{CGError, CGEventTapInformation, CGGetEventTapList};

    let mut count: u32 = 0;
    // SAFETY: a null list asks only for the count, written to `count`.
    let err = unsafe { CGGetEventTapList(0, std::ptr::null_mut(), &mut count) };
    if err != CGError::Success {
        warn!("event tap check: CGGetEventTapList count failed ({err:?})");
        return None;
    }

    // SAFETY: CGEventTapInformation is plain old data; all-zero is valid.
    let mut list: Vec<CGEventTapInformation> = vec![unsafe { std::mem::zeroed() }; count as usize];
    // SAFETY: `list` holds `count` entries; the call writes at most that many
    // and reports how many it wrote.
    let err = unsafe { CGGetEventTapList(count, list.as_mut_ptr(), &mut count) };
    if err != CGError::Success {
        warn!("event tap check: CGGetEventTapList failed ({err:?})");
        return None;
    }
    list.truncate(count as usize);

    Some(
        list.iter()
            .map(|info| OwnedTap {
                tapping_process: info.tappingProcess,
                enabled: info.enabled,
            })
            .collect(),
    )
}

/// Whether one of this process's event taps is currently disabled.
pub fn own_tap_disabled() -> bool {
    let Some(taps) = list_taps() else {
        // Unknown is treated as healthy: a rebuild on every failed query
        // would churn the listener without evidence that it is broken.
        return false;
    };
    any_disabled_for(&taps, std::process::id() as i32)
}

/// Runs the spaced-out checks after a wake or unlock, off the main thread.
fn schedule_checks(app: &AppHandle, reason: &'static str) {
    debug!("event tap check scheduled after {reason}");
    let app = app.clone();
    std::thread::spawn(move || {
        for delay in CHECK_DELAYS {
            std::thread::sleep(delay);
            super::recover_tap_if_disabled(&app, reason);
        }
    });
}

/// Watch for wake, unlock and session switches for the life of the app.
/// Safe to call more than once; only the first call registers.
pub fn start(app: &AppHandle) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let observer_app = app.clone();
    // Notification observers are registered from the main thread so their
    // blocks run on the main queue, which the app's run loop drains.
    if let Err(e) = app.run_on_main_thread(move || register_observers(&observer_app)) {
        STARTED.store(false, Ordering::SeqCst);
        warn!("event tap recovery not started: {e}");
    }
}

fn register_observers(app: &AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    };
    use objc2_foundation::{
        NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
        NSOperationQueue, NSString,
    };
    use std::ptr::NonNull;

    let observe = |center: &NSNotificationCenter,
                   name: &NSNotificationName,
                   reason: &'static str| {
        let app = app.clone();
        let block = RcBlock::new(move |_: NonNull<NSNotification>| schedule_checks(&app, reason));
        // SAFETY: the block only clones a handle and spawns a thread, so it
        // is fine on the main queue; no object filter is passed.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        // The observer lives as long as the app; the center needs the token
        // only to remove it, which never happens.
        std::mem::forget(token);
    };

    let workspace_center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: these are AppKit's exported notification name constants.
    unsafe {
        observe(&workspace_center, NSWorkspaceDidWakeNotification, "wake");
        observe(
            &workspace_center,
            NSWorkspaceSessionDidBecomeActiveNotification,
            "session became active",
        );
    }

    let unlocked = NSString::from_str(SCREEN_UNLOCKED);
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    observe(&distributed, &unlocked, "screen unlocked");

    info!("event tap recovery watching wake, unlock and session switches");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(pid: i32, enabled: bool) -> OwnedTap {
        OwnedTap {
            tapping_process: pid,
            enabled,
        }
    }

    #[test]
    fn healthy_own_tap_needs_no_recovery() {
        assert!(!any_disabled_for(&[tap(7, true), tap(9, false)], 7));
    }

    #[test]
    fn disabled_own_tap_needs_recovery() {
        assert!(any_disabled_for(
            &[tap(9, true), tap(7, true), tap(7, false)],
            7
        ));
    }

    #[test]
    fn other_processes_disabled_taps_are_ignored() {
        assert!(!any_disabled_for(&[tap(9, false), tap(11, false)], 7));
    }

    #[test]
    fn no_taps_needs_no_recovery() {
        assert!(!any_disabled_for(&[], 7));
    }

    /// Exercises the real WindowServer query: a tap this process creates and
    /// then disables must show up as disabled, and enabled again after.
    #[test]
    fn detects_a_tap_disabled_without_notice() {
        use objc2_core_graphics::{
            CGEvent, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        };
        use std::ffi::c_void;
        use std::ptr::NonNull;

        unsafe extern "C-unwind" fn pass(
            _: objc2_core_graphics::CGEventTapProxy,
            _: CGEventType,
            event: NonNull<CGEvent>,
            _: *mut c_void,
        ) -> *mut CGEvent {
            event.as_ptr()
        }

        let mask = 1u64 << CGEventType::FlagsChanged.0;
        // SAFETY: listen-only tap with a pass-through callback and no refcon.
        let Some(tap) = (unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::TailAppendEventTap,
                CGEventTapOptions::ListenOnly,
                mask,
                Some(pass),
                std::ptr::null_mut(),
            )
        }) else {
            eprintln!("skipping: this process may not create event taps");
            return;
        };

        assert!(!own_tap_disabled(), "a fresh tap is enabled");
        // Disabling from code sends no pseudo-event, the same state macOS
        // left the app in after sleep.
        CGEvent::tap_enable(&tap, false);
        assert!(own_tap_disabled(), "the disabled tap is reported");
        CGEvent::tap_enable(&tap, true);
        assert!(!own_tap_disabled(), "re-enabled tap is healthy again");
        drop(tap);
    }
}
