//! On-demand metadata. No lyric or album requests happen while the panel is closed.
use crate::{
    mygo,
    protocol::{AlbumPage, Track, TrackInfo},
};
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use musicx_plugin_api::Media;

pub fn resolve_with_quality(track: Track, quality: crate::protocol::Quality) -> Result<Media> {
    ensure!(
        !track.title.is_empty() && track.title.len() <= 2048 && track.artist.len() <= 4096,
        tr!("Invalid source track information")
    );
    let source = track.source.as_ref().context(tr!(
        "This track has no plugin source; add it again from an installed source"
    ))?;
    ensure!(
        !source.id.is_empty() && source.id.len() <= 512 && mygo::valid_id(&source.plugin),
        tr!("Invalid source")
    );
    mygo::resolve_with_quality(track, quality)
}

pub fn info(mut track: Track) -> TrackInfo {
    mygo::fill_artwork(&mut track);
    let source_name = track.source.as_ref().map_or_else(
        || tr!("No source connected").to_owned(),
        |s| mygo::source_name(&s.plugin),
    );
    let source_link = track
        .source_url
        .as_deref()
        .and_then(musicx_core::metadata::public_url)
        .map(|url| crate::protocol::SourceLink {
            label: tr!("Track source").into(),
            url,
        })
        .or_else(|| {
            let source = track.source.as_ref()?;
            mygo::source_repository(&source.plugin).map(|url| crate::protocol::SourceLink {
                label: tr!("Source plugin").into(),
                url,
            })
        });
    let (lyrics, lyrics_error) = match mygo::lyrics(&track) {
        Ok(value) => (value, None),
        Err(_) => (
            Default::default(),
            Some(tr!("Lyrics unavailable; please try again later").into()),
        ),
    };
    TrackInfo {
        track,
        source_name,
        source_link,
        lyrics,
        lyrics_error,
    }
}

pub fn album(track: Track) -> Result<AlbumPage> {
    mygo::album(&track)
}
