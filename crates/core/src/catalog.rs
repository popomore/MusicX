//! On-demand metadata. No lyric or album requests happen while the panel is closed.
use crate::{
    mygo, netease,
    protocol::{AlbumPage, Track, TrackInfo},
};
use anyhow::{Context, Result, ensure};

pub fn resolve(track: Track) -> Result<netease::Media> {
    resolve_with_quality(track, crate::audio_quality::preferred())
}
pub fn resolve_with_quality(
    track: Track,
    quality: crate::protocol::Quality,
) -> Result<netease::Media> {
    ensure!(
        !track.title.is_empty() && track.title.len() <= 2048 && track.artist.len() <= 4096,
        "无效曲目信息"
    );
    if let Some(source) = &track.source {
        ensure!(
            !source.id.is_empty() && source.id.len() <= 512 && source.plugin.len() <= 100,
            "无效来源"
        );
        mygo::resolve_with_quality(track, quality)
    } else {
        mygo::ensure_netease()?;
        mygo::resolve_with_quality(netease::with_plugin_source(track), quality)
    }
}

pub fn info(mut track: Track) -> TrackInfo {
    mygo::fill_artwork(&mut track);
    let source_name = track
        .source
        .as_ref()
        .map_or_else(|| "网易云音乐".to_owned(), |s| mygo::source_name(&s.plugin));
    let source_link = if let Some(url) = track.source_url.clone().or_else(|| {
        track
            .source
            .is_none()
            .then(|| format!("https://music.163.com/#/song?id={}", track.id))
    }) {
        Some(crate::protocol::SourceLink {
            label: "歌曲来源".into(),
            url,
        })
    } else {
        track
            .source
            .as_ref()
            .and_then(|source| match source.plugin.as_str() {
                "bodian" => Some("https://github.com/amqsy/mygo-music-bodian"),
                "kugou-lite" => Some("https://github.com/bitbyte404/mygo-music-kugou-lite"),
                _ => None,
            })
            .map(|url| crate::protocol::SourceLink {
                label: "来源插件".into(),
                url: url.into(),
            })
    };
    let result = if track.source.is_some() {
        mygo::lyrics(&track)
    } else {
        netease::lyrics(track.id)
    };
    let (lyrics, lyrics_error) = match result {
        Ok(value) => (value, None),
        Err(_) => (Default::default(), Some("歌词暂不可用，可稍后重试".into())),
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
    if track.source.is_some() {
        mygo::album(&track)
    } else {
        netease::album(&track.album.context("当前曲目没有专辑信息")?.id)
    }
}
