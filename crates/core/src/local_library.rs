//! On-demand local playlists. No background sync, audio cache or resident library copy.
use crate::{metadata, protocol::*, storage};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
static REVISION: AtomicU64 = AtomicU64::new(0);
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

static WRITER: Mutex<()> = Mutex::new(());
const MAX_LISTS: usize = 200;
const MAX_TRACKS: usize = 10000;
const MAX_TOTAL: usize = 50000;
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct Library {
    #[serde(default)]
    playlists: Vec<LocalPlaylist>,
}

fn read() -> Result<Library> {
    let path = storage::state_dir()?.join("local-playlists.json");
    match fs::File::open(&path) {
        Ok(file) => {
            ensure!(file.metadata()?.len() <= MAX_BYTES, "本地歌单文件过大");
            serde_json::from_reader(std::io::BufReader::new(file))
                .context("本地歌单文件无法读取，原文件保持不变")
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Library::default()),
        Err(e) => Err(e.into()),
    }
}

fn save(library: &Library) -> Result<()> {
    ensure!(
        library.playlists.len() <= MAX_LISTS,
        "最多创建200个本地歌单"
    );
    ensure!(
        library
            .playlists
            .iter()
            .map(|p| p.tracks.len())
            .sum::<usize>()
            <= MAX_TOTAL,
        "本地歌单曲目总数超过50000首"
    );
    let value = serde_json::to_string(library)?;
    ensure!(value.len() as u64 <= MAX_BYTES, "本地歌单文件过大");
    storage::save_private("local-playlists.json", &value)?;
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

fn name(value: String) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.is_empty() && value.chars().count() <= 80,
        "歌单名称需为1至80个字"
    );
    Ok(value.to_owned())
}

fn identity(track: &Track) -> String {
    crate::protocol::track_key(track)
}

fn append(list: &mut LocalPlaylist, tracks: Vec<Track>) -> Result<()> {
    ensure!(tracks.len() <= MAX_TRACKS, "一次最多添加10000首歌");
    let mut keys: std::collections::HashSet<_> = list.tracks.iter().map(identity).collect();
    for track in tracks {
        ensure!(
            !track.title.trim().is_empty()
                && track.title.len() <= 2048
                && track.artist.len() <= 4096
                && track.duration.is_finite()
                && track.duration >= 0.0,
            "曲目信息不完整"
        );
        ensure!(
            track
                .source
                .as_ref()
                .map_or(track.id > 0, |s| !s.plugin.is_empty() && !s.id.is_empty()),
            "歌曲缺少来源标识"
        );
        if keys.insert(identity(&track)) {
            list.tracks.push(track);
        }
    }
    ensure!(list.tracks.len() <= MAX_TRACKS, "每个歌单最多10000首歌");
    Ok(())
}

impl Library {
    fn favorite_index(&self) -> Option<usize> {
        self.playlists
            .iter()
            .position(|p| p.favorite || p.id == "liked")
            .or_else(|| self.playlists.iter().position(|p| p.name == "我喜欢"))
    }
    fn favorite_state(&self, key: &str) -> FavoriteState {
        let list = self.favorite_index().map(|i| &self.playlists[i]);
        FavoriteState {
            key: key.into(),
            liked: list.is_some_and(|p| p.tracks.iter().any(|t| identity(t) == key)),
            playlist_id: list.map(|p| p.id.clone()),
        }
    }
    fn set_favorite(&mut self, track: Track, liked: bool) -> Result<Value> {
        let key = identity(&track);
        let index = self.favorite_index();
        if !liked && index.is_none() {
            return Ok(serde_json::to_value(self.favorite_state(&key))?);
        }
        let mut list = index
            .map(|i| self.playlists[i].clone())
            .unwrap_or_else(|| LocalPlaylist {
                id: "liked".into(),
                name: "我喜欢".into(),
                favorite: true,
                pinned: false,
                revision: 0,
                tracks: Vec::new(),
                imported_from: None,
            });
        list.favorite = true;
        list.pinned = false;
        if liked {
            append(&mut list, vec![track])?;
        } else {
            list.tracks.retain(|t| identity(t) != key);
        }
        list.revision = list.revision.wrapping_add(1);
        if let Some(i) = index {
            self.playlists[i] = list;
        } else {
            self.playlists.push(list);
        }
        Ok(serde_json::to_value(self.favorite_state(&key))?)
    }

    fn get(&self, id: &str) -> Result<&LocalPlaylist> {
        self.playlists
            .iter()
            .find(|p| p.id == id)
            .context("本地歌单不存在，请刷新")
    }
    fn edit(&mut self, action: LocalLibraryAction) -> Result<Value> {
        if let LocalLibraryAction::SetFavorite { track, liked } = action {
            return self.set_favorite(*track, liked);
        }
        if let LocalLibraryAction::Create { name: title } = action {
            let list = LocalPlaylist {
                favorite: false,
                pinned: false,
                id: storage::random_hex::<12>(),
                name: name(title)?,
                revision: 1,
                tracks: Vec::new(),
                imported_from: None,
            };
            self.playlists.push(list.clone());
            return Ok(serde_json::to_value(list)?);
        }
        let id = match &action {
            LocalLibraryAction::Rename { id, .. }
            | LocalLibraryAction::SetPinned { id, .. }
            | LocalLibraryAction::Add { id, .. }
            | LocalLibraryAction::Remove { id, .. }
            | LocalLibraryAction::Delete { id } => id,
            _ => anyhow::bail!("无效的本地歌单操作"),
        };
        let index = self
            .playlists
            .iter()
            .position(|p| &p.id == id)
            .context("本地歌单不存在，请刷新")?;
        if self.playlists[index].favorite
            && matches!(
                action,
                LocalLibraryAction::Delete { .. } | LocalLibraryAction::Rename { .. }
            )
        {
            anyhow::bail!("我喜欢是收藏歌单，可移除歌曲，不能重命名或删除");
        }
        if matches!(action, LocalLibraryAction::Delete { .. }) {
            self.playlists.remove(index);
            return Ok(json!({"ok":true}));
        }
        let list = &mut self.playlists[index];
        match action {
            LocalLibraryAction::Rename { name: title, .. } => list.name = name(title)?,
            LocalLibraryAction::SetPinned { pinned, .. } => {
                ensure!(!list.favorite, "我喜欢已固定在首位");
                list.pinned = pinned;
            }
            LocalLibraryAction::Add { tracks, .. } => append(list, tracks)?,
            LocalLibraryAction::Remove {
                index, revision, ..
            } => {
                ensure!(list.revision == revision, "歌单已更新，请刷新后再移除");
                ensure!(index < list.tracks.len(), "无效的歌曲位置");
                list.tracks.remove(index);
            }
            _ => unreachable!(),
        }
        list.revision = list.revision.wrapping_add(1);
        Ok(serde_json::to_value(list)?)
    }
}

pub fn lists() -> Result<Vec<LocalPlaylistSummary>> {
    let mut lists = read()?.playlists;
    lists.sort_by_key(|p| (!p.favorite, !p.pinned));
    Ok(lists
        .into_iter()
        .map(|p| LocalPlaylistSummary {
            cover_url: p
                .tracks
                .iter()
                .find_map(|t| t.cover_url.as_deref().and_then(metadata::artwork_url)),
            favorite: p.favorite,
            pinned: p.pinned,
            id: p.id,
            name: p.name,
            revision: p.revision,
            track_count: p.tracks.len(),
            imported_from: p.imported_from,
        })
        .collect())
}
pub fn playlist(id: &str) -> Result<LocalPlaylist> {
    Ok(read()?.get(id)?.clone())
}

fn update(edit: impl FnOnce(&mut Library) -> Result<Value>) -> Result<Value> {
    let _guard = WRITER
        .lock()
        .map_err(|_| anyhow::anyhow!("本地歌单暂时不可用"))?;
    let mut library = read()?;
    let result = edit(&mut library)?;
    save(&library)?;
    Ok(result)
}

pub fn change(action: LocalLibraryAction) -> Result<Value> {
    update(|library| library.edit(action))
}

pub fn import(origin: PlaylistOrigin, tracks: Vec<Track>) -> Result<Value> {
    update(|library| {
        let mut list = LocalPlaylist {
            favorite: false,
            pinned: false,
            id: storage::random_hex::<12>(),
            name: name(origin.name.clone())?,
            revision: 1,
            tracks: Vec::new(),
            imported_from: Some(origin),
        };
        append(&mut list, tracks)?;
        let result = serde_json::to_value(&list)?;
        library.playlists.push(list);
        Ok(result)
    })
}

pub fn favorite(key: &str) -> Result<FavoriteState> {
    ensure!(!key.is_empty() && key.len() <= 2048, "无效的歌曲标识");
    Ok(read()?.favorite_state(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(id: u64) -> Track {
        serde_json::from_value(
            json!({"id":id,"title":format!("song{id}"),"artist":"artist","duration":10.0}),
        )
        .unwrap()
    }
    fn list() -> LocalPlaylist {
        LocalPlaylist {
            favorite: false,
            pinned: false,
            id: "local".into(),
            name: "Mine".into(),
            revision: 1,
            tracks: vec![track(1), track(2)],
            imported_from: None,
        }
    }
    #[test]
    fn add_deduplicates_by_provider_identity_preserving_order() {
        let mut p = list();
        let mut plugin = track(1);
        plugin.source = Some(Source {
            plugin: "test".into(),
            id: "1".into(),
        });
        append(&mut p, vec![track(2), track(3), plugin]).unwrap();
        assert_eq!(
            p.tracks.iter().map(identity).collect::<Vec<_>>(),
            ["netease:1", "netease:2", "netease:3", "mygo:test:1"]
        );
    }
    #[test]
    fn stale_remove_cannot_delete_a_different_song() {
        let mut lib = Library {
            playlists: vec![list()],
        };
        assert!(
            lib.edit(LocalLibraryAction::Remove {
                id: "local".into(),
                index: 1,
                revision: 0
            })
            .is_err()
        );
        assert_eq!(lib.playlists[0].tracks.len(), 2);
        lib.edit(LocalLibraryAction::Remove {
            id: "local".into(),
            index: 1,
            revision: 1,
        })
        .unwrap();
        assert_eq!(lib.playlists[0].revision, 2);
        assert_eq!(lib.playlists[0].tracks[0].id, 1);
    }
    #[test]
    fn playback_snapshot_is_independent_of_library_edits_and_serializes() {
        let mut lib = Library {
            playlists: vec![list()],
        };
        let queue = lib.playlists[0].tracks.clone();
        lib.edit(LocalLibraryAction::Add {
            id: "local".into(),
            tracks: vec![track(3)],
        })
        .unwrap();
        let restored: Library = serde_json::from_slice(&serde_json::to_vec(&lib).unwrap()).unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(restored.playlists[0].tracks.len(), 3);
    }
    #[test]
    fn pin_state_survives_serialization_and_old_playlists_default_to_unpinned() {
        let mut old = serde_json::to_value(list()).unwrap();
        old.as_object_mut().unwrap().remove("pinned");
        let playlist: LocalPlaylist = serde_json::from_value(old).unwrap();
        assert!(!playlist.pinned);
        let mut library = Library {
            playlists: vec![playlist],
        };
        library
            .edit(LocalLibraryAction::SetPinned {
                id: "local".into(),
                pinned: true,
            })
            .unwrap();
        let mut restored: Library =
            serde_json::from_slice(&serde_json::to_vec(&library).unwrap()).unwrap();
        assert!(restored.playlists[0].pinned);
        assert_eq!(restored.playlists[0].tracks.len(), 2);
        restored
            .edit(LocalLibraryAction::SetPinned {
                id: "local".into(),
                pinned: false,
            })
            .unwrap();
        assert!(!restored.playlists[0].pinned);
    }
    #[test]
    fn favorites_are_idempotent_and_scoped_by_source() {
        let mut lib = Library::default();
        for _ in 0..2 {
            lib.set_favorite(track(1), true).unwrap();
        }
        let mut other = track(1);
        other.source = Some(Source {
            plugin: "test".into(),
            id: "1".into(),
        });
        lib.set_favorite(other.clone(), true).unwrap();
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(lib.playlists[0].name, "我喜欢");
        assert_eq!(lib.playlists[0].tracks.len(), 2);
        let queue = lib.playlists[0].tracks.clone();
        lib.set_favorite(track(1), false).unwrap();
        assert!(!lib.favorite_state("netease:1").liked);
        assert!(lib.favorite_state("mygo:test:1").liked);
        assert_eq!(queue.len(), 2);
        lib.set_favorite(other, false).unwrap();
        assert!(lib.playlists[0].tracks.is_empty());
        assert!(lib.playlists[0].favorite);
    }
    #[test]
    fn existing_named_favorites_keep_their_id_and_cannot_be_deleted_or_renamed() {
        let mut p = list();
        p.name = "我喜欢".into();
        let mut lib = Library { playlists: vec![p] };
        lib.set_favorite(track(3), true).unwrap();
        assert_eq!(lib.playlists.len(), 1);
        assert_eq!(
            lib.favorite_state("netease:3").playlist_id.as_deref(),
            Some("local")
        );
        assert!(
            lib.edit(LocalLibraryAction::Rename {
                id: "local".into(),
                name: "Other".into()
            })
            .is_err()
        );
        assert!(
            lib.edit(LocalLibraryAction::Delete { id: "local".into() })
                .is_err()
        );
        assert_eq!(lib.playlists[0].tracks.len(), 3);
    }
    #[test]
    fn favorites_survive_serialization_and_old_libraries_default_to_ordinary_lists() {
        let mut lib = Library::default();
        lib.set_favorite(track(2), true).unwrap();
        let restored: Library = serde_json::from_slice(&serde_json::to_vec(&lib).unwrap()).unwrap();
        assert!(restored.favorite_state("netease:2").liked);
        let mut old = serde_json::to_value(list()).unwrap();
        old.as_object_mut().unwrap().remove("favorite");
        assert!(
            !serde_json::from_value::<LocalPlaylist>(old)
                .unwrap()
                .favorite
        );
        let mut invalid = track(0);
        invalid.title = String::new();
        let mut empty = Library::default();
        assert!(empty.set_favorite(invalid, true).is_err());
        assert!(empty.playlists.is_empty());
    }
    #[test]
    fn legacy_netease_favorite_survives_switch_to_canonical_plugin() {
        let mut lib = Library::default();
        lib.set_favorite(track(1), true).unwrap();
        let mut plugin = track(1);
        plugin.source = Some(Source {
            plugin: "netease".into(),
            id: "1".into(),
        });
        lib.set_favorite(plugin, true).unwrap();
        assert_eq!(lib.playlists[0].tracks.len(), 1);
    }
}
