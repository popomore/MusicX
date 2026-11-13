//! On-demand, bounded PCM capture for the disposable spectrum view.
mod analyzer;
mod tap;

use analyzer::{Analyzer, FFT_SIZE, HOP_SIZE};
use musicx_core::protocol::spectrum::{Spectrum, SpectrumState};
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const CAPACITY: usize = FFT_SIZE * 2;
const INTEREST_MS: u64 = 500;
static CURRENT: Mutex<Option<Arc<Capture>>> = Mutex::new(None);

/// A single producer (the audio tap) and single consumer (the FFT worker).
/// Atomic samples make dropping stale/overflowed input safe without locks.
pub(crate) struct Capture {
    track_key: String,
    clock: Instant,
    interest: AtomicU64,
    running: AtomicBool,
    seek_serial: AtomicU64,
    pending_seek: AtomicU64,
    format: AtomicU8,
    rate: AtomicU32,
    epoch: AtomicU64,
    ring: Box<[AtomicU64]>,
    write: AtomicUsize,
    read: AtomicUsize,
    overflow: AtomicU64,
    worker: OnceLock<()>,
    frame: Mutex<Option<Frame>>,
}

struct Frame {
    epoch: u64,
    at: Instant,
    spectrum: Spectrum,
}

pub(crate) fn begin(track_key: String) -> Arc<Capture> {
    let capture = Arc::new(Capture::new(track_key));
    *CURRENT.lock().unwrap() = Some(capture.clone());
    capture
}

pub(crate) fn clear() {
    CURRENT.lock().unwrap().take();
}

/// Reading renews a short capture lease. Closing the view stops PCM capture
/// and FFT work automatically, including when the panel process disappears.
pub fn snapshot() -> Spectrum {
    let current = CURRENT.lock().unwrap().clone();
    current.map_or_else(Spectrum::default, |capture| capture.snapshot())
}

impl Capture {
    fn new(track_key: String) -> Self {
        Self {
            track_key,
            clock: Instant::now(),
            interest: AtomicU64::new(0),
            running: AtomicBool::new(false),
            seek_serial: AtomicU64::new(0),
            pending_seek: AtomicU64::new(0),
            format: AtomicU8::new(0),
            rate: AtomicU32::new(0),
            epoch: AtomicU64::new(1),
            ring: (0..CAPACITY).map(|_| AtomicU64::new(0)).collect(),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            overflow: AtomicU64::new(0),
            worker: OnceLock::new(),
            frame: Mutex::new(None),
        }
    }

    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64 + 1
    }

    fn interested(&self) -> bool {
        let interest = self.interest.load(Ordering::Relaxed);
        interest != 0 && self.now().saturating_sub(interest) < INTEREST_MS
    }

    fn active(&self) -> bool {
        self.running.load(Ordering::Relaxed)
            && self.pending_seek.load(Ordering::Acquire) == 0
            && self.interested()
    }

    pub(crate) fn set_running(&self, running: bool) {
        if self.running.swap(running, Ordering::Relaxed) != running {
            self.invalidate();
        }
    }

    pub(crate) fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn begin_seek(&self) -> u64 {
        let serial = self.seek_serial.fetch_add(1, Ordering::Relaxed) + 1;
        self.pending_seek.store(serial, Ordering::Release);
        self.invalidate();
        serial
    }

    pub(crate) fn complete_seek(&self, serial: u64) {
        if self
            .pending_seek
            .compare_exchange(serial, 0, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            self.invalidate();
        }
    }

    pub(crate) fn unavailable(&self) {
        self.format.store(2, Ordering::Relaxed);
    }

    pub(crate) fn install(
        self: &Arc<Self>,
        item: &objc2_av_foundation::AVPlayerItem,
        track_id: i32,
    ) {
        if !tap::install(item, track_id, self.clone()) {
            self.format.store(2, Ordering::Relaxed);
        }
    }

    fn snapshot(self: &Arc<Self>) -> Spectrum {
        let was_interested = self.interested();
        self.interest.store(self.now(), Ordering::Relaxed);
        if !was_interested {
            self.invalidate();
        }
        self.worker.get_or_init(|| {
            let weak = Arc::downgrade(self);
            std::thread::Builder::new()
                .name("musicx-spectrum".into())
                .spawn(move || {
                    let mut analyzer = Analyzer::new();
                    let mut samples = [[0.0; 2]; FFT_SIZE];
                    let (mut count, mut epoch, mut overflow) = (0, 0, 0);
                    while let Some(capture) = weak.upgrade() {
                        let next_epoch = capture.epoch.load(Ordering::Acquire);
                        let next_overflow = capture.overflow.load(Ordering::Acquire);
                        let active = capture.active();
                        if !active || epoch != next_epoch || overflow != next_overflow {
                            count = 0;
                            epoch = next_epoch;
                            overflow = next_overflow;
                            capture
                                .read
                                .store(capture.write.load(Ordering::Acquire), Ordering::Release);
                        } else {
                            let mut read = capture.read.load(Ordering::Relaxed);
                            let write = capture.write.load(Ordering::Acquire);
                            while read != write {
                                let packed = capture.ring[read % CAPACITY].load(Ordering::Relaxed);
                                samples[count] = [
                                    f32::from_bits(packed as u32),
                                    f32::from_bits((packed >> 32) as u32),
                                ];
                                count += 1;
                                read = read.wrapping_add(1);
                                if count == FFT_SIZE {
                                    let sample_rate =
                                        f32::from_bits(capture.rate.load(Ordering::Acquire));
                                    let bands = analyzer.analyze(&samples, sample_rate);
                                    *capture.frame.lock().unwrap() = Some(Frame {
                                        epoch,
                                        at: Instant::now(),
                                        spectrum: Spectrum {
                                            state: SpectrumState::Live,
                                            track_key: capture.track_key.clone(),
                                            sample_rate,
                                            bands,
                                        },
                                    });
                                    samples.copy_within(HOP_SIZE.., 0);
                                    count = FFT_SIZE - HOP_SIZE;
                                }
                            }
                            capture.read.store(read, Ordering::Release);
                        }
                        drop(capture);
                        std::thread::sleep(Duration::from_millis(if active { 20 } else { 100 }));
                    }
                })
                .expect("spectrum worker");
        });
        let empty = |state| Spectrum {
            state,
            track_key: self.track_key.clone(),
            sample_rate: f32::from_bits(self.rate.load(Ordering::Acquire)),
            ..Spectrum::default()
        };
        if !self.running.load(Ordering::Relaxed) {
            return empty(SpectrumState::Idle);
        }
        if self.pending_seek.load(Ordering::Acquire) != 0 {
            return empty(SpectrumState::Waiting);
        }
        if self.format.load(Ordering::Relaxed) == 2 {
            return empty(SpectrumState::Unavailable);
        }
        if let Some(frame) = &*self.frame.lock().unwrap()
            && frame.epoch == self.epoch.load(Ordering::Acquire)
            && frame.at.elapsed() < Duration::from_millis(250)
        {
            return frame.spectrum.clone();
        }
        empty(
            if self.clock.elapsed() > Duration::from_secs(3)
                && self.format.load(Ordering::Relaxed) == 0
            {
                SpectrumState::Unavailable
            } else {
                SpectrumState::Waiting
            },
        )
    }

    /// Never allocates, blocks, computes FFTs, or modifies playback buffers.
    fn push(&self, frames: usize, mut sample: impl FnMut(usize) -> [f32; 2]) {
        if !self.active() {
            return;
        }
        let write = self.write.load(Ordering::Relaxed);
        let read = self.read.load(Ordering::Acquire);
        let room = CAPACITY.saturating_sub(write.wrapping_sub(read));
        let accepted = frames.min(room);
        for i in 0..accepted {
            let [left, right] = sample(i);
            let packed = u64::from(left.to_bits()) | (u64::from(right.to_bits()) << 32);
            self.ring[write.wrapping_add(i) % CAPACITY].store(packed, Ordering::Relaxed);
        }
        self.write
            .store(write.wrapping_add(accepted), Ordering::Release);
        if accepted < frames {
            self.overflow.fetch_add(1, Ordering::Release);
        }
    }
}

pub(crate) fn attach(
    item: &objc2::rc::Retained<objc2_av_foundation::AVPlayerItem>,
    capture: Arc<Capture>,
) {
    tap::load_track(item, capture);
}

#[cfg(test)]
mod tests {
    use super::*;
    use musicx_core::protocol::spectrum::SPECTRUM_FLOOR_DB;

    #[test]
    fn an_older_seek_completion_cannot_reenable_capture_during_a_newer_seek() {
        let capture = Capture::new("fixture".into());
        let first = capture.begin_seek();
        let second = capture.begin_seek();
        capture.complete_seek(first);
        assert_eq!(capture.pending_seek.load(Ordering::Acquire), second);
        capture.complete_seek(second);
        assert_eq!(capture.pending_seek.load(Ordering::Acquire), 0);
    }

    #[test]
    fn expired_leases_and_pause_stop_capture_and_invalidate_frames() {
        let capture = Arc::new(Capture::new("fixture".into()));
        capture.set_running(true);
        capture.push(100, |_| [0.5; 2]);
        assert_eq!(capture.write.load(Ordering::Relaxed), 0);
        capture.interest.store(capture.now(), Ordering::Relaxed);
        capture.push(100, |_| [0.5; 2]);
        assert_eq!(capture.write.load(Ordering::Relaxed), 100);
        let epoch = capture.epoch.load(Ordering::Relaxed);
        capture.set_running(false);
        assert!(capture.epoch.load(Ordering::Relaxed) > epoch);
        let frame = capture.snapshot();
        assert!(frame.state == SpectrumState::Idle);
        assert!(frame.bands.iter().all(|level| *level == SPECTRUM_FLOOR_DB));
    }

    #[test]
    fn slow_consumers_have_bounded_memory_and_drop_overflow() {
        let capture = Capture::new("fixture".into());
        capture.set_running(true);
        capture.interest.store(capture.now(), Ordering::Relaxed);
        capture.push(CAPACITY + 10, |_| [0.2, -0.2]);
        assert_eq!(capture.write.load(Ordering::Relaxed), CAPACITY);
        assert_eq!(capture.overflow.load(Ordering::Relaxed), 1);
        capture.push(10, |_| [0.0; 2]);
        assert_eq!(capture.write.load(Ordering::Relaxed), CAPACITY);
    }
}
