//! Audio acquisition only. No Connect controller or audio-device backend.
use super::{MAX_PCM, Request, spotify_id};
use anyhow::{Context, Result, bail, ensure};
use librespot_core::{
    SpotifyUri, authentication::Credentials, config::SessionConfig, session::Session,
    spotify_id::SpotifyId,
};
use librespot_playback::{
    NUM_CHANNELS, SAMPLE_RATE,
    audio_backend::{Sink, SinkError, SinkResult},
    config::{Bitrate, PlayerConfig},
    convert::Converter,
    decoder::AudioPacket,
    mixer::NoOpVolume,
    player::{Player, PlayerEvent},
};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsFd,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct Output {
    bytes: AtomicU64,
    failed: AtomicBool,
}
struct PcmSink {
    file: File,
    output: Arc<Output>,
}

fn complete_audio(bytes: u64, duration_ms: u32) -> bool {
    let expected = u64::from(duration_ms);
    let decoded = bytes * 1000 / 176_400;
    let tolerance = (expected / 100).clamp(50, 1000);
    bytes > 0 && duration_ms > 0 && decoded.abs_diff(expected) <= tolerance
}
impl Sink for PcmSink {
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let result = (|| -> Result<()> {
            let AudioPacket::Samples(samples) = packet else {
                bail!("Expected decoded PCM")
            };
            ensure!(
                samples.len() <= 4 * 1024 * 1024 && samples.iter().all(|n| n.is_finite()),
                "Invalid PCM packet"
            );
            let bytes: Vec<u8> = converter
                .f64_to_s16(&samples)
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect();
            let total = self.output.bytes.load(Ordering::Acquire) + bytes.len() as u64;
            ensure!(total <= MAX_PCM, "PCM output limit reached");
            self.file.write_all(&bytes)?;
            self.output.bytes.store(total, Ordering::Release);
            Ok(())
        })();
        if result.is_err() {
            self.output.failed.store(true, Ordering::Release);
            return Err(SinkError::OnWrite("MusicX audio acquisition failed".into()));
        }
        Ok(())
    }
}

async fn decode(request: &Request, file: File) -> Result<()> {
    ensure!(
        SAMPLE_RATE == 44_100 && NUM_CHANNELS == 2,
        "Unsupported PCM format"
    );
    let id = SpotifyUri::Track {
        id: SpotifyId::from_base62(spotify_id(&request.id)?)?,
    };
    let session = Session::new(SessionConfig::default(), None);
    // No disk token/audio cache and no credential-saving flag.
    session
        .connect(Credentials::with_access_token(&request.access), false)
        .await?;
    let output = Arc::new(Output::default());
    let sink_output = output.clone();
    let player = Player::new(
        PlayerConfig {
            bitrate: if request.high {
                Bitrate::Bitrate320
            } else {
                Bitrate::Bitrate160
            },
            gapless: false,
            normalisation: false,
            ditherer: None,
            ..Default::default()
        },
        session.clone(),
        Box::new(NoOpVolume),
        move || {
            Box::new(PcmSink {
                file,
                output: sink_output,
            })
        },
    );
    let mut events = player.get_player_event_channel();
    player.load(id, true, 0);
    let mut duration_ms = 0u32;
    while let Some(event) = events.recv().await {
        match event {
            PlayerEvent::TrackChanged { audio_item } => duration_ms = audio_item.duration_ms,
            PlayerEvent::EndOfTrack { .. } => {
                let bytes = output.bytes.load(Ordering::Acquire);
                // The decoder can report EndOfTrack after a read error. Never
                // hand a truncated file to AVPlayer as a complete acquisition.
                ensure!(
                    !output.failed.load(Ordering::Acquire) && complete_audio(bytes, duration_ms),
                    "Incomplete PCM output"
                );
                player.stop();
                session.shutdown();
                return Ok(());
            }
            PlayerEvent::Unavailable { .. }
            | PlayerEvent::Stopped { .. }
            | PlayerEvent::SessionDisconnected { .. } => bail!("Source audio unavailable"),
            _ => {}
        }
    }
    bail!("Source audio ended without completion")
}

/// Private helper mode. Its only output is PCM; errors never print credentials,
/// URLs, source responses or third-party diagnostic logs.
pub fn worker() -> ExitCode {
    let result = (|| -> Result<()> {
        ensure!(std::env::args().count() == 2, "Invalid decoder invocation");
        let mut input = zeroize::Zeroizing::new(Vec::new());
        std::io::stdin()
            .take(16 * 1024 + 1)
            .read_to_end(&mut input)?;
        ensure!(input.len() <= 16 * 1024, "Invalid decoder input");
        let request: Request = serde_json::from_slice(&input)?;
        ensure!(
            !request.access.is_empty()
                && request.access.len() <= 8192
                && request.access.bytes().all(|b| b.is_ascii_graphic()),
            "Invalid account credentials"
        );
        let file = File::from(std::io::stdout().as_fd().try_clone_to_owned()?);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(110), decode(&request, file))
                .await
                .context("Audio acquisition timed out")?
        })?;
        Ok(())
    })();
    if result.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_audio_is_not_mistaken_for_a_complete_track() {
        assert!(complete_audio(176_400 * 180, 180_000));
        assert!(complete_audio(176_400, 1000));
        assert!(!complete_audio(4, 1000));
        assert!(!complete_audio(176_400, 180_000));
        assert!(!complete_audio(176_400 * 180, 1000));
        assert!(!complete_audio(0, 0));
    }

    #[test]
    fn sink_writes_pcm_and_rejects_output_after_its_bound() {
        let root = std::env::temp_dir().join(format!(
            "musicx-pcm-{}",
            musicx_core::storage::random_hex::<8>()
        ));
        let file = File::create(&root).unwrap();
        let output = Arc::new(Output::default());
        let mut sink = PcmSink {
            file,
            output: output.clone(),
        };
        sink.write(
            AudioPacket::Samples(vec![0.0, 0.5, -0.5, 1.0]),
            &mut Converter::new(None),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(&root).unwrap(),
            [0, 0, 0, 64, 0, 192, 255, 127]
        );
        output.bytes.store(MAX_PCM, Ordering::Release);
        assert!(
            sink.write(
                AudioPacket::Samples(vec![0.0, 0.0]),
                &mut Converter::new(None)
            )
            .is_err()
        );
        assert!(output.failed.load(Ordering::Acquire));
        assert_eq!(std::fs::metadata(&root).unwrap().len(), 8);
        drop(sink);
        std::fs::remove_file(root).unwrap();
    }
}
