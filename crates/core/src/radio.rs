//! Personal recommendations become an ordinary local playlist, independent of playback.
use crate::{local_library, protocol::*};
use anyhow::{Result, ensure};
use musicx_plugin::PluginSource;
use std::{collections::HashSet, sync::Mutex};

static REFRESH: Mutex<()> = Mutex::new(());
const BATCH_PER_SOURCE: usize = 50;

fn collect(sources: &dyn PluginSource, source: &MusicSource) -> (RadioSourceResult, Vec<Track>) {
    let result = (|| -> Result<Vec<Track>> {
        ensure!(
            sources.account(&source.id)?.connected,
            tr!("Sign in to this source to get personal recommendations")
        );
        let tracks = sources.radio_tracks(&source.id)?;
        ensure!(
            tracks.len() <= MAX_QUEUE_TRACKS,
            tr!("Source exceeds 10,000 tracks")
        );
        let mut seen = HashSet::new();
        let mut batch = Vec::new();
        for track in tracks {
            local_library::validate_track(&track)?;
            // Personal results must keep their provider locator for later playback.
            ensure!(
                track.source.is_some(),
                tr!("Track source identifier missing")
            );
            if seen.insert(track_key(&track)) && batch.len() < BATCH_PER_SOURCE {
                batch.push(track);
            }
        }
        Ok(batch)
    })();
    let (tracks, error) = match result {
        Ok(tracks) => (tracks, None),
        Err(error) => (
            Vec::new(),
            Some(error.to_string().chars().take(512).collect()),
        ),
    };
    (
        RadioSourceResult {
            source: source.id.clone(),
            name: source.name.clone(),
            tracks: tracks.len(),
            error,
        },
        tracks,
    )
}

pub fn refresh(sources: &dyn PluginSource) -> Result<RadioRefresh> {
    let _guard = REFRESH
        .try_lock()
        .map_err(|_| anyhow::anyhow!(tr!("Radio is already getting recommendations")))?;
    let eligible: Vec<_> = sources
        .sources()?
        .into_iter()
        .filter(|source| {
            source.installed
                && source.supports(SourceCapability::Playback)
                && source.supports(SourceCapability::Account)
                && (source.supports(SourceCapability::Recommendations)
                    || (source.supports(SourceCapability::Stations)
                        && source.supports(SourceCapability::StationContinuation)))
        })
        .collect();
    let mut reports = Vec::new();
    let mut batches = Vec::new();
    // Bound network fan-out while allowing a slow or unavailable provider to fail independently.
    for group in eligible.chunks(4) {
        std::thread::scope(|scope| -> Result<()> {
            let jobs: Vec<_> = group
                .iter()
                .map(|source| scope.spawn(move || collect(sources, source)))
                .collect();
            for job in jobs {
                let (report, tracks) = job
                    .join()
                    .map_err(|_| anyhow::anyhow!(tr!("Request ended; please try again")))?;
                reports.push(report);
                batches.push(tracks.into_iter());
            }
            Ok(())
        })?;
    }
    // Alternate sources so a large daily list does not bury a short FM batch.
    let mut tracks = Vec::new();
    loop {
        let before = tracks.len();
        for batch in &mut batches {
            if let Some(track) = batch.next() {
                tracks.push(track);
            }
        }
        if tracks.len() == before {
            break;
        }
    }
    let (playlist, added) = local_library::add_radio(tracks)?;
    Ok(RadioRefresh {
        playlist,
        added,
        sources: reports,
    })
}
