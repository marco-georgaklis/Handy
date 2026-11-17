use super::{CaptureSink, WarmInput, WarmTake};
use crate::notices::NoticeCode;
use handy_recorder::ErrorKind;
use handy_recorder::{EndReason, Error, Recorder, RecorderConfig, StartError, Stopped};
use std::sync::{Arc, Mutex};

pub(super) enum Device {
    Standard(Recorder<CaptureSink>),
    #[cfg(target_os = "macos")]
    Warm {
        recorder: super::timed_input::TimedInput,
        input: WarmInput,
        active: Mutex<Option<Arc<WarmTake>>>,
    },
}

impl Device {
    pub fn open(
        config: RecorderConfig,
        warm: bool,
        failed: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, Error> {
        #[cfg(target_os = "macos")]
        if warm
            && config.channels == handy_recorder::Channels::MixToMono
            && handy_recorder::permission_status() != handy_recorder::Permission::Denied
        {
            let input = WarmInput::default();
            match super::timed_input::TimedInput::open(
                input.clone(),
                config.frames_per_chunk.unwrap_or(480),
                config.device.as_deref(),
                failed.clone(),
            ) {
                Ok(recorder) => {
                    return Ok(Self::Warm {
                        recorder,
                        input,
                        active: Mutex::default(),
                    })
                }
                Err(error) => log::warn!(
                    "Timestamped microphone unavailable; opening standard input: {error}"
                ),
            }
        }
        Ok(Self::Standard(Recorder::open_with_failure_handler(
            config,
            move |_| failed.store(true, std::sync::atomic::Ordering::Release),
        )?))
    }

    pub fn stalled(&self) -> bool {
        #[cfg(target_os = "macos")]
        if let Self::Warm { recorder, .. } = self {
            return recorder.stalled();
        }
        false
    }

    pub fn input(&self) -> Option<WarmInput> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Warm { input, .. } => Some(input.clone()),
            Self::Standard(_) => None,
        }
    }

    pub fn start(
        &self,
        sink: CaptureSink,
        prepared: Option<Arc<WarmTake>>,
    ) -> Result<(), StartError<CaptureSink>> {
        match self {
            Self::Standard(recorder) => recorder.start(sink),
            #[cfg(target_os = "macos")]
            Self::Warm { input, active, .. } => {
                let take = prepared
                    .filter(|take| input.contains(take))
                    .unwrap_or_else(|| input.begin());
                let mut active = active.lock().unwrap();
                debug_assert!(active.is_none());
                take.attach(sink);
                *active = Some(take);
                Ok(())
            }
        }
    }

    pub fn stop(&self) -> anyhow::Result<Stopped<CaptureSink>> {
        match self {
            Self::Standard(recorder) => Ok(recorder.stop()?),
            #[cfg(target_os = "macos")]
            Self::Warm { active, .. } => {
                let take = active
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("No warm recording to collect"))?;
                Ok(Stopped {
                    sink: take.finish(),
                    end_reason: EndReason::StopCalled,
                    dropped_frames: 0,
                })
            }
        }
    }

    pub fn close(self) -> Result<(), Error> {
        match self {
            Self::Standard(recorder) => recorder.close(),
            #[cfg(target_os = "macos")]
            Self::Warm { recorder, .. } => {
                drop(recorder);
                Ok(())
            }
        }
    }
}

/// The notice for a recording that couldn't start.
pub fn recording_notice_code(error: &anyhow::Error) -> NoticeCode {
    match error
        .downcast_ref::<handy_recorder::Error>()
        .map(handy_recorder::Error::kind)
    {
        Some(ErrorKind::PermissionDenied) => NoticeCode::MicPermissionDenied,
        // A missing selected microphone falls back to the default, so this
        // means there is no input device at all.
        Some(ErrorKind::DeviceUnavailable) => NoticeCode::NoMicrophone,
        _ => NoticeCode::RecordingFailed,
    }
}
