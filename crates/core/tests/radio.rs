//! Radio must work with an injected provider without resolving or starting audio.
use anyhow::Result;
use musicx_core::{local_library, protocol::*, sources};
use musicx_plugin::{DataSource, Media, PluginSource};
use std::{
    fs,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

struct Accounts {
    mode: AtomicU8,
    calls: Mutex<Vec<String>>,
}
fn track(plugin: &str, id: &str) -> Track {
    Track {
        id: 1,
        title: format!("{plugin} {id}"),
        artist: "Artist".into(),
        duration: 180.0,
        source: Some(Source {
            plugin: plugin.into(),
            id: id.into(),
        }),
        artists: Vec::new(),
        episode: None,
        cover_url: None,
        album: None,
        source_url: None,
    }
}
impl DataSource for Accounts {
    fn resolve(&self, _: Track, _: Quality) -> Result<Media> {
        panic!("Radio must not resolve or play audio")
    }
    fn info(&self, _: Track) -> TrackInfo {
        panic!("Radio must not fetch unrelated metadata")
    }
    fn album(&self, _: Track) -> Result<AlbumPage> {
        panic!("Radio must not browse albums")
    }
}
impl PluginSource for Accounts {
    fn sources(&self) -> Result<Vec<MusicSource>> {
        let mut sources: Vec<_> = [
            "a",
            "b",
            "signed-out",
            "broken",
            "invalid",
            "catalog",
            "podcast",
        ]
        .into_iter()
        .map(|id| MusicSource {
            id: id.into(),
            name: id.into(),
            installed: id != "catalog",
            host_capabilities: if id == "podcast" {
                vec![SourceCapability::Account, SourceCapability::Podcasts]
            } else {
                vec![
                    SourceCapability::Account,
                    SourceCapability::Playback,
                    SourceCapability::Recommendations,
                ]
            },
            ..Default::default()
        })
        .collect();
        sources.push(MusicSource {
            id: "remote".into(),
            installed: true,
            playback: "remote".into(),
            host_capabilities: vec![SourceCapability::Account, SourceCapability::Recommendations],
            ..Default::default()
        });
        Ok(sources)
    }
    fn account(&self, source: &str) -> Result<SourceAccount> {
        assert!(!matches!(source, "catalog" | "podcast" | "remote"));
        Ok(SourceAccount {
            connected: source != "signed-out",
            ..Default::default()
        })
    }
    fn radio_tracks(&self, source: &str) -> Result<Vec<Track>> {
        self.calls.lock().unwrap().push(source.into());
        match source {
            "a" if self.mode.load(Ordering::Relaxed) == 0 => {
                Ok(vec![track("a", "1"), track("a", "1"), track("a", "2")])
            }
            "b" if self.mode.load(Ordering::Relaxed) == 0 => Ok(vec![track("b", "1")]),
            "a" | "b" if self.mode.load(Ordering::Relaxed) == 1 => Ok(Vec::new()),
            "a" | "b" => Ok(vec![track(source, "new")]),
            "broken" => anyhow::bail!("fixture provider unavailable"),
            "invalid" => Ok(vec![Track {
                title: String::new(),
                ..track("invalid", "bad")
            }]),
            _ => panic!("must not fetch recommendations from an ineligible source"),
        }
    }
}
fn refresh(provider: &Accounts) -> Result<RadioRefresh> {
    Ok(serde_json::from_value(sources::change_library(
        provider,
        LocalLibraryAction::RefreshRadio {},
    )?)?)
}

#[test]
fn radio_combines_connected_accounts_and_appends_an_atomic_deduplicated_playlist() {
    if std::env::var_os("MUSICX_RADIO_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!("musicx-radio-core-{}", std::process::id()));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "radio_combines_connected_accounts_and_appends_an_atomic_deduplicated_playlist",
                "--nocapture",
            ])
            .env("MUSICX_RADIO_CHILD", "1")
            .env("MUSICX_DATA_DIR", &root)
            .output()
            .unwrap();
        if root.exists() {
            fs::remove_dir_all(root).unwrap();
        }
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let provider = Accounts {
        mode: AtomicU8::new(0),
        calls: Mutex::new(Vec::new()),
    };
    let response = refresh(&provider).unwrap();
    assert_eq!(response.added, 3);
    let playlist = response.playlist.unwrap();
    assert_eq!(playlist.id, RADIO_PLAYLIST_ID);
    assert_eq!(
        playlist.tracks.iter().map(track_key).collect::<Vec<_>>(),
        ["mygo:a:1", "mygo:b:1", "mygo:a:2"]
    );
    assert_eq!(response.sources.len(), 5);
    assert!(
        response
            .sources
            .iter()
            .find(|s| s.source == "signed-out")
            .unwrap()
            .error
            .is_some()
    );
    assert!(
        response
            .sources
            .iter()
            .find(|s| s.source == "broken")
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("fixture provider unavailable")
    );
    assert!(
        response
            .sources
            .iter()
            .find(|s| s.source == "invalid")
            .unwrap()
            .error
            .is_some()
    );
    assert_eq!(provider.calls.lock().unwrap().len(), 4);
    let repeated = refresh(&provider).unwrap();
    assert_eq!(repeated.added, 0);
    assert_eq!(repeated.playlist.unwrap().revision, playlist.revision);
    assert_eq!(local_library::lists().unwrap().len(), 1);

    local_library::change(LocalLibraryAction::Rename {
        id: RADIO_PLAYLIST_ID.into(),
        name: "My discoveries".into(),
    })
    .unwrap();
    local_library::change(LocalLibraryAction::SetPinned {
        id: RADIO_PLAYLIST_ID.into(),
        pinned: true,
    })
    .unwrap();
    let path = data_dir().unwrap().join("local-playlists.json");
    let before = fs::read(&path).unwrap();
    provider.mode.store(1, Ordering::Relaxed);
    let empty = refresh(&provider).unwrap();
    assert_eq!(empty.added, 0);
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "empty and failing providers must not write or clear the library"
    );
    provider.mode.store(2, Ordering::Relaxed);
    let more = refresh(&provider).unwrap();
    assert_eq!(more.added, 2);
    let playlist = more.playlist.unwrap();
    assert_eq!(playlist.name, "My discoveries");
    assert!(playlist.pinned);
    assert_eq!(playlist.tracks.len(), 5);

    local_library::change(LocalLibraryAction::Delete {
        id: RADIO_PLAYLIST_ID.into(),
    })
    .unwrap();
    provider.mode.store(1, Ordering::Relaxed);
    assert!(refresh(&provider).unwrap().playlist.is_none());
    assert!(
        local_library::lists().unwrap().is_empty(),
        "empty results must not create a playlist"
    );
    provider.mode.store(2, Ordering::Relaxed);
    assert_eq!(refresh(&provider).unwrap().added, 2);
    let fill: Vec<_> = (0..9_998)
        .map(|id| track("fill", &id.to_string()))
        .collect();
    local_library::change(LocalLibraryAction::Add {
        id: RADIO_PLAYLIST_ID.into(),
        tracks: fill,
    })
    .unwrap();
    let before = fs::read(&path).unwrap();
    provider.mode.store(0, Ordering::Relaxed);
    assert!(refresh(&provider).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "a full playlist must remain intact on a failed append"
    );
}
