use super::{
    merge::{Replica, Snapshot, recover},
    snapshot,
};
use crate::protocol::{HistoryEntry, HistoryProgress, LocalPlaylist, Track};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
struct TestDir(PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "musicx-sync-test-{}",
            crate::storage::random_hex::<12>()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn map(value: serde_json::Value) -> Snapshot {
    serde_json::from_value(value).unwrap()
}
fn track(id: u64) -> Track {
    serde_json::from_value(
        json!({"id":id,"title":format!("Song {id}"),"artist":"Artist","duration":90}),
    )
    .unwrap()
}
fn playlist() -> LocalPlaylist {
    LocalPlaylist {
        id: "mix".into(),
        name: "Mix".into(),
        favorite: false,
        pinned: false,
        revision: 7,
        tracks: vec![track(1)],
        imported_from: None,
    }
}
fn capture(lists: &[LocalPlaylist]) -> Snapshot {
    // No global data directory or credentials are accessed by the merge tests.
    let directory = TestDir::new();
    snapshot::capture(
        lists,
        &BTreeMap::new(),
        &directory.0,
        &Replica::new(A.into()),
    )
    .unwrap()
}

#[test]
fn independent_and_conflicting_edits_converge_in_any_arrival_order() {
    let mut a = Replica::new(A.into());
    let mut b = Replica::new(B.into());
    a.capture(
        &Snapshot::new(),
        &map(json!({"s/language":"en","s/normalization.json":true})),
    )
    .unwrap();
    b.capture(
        &Snapshot::new(),
        &map(json!({"s/language":"zh-CN","s/audio-quality.json":"high"})),
    )
    .unwrap();
    let mut ab = a.clone();
    ab.merge(&b).unwrap();
    b.merge(&a).unwrap();
    assert_eq!(ab.records, b.records);
    assert_eq!(ab.snapshot()["s/language"], "zh-CN");
    assert_eq!(ab.snapshot()["s/normalization.json"], true);
    assert_eq!(ab.snapshot()["s/audio-quality.json"], "high");
    ab.merge(&a).unwrap();
    assert_eq!(ab.records, b.records);
}

#[test]
fn concurrent_track_additions_and_metadata_edits_are_retained() {
    let original = playlist();
    let baseline = capture(std::slice::from_ref(&original));
    let mut a = Replica::new(A.into());
    a.capture(&Snapshot::new(), &baseline).unwrap();
    let mut b = Replica::new(B.into());
    b.merge(&a).unwrap();
    let mut left = original.clone();
    left.name = "Renamed".into();
    left.tracks.push(track(2));
    let mut right = original.clone();
    right.pinned = true;
    right.tracks.push(track(3));
    a.capture(&baseline, &capture(&[left])).unwrap();
    b.capture(&baseline, &capture(&[right])).unwrap();
    a.merge(&b).unwrap();
    let lists = snapshot::playlists(&a.snapshot(), &[original]).unwrap();
    assert_eq!(lists[0].name, "Renamed");
    assert!(lists[0].pinned);
    let mut ids: Vec<_> = lists[0].tracks.iter().map(|t| t.id).collect();
    ids.sort();
    assert_eq!(ids, vec![1, 2, 3]);
    assert_eq!(lists[0].revision, 8);
}

#[test]
fn deleting_a_playlist_does_not_get_undone_by_an_offline_track_addition() {
    let original = playlist();
    let baseline = capture(std::slice::from_ref(&original));
    let mut a = Replica::new(A.into());
    a.capture(&Snapshot::new(), &baseline).unwrap();
    let mut b = Replica::new(B.into());
    b.merge(&a).unwrap();
    a.capture(&baseline, &Snapshot::new()).unwrap();
    let mut offline = original;
    offline.tracks.push(track(2));
    b.capture(&baseline, &capture(&[offline])).unwrap();
    b.merge(&a).unwrap();
    assert!(snapshot::playlists(&b.snapshot(), &[]).unwrap().is_empty());
    b.merge(&a).unwrap();
    assert!(b.records.values().any(|r| r.value.is_none()));
}

#[test]
fn removing_a_favorite_survives_a_stale_device_and_can_be_liked_again() {
    let mut list = playlist();
    list.id = "liked".into();
    list.favorite = true;
    let baseline = capture(std::slice::from_ref(&list));
    let mut a = Replica::new(A.into());
    a.capture(&Snapshot::new(), &baseline).unwrap();
    let stale = a.clone();
    list.tracks.clear();
    let removed = capture(std::slice::from_ref(&list));
    a.capture(&baseline, &removed).unwrap();
    a.merge(&stale).unwrap();
    assert!(
        snapshot::playlists(&a.snapshot(), &[]).unwrap()[0]
            .tracks
            .is_empty()
    );
    list.tracks.push(track(1));
    a.capture(&removed, &capture(&[list])).unwrap();
    assert_eq!(
        snapshot::playlists(&a.snapshot(), &[]).unwrap()[0]
            .tracks
            .len(),
        1
    );
}

#[test]
fn repeated_plays_keep_separate_progress_and_devices() {
    let entry = HistoryEntry {
        track: track(1),
        played_at_unix: 123,
        progress: Some(HistoryProgress {
            position: 2.0,
            duration: 90.0,
            completed: false,
        }),
    };
    let mut history = BTreeMap::from([
        (format!("{A}/0"), entry.clone()),
        (format!("{A}/256"), entry.clone()),
        (format!("{B}/0"), entry),
    ]);
    history
        .get_mut(&format!("{A}/256"))
        .unwrap()
        .progress
        .as_mut()
        .unwrap()
        .position = 60.0;
    let s: Snapshot = history
        .iter()
        .map(|(id, entry)| (format!("h/{id}"), serde_json::to_value(entry).unwrap()))
        .collect();
    let rows = snapshot::history(&s).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[&format!("{A}/0")].progress.unwrap().position, 2.0);
    assert_eq!(rows[&format!("{A}/256")].progress.unwrap().position, 60.0);
    assert_eq!(snapshot::local_view(&s, A).len(), 2);
}

#[test]
fn interrupted_apply_replays_remote_changes_and_preserves_new_local_edits() {
    let before = map(json!({"s/language":"en","s/normalization.json":false}));
    let after = map(
        json!({"s/language":"zh-CN","s/normalization.json":false,"s/panel-preferences.json":false}),
    );
    let edited = map(json!({"s/language":"en","s/normalization.json":true}));
    let recovered = recover(&before, &after, &edited);
    assert_eq!(recovered["s/language"], "zh-CN");
    assert_eq!(recovered["s/normalization.json"], true);
    assert_eq!(recovered["s/panel-preferences.json"], false);
    assert_eq!(recover(&before, &after, &after), after);
}

#[test]
fn untrusted_preferences_and_invalid_revisions_are_rejected() {
    assert!(snapshot::validate_settings(&map(json!({"s/mcp-token":"secret"}))).is_err());
    assert!(snapshot::validate_settings(&map(json!({"s/language":"invalid"}))).is_err());
    let mut replica = Replica::new(A.into());
    replica.schema = 2;
    assert!(replica.validate().is_err());
    replica.schema = 1;
    replica
        .capture(&Snapshot::new(), &map(json!({"s/language":"en"})))
        .unwrap();
    let mut damaged = replica.clone();
    damaged.records.get_mut("s/language").unwrap().value = Some(json!("zh-CN"));
    assert!(replica.merge(&damaged).is_err());
}

#[test]
fn radio_identity_and_user_playlist_names_survive_sync() {
    let mut radio = playlist();
    radio.id = crate::protocol::RADIO_PLAYLIST_ID.into();
    radio.name = "Radio".into();
    let mut liked = playlist();
    liked.id = "liked".into();
    liked.name = "我喜欢".into();
    liked.favorite = true;
    let mut custom = playlist();
    custom.name = "我喜欢".into();
    let lists = snapshot::playlists(&capture(&[radio, liked, custom]), &[]).unwrap();
    assert_eq!(lists.len(), 3);
    assert!(
        lists
            .iter()
            .any(|p| p.id == crate::protocol::RADIO_PLAYLIST_ID && p.name == "Radio")
    );
    assert!(
        lists
            .iter()
            .any(|p| p.id == "mix" && !p.favorite && p.name == "我喜欢")
    );
}

#[test]
fn local_artwork_is_not_exported_in_playlists_or_history() {
    let mut list = playlist();
    list.tracks[0].cover_url =
        Some("file:///Users/device-a/Library/MusicX/plugin/cover.jpg".into());
    let entry = HistoryEntry {
        track: list.tracks[0].clone(),
        played_at_unix: 123,
        progress: None,
    };
    let history = BTreeMap::from([(format!("{A}/0"), entry)]);
    let directory = TestDir::new();
    let snapshot =
        snapshot::capture(&[list], &history, &directory.0, &Replica::new(A.into())).unwrap();
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains("file:///Users/device-a"),
        "Device-local artwork must be replenished by the receiving source"
    );
}

#[test]
fn artwork_restoration_matches_identity_and_preserves_remote_covers() {
    let directory = TestDir::new();
    let artwork = directory.0.join("cover.jpg");
    fs::write(&artwork, b"artwork").unwrap();
    let local_cover = url::Url::from_file_path(&artwork).unwrap().to_string();
    let missing_cover = url::Url::from_file_path(directory.0.join("missing.jpg"))
        .unwrap()
        .to_string();
    let mut previous = playlist();
    previous.tracks = (1..=5).map(track).collect();
    for track in &mut previous.tracks[..4] {
        track.cover_url = Some(local_cover.clone());
    }
    previous.tracks[2].cover_url = Some(missing_cover);
    previous.tracks[4].cover_url = Some("https://old.example/cover.jpg".into());
    let mut incoming = previous.clone();
    incoming.tracks = [2, 1, 3, 4, 5].into_iter().map(track).collect();
    incoming.tracks[3].cover_url = Some("https://remote.example/cover.jpg".into());

    let restored = snapshot::playlists(&capture(&[incoming]), &[previous.clone()]).unwrap();
    let covers: Vec<_> = restored[0]
        .tracks
        .iter()
        .map(|track| track.cover_url.as_deref())
        .collect();
    assert_eq!(
        covers,
        [
            Some(local_cover.as_str()),
            Some(local_cover.as_str()),
            None,
            Some("https://remote.example/cover.jpg"),
            None,
        ]
    );
    previous.tracks[2].cover_url = None;
    let unchanged =
        snapshot::playlists(&capture(&[previous.clone()]), &[previous.clone()]).unwrap();
    assert!(unchanged == [previous]);
}
