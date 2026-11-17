//! One open device can deliver the end of one hold and the start of the next
//! in a single hardware block. Route just the samples inside each hold.
use super::CaptureSink;
use handy_recorder::{AudioChunk, Sink};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Routes {
    takes: Vec<Weak<WarmTake>>,
    key: Option<(String, Weak<WarmTake>)>,
}

#[derive(Clone, Default)]
pub(crate) struct WarmInput(Arc<Mutex<Routes>>);

/// An input edge owns the take until the coordinator attaches the ordinary
/// VAD/streaming sink. An ignored edge drops it, including any queued audio.
pub(crate) struct WarmTake {
    start: Instant,
    state: Mutex<TakeState>,
    delivered: Condvar,
}

#[derive(Default)]
struct TakeState {
    end: Option<Instant>,
    drained: bool,
    blocks: Vec<Vec<f32>>,
    sink: Option<CaptureSink>,
}

impl WarmTake {
    fn new(at: Instant) -> Arc<Self> {
        Arc::new(Self {
            start: at,
            state: Mutex::default(),
            delivered: Condvar::new(),
        })
    }

    fn end_at(&self, at: Instant) {
        let mut state = self.state.lock().unwrap();
        state.end = Some(state.end.map_or(at, |end| end.min(at)));
    }

    fn trim_tail(&self, at: Instant) {
        let mut state = self.state.lock().unwrap();
        if let Some(end) = &mut state.end {
            *end = (*end).min(at);
        }
    }

    fn deliver(&self, chunk: AudioChunk<'_>, end: Instant) {
        let mut state = self.state.lock().unwrap();
        if state.drained {
            return;
        }
        let frame = Duration::from_secs_f64(chunk.valid_frames as f64 / chunk.sample_rate as f64);
        let start = end - frame;
        let index = |time: Instant| {
            (time.saturating_duration_since(start).as_secs_f64() * chunk.sample_rate as f64).ceil()
                as usize
        };
        let first = index(self.start).min(chunk.valid_frames);
        let last = state
            .end
            .map_or(chunk.valid_frames, |t| index(t).min(chunk.valid_frames));
        if first < last {
            let mut clipped;
            let samples = if first == 0 && last == chunk.samples.len() {
                chunk.samples
            } else {
                clipped = vec![0.0; chunk.samples.len()];
                clipped[first..last].copy_from_slice(&chunk.samples[first..last]);
                &clipped
            };
            if let Some(sink) = &mut state.sink {
                sink.process_chunk(AudioChunk { samples, ..chunk });
            } else {
                state.blocks.push(samples.to_vec());
            }
        }
        if state.end.is_some_and(|cut| cut <= end) {
            state.drained = true;
            self.delivered.notify_all();
        }
    }

    pub(super) fn attach(&self, mut sink: CaptureSink) {
        let mut state = self.state.lock().unwrap();
        for samples in state.blocks.drain(..) {
            sink.process_chunk(AudioChunk {
                samples: &samples,
                sample_rate: 16_000,
                channels: 1,
                valid_frames: samples.len(),
            });
        }
        state.sink = Some(sink);
    }

    pub(super) fn finish(&self) -> CaptureSink {
        self.end_at(Instant::now());
        let state = self.state.lock().unwrap();
        // A disconnected microphone may never deliver the final frame.
        let (mut state, _) = self
            .delivered
            .wait_timeout_while(state, Duration::from_millis(100), |s| !s.drained)
            .unwrap();
        state.drained = true;
        state
            .sink
            .take()
            .expect("active warm capture has an attached sink")
    }
}

impl WarmInput {
    fn insert(routes: &mut Routes, at: Instant) -> Arc<WarmTake> {
        let take = WarmTake::new(at);
        routes.takes.retain(|t| t.strong_count() > 0);
        routes.takes.push(Arc::downgrade(&take));
        take
    }

    pub(super) fn contains(&self, take: &Arc<WarmTake>) -> bool {
        self.0
            .lock()
            .unwrap()
            .takes
            .iter()
            .any(|slot| slot.ptr_eq(&Arc::downgrade(take)))
    }

    pub(super) fn begin(&self) -> Arc<WarmTake> {
        Self::insert(&mut self.0.lock().unwrap(), Instant::now())
    }

    pub(crate) fn edge(
        &self,
        binding: &str,
        pressed: bool,
        at: Instant,
        tail: Duration,
    ) -> Option<Arc<WarmTake>> {
        let mut routes = self.0.lock().unwrap();
        if pressed {
            if routes
                .key
                .as_ref()
                .is_some_and(|(_, take)| take.strong_count() > 0)
            {
                return None;
            }
            for older in routes.takes.iter().filter_map(Weak::upgrade) {
                older.trim_tail(at);
            }
            let take = Self::insert(&mut routes, at);
            routes.key = Some((binding.into(), Arc::downgrade(&take)));
            return Some(take);
        }
        if routes.key.as_ref().is_some_and(|(id, _)| id == binding) {
            if let Some(take) = routes.key.take().and_then(|(_, take)| take.upgrade()) {
                take.end_at(at + tail);
            }
        }
        None
    }
}

impl Sink for WarmInput {
    fn process_chunk(&mut self, chunk: AudioChunk<'_>) {
        let at = Instant::now();
        let takes: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .takes
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for take in takes {
            take.deliver(chunk, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_held_intervals_keep_their_own_pcm_while_the_pipeline_drains() {
        let input = WarmInput::default();
        let origin = Instant::now();
        let at = |ms| origin + Duration::from_millis(ms);
        let first = input.edge("ptt", true, at(2), Duration::ZERO).unwrap();
        input.edge("ptt", false, at(4), Duration::from_secs(1));
        let second = input.edge("ptt", true, at(6), Duration::ZERO).unwrap();
        input.edge("ptt", false, at(8), Duration::ZERO);
        let samples = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let chunk = AudioChunk {
            samples: &samples,
            sample_rate: 1000,
            channels: 1,
            valid_frames: 10,
        };
        for take in [&first, &second] {
            take.deliver(chunk, at(10));
        }
        assert_eq!(
            first.state.lock().unwrap().blocks[0],
            [0., 0., 3., 4., 5., 6., 0., 0., 0., 0.]
        );
        assert_eq!(
            second.state.lock().unwrap().blocks[0],
            [0., 0., 0., 0., 0., 0., 7., 8., 0., 0.]
        );
    }
}
