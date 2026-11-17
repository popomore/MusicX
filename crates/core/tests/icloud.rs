//! Exercise production persistence in two isolated device processes.
use musicx_core::{
    audio_quality,
    cloud_sync::{self, merge::Replica},
    history, local_library,
    protocol::{self, HistoryProgress, LocalLibraryAction as Edit, Track},
    storage,
};
use serde_json::json;
use std::{fs, path::Path, process::Command};

fn track(id: u64) -> Track {
    serde_json::from_value(
        json!({"id":id,"title":format!("Song {id}"),"artist":"Artist","duration":90}),
    )
    .unwrap()
}
fn progress(position: f64) -> HistoryProgress {
    HistoryProgress {
        position,
        duration: 90.0,
        completed: false,
    }
}
fn incoming(root: &Path, device: &str) -> Replica {
    serde_json::from_slice(&fs::read(root.join(format!("{device}.json"))).unwrap()).unwrap()
}
fn export(root: &Path, device: &str, replica: Replica) {
    fs::write(
        root.join(format!("{device}.json")),
        serde_json::to_vec(&replica).unwrap(),
    )
    .unwrap();
}
fn child(root: &Path, device: &str, step: &str) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "icloud_child", "--nocapture"])
        .env("MUSICX_DATA_DIR", root.join(device))
        .env("MUSICX_ICLOUD_TEST_ROOT", root)
        .env("MUSICX_ICLOUD_TEST_STEP", step)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn two_devices_merge_offline_changes_and_history_without_syncing_secrets() {
    let root = std::env::temp_dir().join(format!("musicx-icloud-{}", storage::random_hex::<12>()));
    fs::create_dir_all(&root).unwrap();
    child(&root, "a", "seed");
    child(&root, "b", "receive");
    child(&root, "a", "offline");
    child(&root, "b", "converge");
    child(&root, "a", "recover");
    child(&root, "a", "corrupt");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn icloud_child() {
    let Ok(step) = std::env::var("MUSICX_ICLOUD_TEST_STEP") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_ICLOUD_TEST_ROOT").unwrap());
    match step.as_str() {
        "seed" => {
            assert!(!cloud_sync::enabled());
            assert!(cloud_sync::synchronize(&[]).unwrap().is_none());
            storage::save_private("mcp-token", "fixture-secret-do-not-sync").unwrap();
            storage::save_private(
                "plugin-updates.json",
                r#"{"enabled":false,"checks":{"device-local-record":"do-not-sync-checks"}}"#,
            )
            .unwrap();
            storage::save_private("mygo-source-storage.json", "fixture-account-do-not-sync")
                .unwrap();
            local_library::change(Edit::SetFavorite {
                track: Box::new(track(1)),
                liked: true,
            })
            .unwrap();
            let list = local_library::change(Edit::Create { name: "Mix".into() }).unwrap();
            let id = list["id"].as_str().unwrap().to_owned();
            fs::write(root.join("playlist-id"), &id).unwrap();
            local_library::change(Edit::Add {
                id,
                tracks: vec![track(1)],
            })
            .unwrap();
            history::record(track(1), progress(2.0)).unwrap();
            history::record(track(1), progress(27.0)).unwrap();
            protocol::i18n::Language::English.save().unwrap();
            cloud_sync::set_enabled(true).unwrap();
            assert!(cloud_sync::bind_account("account-one").unwrap());
            let replica = cloud_sync::synchronize(&[]).unwrap().unwrap();
            let text = serde_json::to_string(&replica).unwrap();
            assert!(
                !text.contains("fixture-secret")
                    && !text.contains("fixture-account")
                    && !text.contains("mcp-token")
                    && !text.contains("do-not-sync-checks")
            );
            export(&root, "stale", replica.clone());
            export(&root, "a", replica);
        }
        "receive" => {
            local_library::change(Edit::SetFavorite {
                track: Box::new(track(2)),
                liked: true,
            })
            .unwrap();
            cloud_sync::set_enabled(true).unwrap();
            assert!(cloud_sync::bind_account("account-one").unwrap());
            cloud_sync::synchronize(&[incoming(&root, "a")]).unwrap();
            assert_eq!(local_library::playlist("liked").unwrap().tracks.len(), 2);
            assert_eq!(history::recent(100).unwrap().len(), 2);
            assert_eq!(
                protocol::i18n::Language::load(),
                protocol::i18n::Language::English
            );
            assert!(!storage::state_dir().unwrap().join("mcp-token").exists());
            assert_eq!(
                fs::read_to_string(
                    storage::state_dir()
                        .unwrap()
                        .join("plugin-auto-update.json")
                )
                .unwrap(),
                "false"
            );
            let id = fs::read_to_string(root.join("playlist-id")).unwrap();
            local_library::change(Edit::Rename {
                id: id.clone(),
                name: "From B".into(),
            })
            .unwrap();
            local_library::change(Edit::Add {
                id,
                tracks: vec![track(2)],
            })
            .unwrap();
            local_library::change(Edit::SetFavorite {
                track: Box::new(track(1)),
                liked: false,
            })
            .unwrap();
            audio_quality::set_normalization(true).unwrap();
            history::record(track(4), progress(9.0)).unwrap();
            export(&root, "b", cloud_sync::synchronize(&[]).unwrap().unwrap());
        }
        "offline" => {
            let id = fs::read_to_string(root.join("playlist-id")).unwrap();
            local_library::change(Edit::SetPinned {
                id: id.clone(),
                pinned: true,
            })
            .unwrap();
            local_library::change(Edit::Add {
                id,
                tracks: vec![track(3)],
            })
            .unwrap();
            local_library::change(Edit::SetFavorite {
                track: Box::new(track(5)),
                liked: true,
            })
            .unwrap();
            assert!(history::restore(&track(1)).unwrap());
            history::update(progress(40.0), true).unwrap();
            let replica = cloud_sync::synchronize(&[incoming(&root, "b")])
                .unwrap()
                .unwrap();
            assert!(audio_quality::normalization_enabled());
            assert!(!cloud_sync::bind_account("account-two").unwrap());
            assert_eq!(
                cloud_sync::status().phase,
                protocol::CloudSyncPhase::AccountChanged
            );
            export(&root, "a", replica);
        }
        "converge" => {
            cloud_sync::synchronize(&[incoming(&root, "a"), incoming(&root, "stale")]).unwrap();
            let id = fs::read_to_string(root.join("playlist-id")).unwrap();
            let list = local_library::playlist(&id).unwrap();
            assert!(list.pinned);
            assert_eq!(list.name, "From B");
            let mut ids: Vec<_> = list.tracks.iter().map(|t| t.id).collect();
            ids.sort();
            assert_eq!(ids, vec![1, 2, 3]);
            let mut liked: Vec<_> = local_library::playlist("liked")
                .unwrap()
                .tracks
                .iter()
                .map(|t| t.id)
                .collect();
            liked.sort();
            assert_eq!(liked, vec![2, 5]);
            let rows = history::recent(100).unwrap();
            assert_eq!(rows.len(), 3);
            let mut positions: Vec<_> = rows
                .iter()
                .filter(|e| e.track.id == 1)
                .map(|e| e.progress.unwrap().position as u64)
                .collect();
            positions.sort();
            assert_eq!(positions, vec![2, 40]);
            let revision = local_library::revision();
            cloud_sync::synchronize(&[incoming(&root, "a")]).unwrap();
            assert_eq!(
                local_library::revision(),
                revision,
                "No-op sync must not invalidate playlists"
            );
            cloud_sync::set_enabled(false).unwrap();
            assert!(
                cloud_sync::synchronize(&[incoming(&root, "stale")])
                    .unwrap()
                    .is_none()
            );
            assert_eq!(history::recent(100).unwrap().len(), 3);
        }
        "recover" => {
            use musicx_core::cloud_sync::merge::Snapshot;
            let dir = storage::state_dir().unwrap();
            let mut state: serde_json::Value =
                serde_json::from_slice(&fs::read(dir.join("icloud-state.json")).unwrap()).unwrap();
            let before: Snapshot = serde_json::from_value(state["baseline"].clone()).unwrap();
            let mut after = before.clone();
            let key = before
                .iter()
                .find(|(key, value)| key.ends_with("/name") && **value == json!("From B"))
                .unwrap()
                .0
                .clone();
            after.insert(key, json!("Recovered remote name"));
            let mut replica: Replica = serde_json::from_value(state["replica"].clone()).unwrap();
            let mut remote = Replica::new("cccccccccccccccccccccccccccccccc".into());
            remote.merge(&replica).unwrap();
            remote.capture(&before, &after).unwrap();
            replica.merge(&remote).unwrap();
            state["replica"] = serde_json::to_value(replica).unwrap();
            state["pending"] = json!({"before":before,"after":after});
            storage::save_private("icloud-state.json", &serde_json::to_string(&state).unwrap())
                .unwrap();
            // Simulate a user edit made after a process stopped before applying.
            audio_quality::set_preferred(protocol::Quality::High).unwrap();
            local_library::change(Edit::SetFavorite {
                track: Box::new(track(7)),
                liked: true,
            })
            .unwrap();
            cloud_sync::synchronize(&[]).unwrap();
            let id = fs::read_to_string(root.join("playlist-id")).unwrap();
            assert_eq!(
                local_library::playlist(&id).unwrap().name,
                "Recovered remote name"
            );
            assert!(
                local_library::playlist("liked")
                    .unwrap()
                    .tracks
                    .iter()
                    .any(|t| t.id == 7)
            );
            assert_eq!(audio_quality::preferred(), protocol::Quality::High);
            let state: serde_json::Value =
                serde_json::from_slice(&fs::read(dir.join("icloud-state.json")).unwrap()).unwrap();
            assert!(state["pending"].is_null());
        }
        "corrupt" => {
            let dir = storage::state_dir().unwrap();
            let before = fs::read(dir.join("local-playlists.json")).unwrap();
            storage::save_private("icloud-state.json", "invalid json").unwrap();
            assert!(cloud_sync::synchronize(&[]).is_err());
            assert_eq!(before, fs::read(dir.join("local-playlists.json")).unwrap());
            assert_eq!(
                fs::read_to_string(dir.join("icloud-state.json")).unwrap(),
                "invalid json"
            );
            storage::save_private(cloud_sync::HISTORY_FILE, "invalid history cache").unwrap();
            assert_eq!(
                history::recent(100).unwrap().len(),
                2,
                "Corrupt received history must not hide the local log"
            );
        }
        _ => panic!("Unknown test step"),
    }
}
