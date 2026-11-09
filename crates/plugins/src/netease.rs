//! Legacy numeric-ID entry points backed entirely by the MyGo NetEase plugin.
pub use crate::protocol::Track;
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use musicx_plugin_api::Media;

pub fn with_plugin_source(mut track: Track) -> Track {
    if track.source.is_none() {
        track.source = Some(crate::protocol::Source {
            plugin: "netease".into(),
            id: track.id.to_string(),
        });
        track
            .source_url
            .get_or_insert_with(|| format!("https://music.163.com/song?id={}", track.id));
    }
    track
}
pub fn search(query: String, limit: u64) -> Result<Vec<Track>> {
    ensure!(
        !query.trim().is_empty() && query.chars().count() <= 120,
        "请输入1到120个字符"
    );
    crate::mygo::ensure_netease()?;
    let result = crate::mygo::invoke("netease", "search", json!({"query":query,"kind":"tracks"}))?;
    crate::mygo::remember_artwork("netease", &result);
    Ok(result["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .take(limit.clamp(1, 20) as usize)
        .filter_map(|v| crate::mygo::catalog_track("netease", v))
        .collect())
}
pub fn resolve(id: u64) -> Result<Media> {
    resolve_with_quality(id, crate::audio_quality::preferred())
}
pub fn resolve_with_quality(id: u64, quality: crate::protocol::Quality) -> Result<Media> {
    ensure!(id > 0, "无效歌曲ID");
    crate::mygo::ensure_netease()?;
    crate::mygo::resolve_with_quality(id_track(id), quality)
}
fn id_track(id: u64) -> Track {
    if let Some(track) = crate::mygo::remembered_track("netease", &id.to_string()) {
        return track;
    }
    with_plugin_source(Track {
        id,
        title: id.to_string(),
        artist: String::new(),
        duration: 0.0,
        cover_url: None,
        album: None,
        source_url: None,
        source: None,
    })
}

pub fn lyrics(id: u64) -> Result<crate::protocol::Lyrics> {
    crate::mygo::ensure_netease()?;
    crate::mygo::lyrics(&id_track(id))
}
pub fn album(id: &str) -> Result<crate::protocol::AlbumPage> {
    crate::mygo::ensure_netease()?;
    let mut track = id_track(0);
    track.album = Some(crate::protocol::Album {
        id: id.into(),
        name: "专辑".into(),
    });
    crate::mygo::album(&track)
}
pub fn playlist_queue(id: u64) -> Result<crate::protocol::PlaylistPage> {
    crate::mygo::ensure_netease()?;
    let (origin, tracks) =
        crate::music_sources::playlist_snapshot("mygo:netease", &id.to_string())?;
    Ok(crate::protocol::PlaylistPage {
        id,
        name: origin.name,
        total: tracks.len(),
        more: false,
        tracks,
    })
}
pub fn playlist_tracks(id: u64, offset: u64, limit: u64) -> Result<Value> {
    let page = playlist_queue(id)?;
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    let limit = limit.clamp(1, 100) as usize;
    let total = page.tracks.len();
    Ok(
        json!({"id":id,"name":page.name,"tracks":page.tracks.into_iter().skip(start).take(limit).collect::<Vec<_>>(),"total":total,"more":start.saturating_add(limit)<total}),
    )
}
pub fn my_playlists(offset: u64, limit: u64) -> Result<Value> {
    crate::mygo::ensure_netease()?;
    let rows = crate::music_sources::playlists("mygo:netease")?;
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    let limit = limit.clamp(1, 50) as usize;
    let more = start.saturating_add(limit) < rows.len();
    let rows: Vec<_>=rows.into_iter().skip(start).take(limit).map(|p|json!({"id":p.id.parse::<u64>().unwrap_or(0),"name":p.name,"track_count":p.track_count})).collect();
    Ok(json!({"playlists":rows,"more":more}))
}
pub fn logout() -> Result<()> {
    crate::mygo::ensure_netease()?;
    crate::music_sources::execute(crate::protocol::MusicSourceAction::SignOut {
        source: "mygo:netease".into(),
    })?;
    Ok(())
}
pub fn begin_login() -> Result<crate::protocol::LoginChallenge> {
    crate::mygo::ensure_netease()?;
    let job = crate::music_sources::begin_sign_in("mygo:netease", None)?.job;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let state = crate::music_sources::login_state(&job, false)?;
        if let (Some(url), Some(svg)) = (state.qr_url, state.svg) {
            return Ok(crate::protocol::LoginChallenge {
                key: job,
                device: String::new(),
                url,
                svg,
                expires_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 180,
            });
        }
        if state.status == "error" {
            bail!("{}", state.text);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = crate::music_sources::login_state(&job, true);
    bail!("网易云插件获取二维码超时")
}
pub fn poll_login(input: &crate::protocol::LoginPoll) -> Result<crate::protocol::LoginProgress> {
    let state = crate::music_sources::login_state(&input.key, false)?;
    Ok(crate::protocol::LoginProgress {
        status: match state.status.as_str() {
            "success" => "success",
            "error" | "cancelled" => "expired",
            _ if state.text.contains("已扫码") => "scanned",
            _ => "pending",
        }
        .into(),
    })
}
pub fn login() -> Result<()> {
    let challenge = begin_login()?;
    let code = qrcode::QrCode::new(challenge.url.as_bytes())?;
    println!(
        "{}",
        code.render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .build()
    );
    println!("请使用网易云App扫码确认");
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        let p = poll_login(&crate::protocol::LoginPoll {
            key: challenge.key.clone(),
            device: String::new(),
        })?;
        match p.status.as_str() {
            "success" => {
                println!("网易云插件已登录");
                return Ok(());
            }
            "expired" => break,
            _ => std::thread::sleep(Duration::from_secs(1)),
        }
    }
    let _ = crate::music_sources::login_state(&challenge.key, true);
    bail!("等待扫码超时")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id_playback_reuses_public_plugin_metadata_without_private_exports() {
        let id = 9_223_372_036_854_775_000_u64;
        crate::mygo::remember_artwork(
            "netease",
            &json!({"tracks":[{
                "id":id.to_string(),"name":"来自插件搜索的曲名","artists":[{"name":"歌手"}],"duration":42000
            }]}),
        );
        let track = id_track(id);
        assert_eq!(track.title, "来自插件搜索的曲名");
        assert_eq!(track.artist, "歌手");
        assert_eq!(track.duration, 42.0);
        assert_eq!(track.source.unwrap().id, id.to_string());
        assert!(crate::mygo::remembered_track("other-plugin", &id.to_string()).is_none());
    }
    #[test]
    fn unknown_id_has_an_explicit_identity_without_invented_metadata() {
        let track = id_track(9_223_372_036_854_774_999);
        assert_eq!(track.title, "9223372036854774999");
        assert!(track.artist.is_empty());
        assert_eq!(track.source.unwrap().plugin, "netease");
    }
}
