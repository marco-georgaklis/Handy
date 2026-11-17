use super::{CaptureSink, WarmInput, WarmTake};
use crate::notices::NoticeCode;
use handy_recorder::ErrorKind;
use handy_recorder::{EndReason, Error, Recorder, RecorderConfig, StartError, Stopped};
use std::sync::{Arc, Mutex};

pub(super) enum Device {
    Standard(Recorder<CaptureSink>),
    Warm {
        recorder: Recorder<WarmInput>,
        input: WarmInput,
        active: Mutex<Option<Arc<WarmTake>>>,
    },
}

impl Device {
    pub fn open(
        config: RecorderConfig,
        warm: bool,
        failed: impl FnOnce(Error) + Send + 'static,
    ) -> Result<Self, Error> {
        if warm {
            let recorder = Recorder::open_with_failure_handler(config, failed)?;
            let input = WarmInput::default();
            recorder.start(input.clone())?;
            return Ok(Self::Warm {
                recorder,
                input,
                active: Mutex::default(),
            });
        }
        Ok(Self::Standard(Recorder::open_with_failure_handler(
            config, failed,
        )?))
    }

    pub fn input(&self) -> Option<WarmInput> {
        match self {
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
            Self::Warm { recorder, .. } => recorder.close(),
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
