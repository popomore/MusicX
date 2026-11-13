//! Drive the real AVPlayer and MediaToolbox tap on the main run loop, in isolation.
use musicx_core::{
    player::Action,
    protocol::{
        self, AlbumPage, AudioQuality, Quality, Track, TrackInfo,
        spectrum::{SPECTRUM_FLOOR_DB, Spectrum, SpectrumState, band_edge},
    },
};
use musicx_platform_apple::{player, spectrum};
use musicx_plugin_api::{DataSource, Media};
use objc2::rc::autoreleasepool;
use objc2_foundation::{NSDate, NSRunLoop};
use std::{
    path::Path,
    time::{Duration, Instant},
};

struct Source;
impl DataSource for Source {
    fn resolve(&self, _: Track, _: Quality) -> anyhow::Result<Media> {
        unreachable!()
    }
    fn info(&self, _: Track) -> TrackInfo {
        unreachable!()
    }
    fn album(&self, _: Track) -> anyhow::Result<AlbumPage> {
        unreachable!()
    }
}

fn pump() {
    autoreleasepool(|_| {
        NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.02))
    });
}

fn wait_for(mut accept: impl FnMut(&Spectrum) -> bool) -> Spectrum {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        pump();
        player::tick();
        let frame = spectrum::snapshot();
        if accept(&frame) {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "real AVPlayer spectrum timed out (live={}, unavailable={}, sample_rate={})",
            frame.state == SpectrumState::Live,
            frame.state == SpectrumState::Unavailable,
            frame.sample_rate
        );
    }
}

fn assert_tone(frame: &Spectrum, frequency: f32) {
    let peak = frame
        .bands
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    assert!(
        frequency >= band_edge(peak.saturating_sub(1)) && frequency < band_edge(peak + 2),
        "{frequency} Hz landed in band {peak}"
    );
    assert!(
        frame.bands[peak] > -15.0,
        "tap must capture PCM even with player volume zero"
    );
}

fn media(path: &Path, id: u64) -> Media {
    let track = Track {
        id,
        title: format!("Spectrum fixture {id}"),
        artist: "MusicX tests".into(),
        artists: vec![],
        episode: None,
        duration: 8.0,
        cover_url: None,
        album: None,
        source_url: None,
        source: None,
    };
    Media {
        track,
        url: format!("file://{}", path.display()).parse().unwrap(),
        uses_http: false,
        user_agent: None,
        quality: AudioQuality::default(),
        gain_db: None,
        peak: None,
        lease: None,
    }
}

fn wave(path: &Path, frequencies: [f32; 2]) {
    let frames = 48_000 * 8;
    let bytes = (frames * 8) as u32;
    let mut wav = Vec::with_capacity(bytes as usize + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend((bytes + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend(16_u32.to_le_bytes());
    wav.extend(3_u16.to_le_bytes());
    wav.extend(2_u16.to_le_bytes());
    wav.extend(48_000_u32.to_le_bytes());
    wav.extend((48_000_u32 * 8).to_le_bytes());
    wav.extend(8_u16.to_le_bytes());
    wav.extend(32_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend(bytes.to_le_bytes());
    for i in 0..frames {
        let hz = frequencies[usize::from(i >= 48_000 * 4)];
        let sample = 0.5 * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin();
        wav.extend(sample.to_le_bytes());
        wav.extend((-sample).to_le_bytes());
    }
    std::fs::write(path, wav).unwrap();
}

fn main() {
    if std::env::var_os("MUSICX_SPECTRUM_CHILD").is_none() {
        let directory =
            std::env::temp_dir().join(format!("musicx-spectrum-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .env("MUSICX_SPECTRUM_CHILD", "1")
            .env("MUSICX_DATA_DIR", &directory)
            .status()
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        assert!(status.success(), "isolated AVPlayer spectrum test failed");
        return;
    }
    let directory = protocol::data_dir().unwrap();
    let first = directory.join("first.wav");
    let second = directory.join("second.wav");
    wave(&first, [1_000.0, 8_000.0]);
    wave(&second, [100.0, 100.0]);
    player::configure(&Source).unwrap();
    player::submit(Action::Volume(0.0), None);
    player::submit(Action::Play(media(&first, 1)), None);
    let live = wait_for(|frame| frame.state == SpectrumState::Live);
    assert_tone(&live, 1_000.0);
    player::submit(Action::Pause, None);
    let paused = wait_for(|frame| frame.state == SpectrumState::Idle);
    assert_eq!(paused.bands, [SPECTRUM_FLOOR_DB; 32]);
    // Re-load to resume without asking the fake source for a new URL.
    player::submit(Action::Play(media(&first, 1)), None);
    wait_for(|frame| frame.state == SpectrumState::Live);
    player::submit(
        Action::Seek {
            seconds: 5.0,
            track_key: None,
        },
        None,
    );
    let sought = wait_for(|frame| frame.state == SpectrumState::Live && frame.bands[27] > -15.0);
    assert_tone(&sought, 8_000.0);
    player::submit(Action::Play(media(&second, 2)), None);
    let switched =
        wait_for(|frame| frame.state == SpectrumState::Live && frame.track_key == "netease:2");
    assert_tone(&switched, 100.0);
    player::shutdown();
    assert!(spectrum::snapshot().state == SpectrumState::Idle);
    println!(
        "real AVPlayer spectrum: PCM capture, stereo phase, zero volume, pause, seek, track replacement and shutdown passed"
    );
}
