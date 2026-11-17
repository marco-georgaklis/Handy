use super::WarmInput;
use handy_recorder::AudioChunk;
use std::ffi::{c_char, c_void, CString};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

unsafe extern "C" {
    fn handy_warm_open(
        context: *mut c_void,
        frames: u32,
        uid: *const c_char,
        status: *mut i32,
    ) -> *mut c_void;
    fn handy_warm_close(queue: *mut c_void);
    fn handy_warm_now_ns() -> u64;
}

struct Context {
    input: WarmInput,
    failed: Arc<AtomicBool>,
    frames: usize,
    last_block: std::sync::Mutex<Instant>,
    clock: HostClock,
}

/// Relate Rust shortcut times to Core Audio host timestamps once per open.
struct HostClock {
    reference: Instant,
    nanos: u64,
}

impl HostClock {
    fn measure() -> Self {
        let (_, clock) = (0..5)
            .map(|_| {
                let before = Instant::now();
                let nanos = unsafe { handy_warm_now_ns() };
                let span = before.elapsed();
                (
                    span,
                    Self {
                        reference: before + span / 2,
                        nanos,
                    },
                )
            })
            .min_by_key(|(span, _)| *span)
            .expect("five clock readings");
        clock
    }

    fn instant(&self, nanos: u64) -> Option<Instant> {
        if nanos >= self.nanos {
            self.reference
                .checked_add(Duration::from_nanos(nanos - self.nanos))
        } else {
            self.reference
                .checked_sub(Duration::from_nanos(self.nanos - nanos))
        }
    }
}

pub(super) struct TimedInput {
    queue: NonNull<c_void>,
    context: Box<Context>,
}

// AudioQueue is thread safe. Disposal waits for callbacks before freeing context.
unsafe impl Send for TimedInput {}
unsafe impl Sync for TimedInput {}

impl TimedInput {
    pub(super) fn open(
        input: WarmInput,
        frames: usize,
        device: Option<&str>,
        failed: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let uid = device
            .map(|id| {
                let (_, uid) = id
                    .split_once(':')
                    .ok_or("Unrecognized Core Audio device ID")?;
                CString::new(uid).map_err(|_| "Invalid Core Audio device ID")
            })
            .transpose()?;
        let mut context = Box::new(Context {
            input,
            frames,
            failed,
            last_block: std::sync::Mutex::new(Instant::now()),
            clock: HostClock::measure(),
        });
        let mut status = 0;
        // The queue only borrows this stable allocation until synchronous disposal.
        let queue = unsafe {
            handy_warm_open(
                (&mut *context as *mut Context).cast(),
                frames as u32,
                uid.as_ref().map_or(ptr::null(), |id| id.as_ptr()),
                &mut status,
            )
        };
        Ok(Self {
            queue: NonNull::new(queue).ok_or_else(|| format!("Core Audio input error {status}"))?,
            context,
        })
    }

    pub(super) fn stalled(&self) -> bool {
        self.context.last_block.lock().unwrap().elapsed() > Duration::from_secs(3)
    }
}

impl Drop for TimedInput {
    fn drop(&mut self) {
        unsafe { handy_warm_close(self.queue.as_ptr()) };
    }
}

#[no_mangle]
unsafe extern "C" fn handy_warm_failed(context: *const Context) {
    // AudioQueue owns the callback while TimedInput owns the pointed-to box.
    unsafe { &*context }.failed.store(true, Ordering::Release);
}

#[no_mangle]
unsafe extern "C" fn handy_warm_block(
    context: *const Context,
    samples: *const f32,
    frames: u32,
    first_sample_host_ns: u64,
) {
    let received = Instant::now();
    let ctx = unsafe { &*context };
    if frames == 0 || frames as usize > ctx.frames {
        return;
    }
    let Some(first) = ctx.clock.instant(first_sample_host_ns) else {
        return;
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        *ctx.last_block.lock().unwrap() = received;
        if !ctx.input.listening() {
            return;
        }
        // Core Audio owns these mono floats until the callback returns.
        let data = unsafe { std::slice::from_raw_parts(samples, frames as usize) };
        let mut padded;
        let samples = if data.len() == ctx.frames {
            data
        } else {
            padded = vec![0.0; ctx.frames];
            padded[..data.len()].copy_from_slice(data);
            &padded
        };
        let chunk = AudioChunk {
            samples,
            valid_frames: data.len(),
            channels: 1,
            sample_rate: 16_000,
        };
        ctx.input.deliver_from_device(chunk, first);
    }));
    if result.is_err() {
        ctx.failed.store(true, Ordering::Release);
        log::error!("Warm microphone delivery failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_late_core_audio_callbacks_admit_only_samples_inside_the_hold() {
        let origin = Instant::now() - Duration::from_secs(1);
        let input = WarmInput::default();
        let take = input
            .edge(
                "ptt",
                true,
                origin + Duration::from_millis(12),
                Duration::ZERO,
            )
            .unwrap();
        input.edge(
            "ptt",
            false,
            origin + Duration::from_millis(16),
            Duration::ZERO,
        );
        let ctx = Context {
            input,
            frames: 160,
            failed: Arc::default(),
            last_block: std::sync::Mutex::new(origin),
            clock: HostClock {
                reference: origin,
                nanos: 0,
            },
        };
        let before = [1.; 160];
        let during = [2.; 160];
        // Native buffers captured 10 ms apart both arrive one second late.
        unsafe {
            handy_warm_block(&ctx, before.as_ptr(), 160, 0);
            handy_warm_block(&ctx, during.as_ptr(), 160, 10_000_000);
        }
        let actual = take.queued_samples();
        assert_eq!(actual.len(), 160);
        assert_eq!(actual.iter().filter(|&&v| v == 2.).count(), 64);
        assert!(!actual.contains(&1.));
    }
}
