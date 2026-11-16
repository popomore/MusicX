//! Local snapshots must work with a source that has no MyGo runtime.
use anyhow::Result;
use musicx_core::{history, local_library, protocol::*, sources};
use musicx_plugin::{DataSource, Media, PluginSource};
use std::{
    fs,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Independent {
    calls: Mutex<Vec<String>>,
    fail: AtomicBool,
}
fn track() -> Track {
    serde_json::from_value(serde_json::json!({"id":77,"title":"Independent song","artist":"Artist","duration":42,"source":{"plugin":"independent","id":"opaque:track/one"}})).unwrap()
}
impl DataSource for Independent {
    fn resolve(&self, _: Track, _: Quality) -> Result<Media> {
        panic!("import must not resolve audio")
    }
    fn info(&self, _: Track) -> TrackInfo {
        panic!("import must not fetch track info")
    }
    fn album(&self, _: Track) -> Result<AlbumPage> {
        panic!("import must not fetch albums")
    }
}
impl PluginSource for Independent {
    fn playlist(&self, source: &str, id: &str) -> Result<SourcePlaylistSnapshot> {
        assert_eq!(source, "independent:fixture");
        assert_eq!(id, "opaque:list/one");
        self.calls.lock().unwrap().push("playlist".into());
        anyhow::ensure!(
            !self.fail.load(Ordering::Relaxed),
            "fixture snapshot unavailable"
        );
        Ok(snapshot(source, id))
    }
    fn personal_collection(
        &self,
        source: &str,
        collection: SourcePersonalCollection,
    ) -> Result<SourcePlaylistSnapshot> {
        assert_eq!(source, "independent:fixture");
        let id = match collection {
            SourcePersonalCollection::LikedTracks => "likes",
            SourcePersonalCollection::RecentTracks => "recent",
        };
        self.calls.lock().unwrap().push(id.into());
        Ok(snapshot(source, id))
    }
}
fn snapshot(source: &str, id: &str) -> SourcePlaylistSnapshot {
    SourcePlaylistSnapshot {
        origin: PlaylistOrigin {
            provider: source.into(),
            id: id.into(),
            name: "Independent collection".into(),
        },
        tracks: vec![track()],
    }
}

#[test]
fn imports_use_the_injected_source_and_preserve_local_history() {
    if std::env::var_os("MUSICX_IMPORT_CONTRACT_CHILD").is_none() {
        let root =
            std::env::temp_dir().join(format!("musicx-contract-import-{}", std::process::id()));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "imports_use_the_injected_source_and_preserve_local_history",
                "--nocapture",
            ])
            .env("MUSICX_IMPORT_CONTRACT_CHILD", "1")
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
    let provider = Independent {
        calls: Mutex::new(Vec::new()),
        fail: AtomicBool::new(false),
    };
    for action in [
        MusicSourceAction::ImportPlaylist {
            source: "independent:fixture".into(),
            playlist_id: "opaque:list/one".into(),
        },
        MusicSourceAction::ImportLiked {
            source: "independent:fixture".into(),
        },
        MusicSourceAction::ImportRecent {
            source: "independent:fixture".into(),
        },
    ] {
        let imported = sources::execute(&provider, action).unwrap();
        assert_eq!(imported["tracks"][0]["source"]["id"], "opaque:track/one");
        assert_eq!(imported["tracks"][0]["duration"], 42.0);
    }
    let before = local_library::lists().unwrap().len();
    assert_eq!(before, 3);
    provider.fail.store(true, Ordering::Relaxed);
    assert!(
        sources::change_library(
            &provider,
            LocalLibraryAction::ImportSource {
                source: "independent:fixture".into(),
                playlist_id: "opaque:list/one".into()
            }
        )
        .is_err()
    );
    assert_eq!(local_library::lists().unwrap().len(), before);
    assert!(
        history::recent(100).unwrap().is_empty(),
        "imported history is a snapshot, not local playback events"
    );
    assert_eq!(
        *provider.calls.lock().unwrap(),
        ["playlist", "likes", "recent", "playlist"]
    );
}
