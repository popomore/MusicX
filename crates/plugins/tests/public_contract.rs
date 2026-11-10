//! Exercise the public boundary in a child process so tests never share user data.
use musicx_plugin_api::DataSource;
use musicx_plugins::{MYGO, music_sources, mygo};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn all_sources_are_explicit_plugins() {
    let directory = std::env::temp_dir().join(format!(
        "musicx-public-contract-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "public_exports_child", "--nocapture"])
        .env("MUSICX_PUBLIC_CONTRACT_CHILD", "1")
        .env("MUSICX_PUBLIC_CONTRACT_ROOT", &directory)
        .env("MUSICX_DATA_DIR", directory.join("state"))
        .env("MUSICX_PLUGIN_CATALOG_URL", "not-a-url")
        .output()
        .unwrap();
    fs::remove_dir_all(&directory).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn public_exports_child() {
    if std::env::var("MUSICX_PUBLIC_CONTRACT_CHILD").as_deref() != Ok("1") {
        return;
    }
    assert!(music_sources::list().unwrap().is_empty());
    let root = PathBuf::from(std::env::var_os("MUSICX_PUBLIC_CONTRACT_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir(&fixture).unwrap();
    fs::write(fixture.join("plugin.json"), serde_json::to_vec(&json!({
        "id":"fixture-source", "name":"Public contract fixture", "version":"9.9.9", "main":"index.js", "playback":"stream",
        "repository":"https://github.com/example/fixture-source"
    })).unwrap()).unwrap();
    fs::write(fixture.join("index.js"), r#"
export function search() {
  return {tracks:[{id:'track:123',name:'公开接口曲名',artists:[{name:'歌手'}],duration:60000,
    album:{id:'album',name:'专辑',imageUrl:'https://example.com/cover.jpg'}}],total:1,hasMore:false};
}
export function stream({id,quality}) {
  host.storage.set('stream',{id,quality});
  if(id==='unsupported-headers') return {url:'https://example.com/track.mp3',headers:{Authorization:'fixture'}};
  return {url:'https://example.com/'+id+'.mp3',type:'mp3',headers:{'User-Agent':'Fixture Player',Referer:'https://example.com/'}};
}
export function playlists() {
  return [{id:'collection:mix/001',name:'Fixture playlist',trackCount:1}];
}
export function playlist({id}) {
  host.storage.set('playlistId',id);
  return {id,name:'Fixture playlist',trackCount:1,tracks:search().tracks};
}
export function lyrics() { return {lines:[]}; }
export function album() { return {id:'album',name:'Fixture album',tracks:search().tracks}; }
export function signOut() { host.storage.remove('method'); return {}; }
export async function signIn({method}) {
  if(host.setting('nativeFixtureLogin')==='enabled') {
    if(method==='web') await host.webSignIn({url:'https://example.com'});
    else host.signInPrompt({text:'fixture'});
  }
  host.storage.set('method',method);
  host.storage.set('settingMethod',host.setting('loginMethod'));
  return {id:'fixture-user',name:method};
}
export function account() {
  return {id:'fixture-user',name:host.storage.get('method'),settingMethod:host.storage.get('settingMethod')};
}
"#).unwrap();
    mygo::install(&fixture).unwrap();

    let result = mygo::search("fixture-source", "fixture", 0).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["hasMore"], false);
    let track: musicx_core::protocol::Track =
        serde_json::from_value(result["tracks"][0].clone()).unwrap();
    assert_eq!(track.duration, 60.0);
    assert_eq!(
        track.cover_url.as_deref(),
        Some("https://example.com/cover.jpg")
    );
    assert_eq!(track.source.as_ref().unwrap().id, "track:123");
    let media = MYGO
        .resolve_selection(track.clone(), Default::default())
        .unwrap();
    assert_eq!(media.track.title, "公开接口曲名");
    assert_eq!(media.track.artist, "歌手");
    assert_eq!(
        media.track.source.as_ref().unwrap().plugin,
        "fixture-source"
    );
    assert_eq!(media.url.as_str(), "https://example.com/track:123.mp3");
    assert_eq!(media.user_agent.as_deref(), Some("Fixture Player"));
    let info = MYGO.info(media.track.clone());
    assert_eq!(info.source_name, "Public contract fixture");
    assert_eq!(
        info.source_link.unwrap().url,
        "https://github.com/example/fixture-source"
    );
    let album = MYGO.album(media.track.clone()).unwrap();
    assert_eq!(album.album.name, "Fixture album");
    assert_eq!(
        album.tracks[0].source.as_ref().unwrap().plugin,
        "fixture-source"
    );
    let mut supplied = track;
    supplied.title = "调用方给定的标题".into();
    let media = MYGO
        .resolve_selection(supplied.clone(), Default::default())
        .unwrap();
    assert_eq!(media.track.title, "调用方给定的标题");
    supplied.source = None;
    assert!(
        MYGO.resolve_selection(supplied.clone(), Default::default())
            .is_err()
    );
    let info = MYGO.info(supplied);
    assert_eq!(info.source_name, "未关联来源");
    assert!(info.source_link.is_none());
    assert!(!root.join("state/mygo_source_catalog.json").exists());
    assert!(!root.join("state/mygo-plugins/netease").exists());
    let sources = music_sources::list().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].id, "mygo:fixture-source");
    assert!(music_sources::get_source("fixture-source").is_err());
    let playlists = music_sources::playlists("mygo:fixture-source").unwrap();
    assert_eq!(playlists[0].id, "collection:mix/001");
    let imported = musicx_plugins::change_library(serde_json::from_value(json!({
        "action":"import_source","source":"mygo:fixture-source","playlist_id":"  collection:mix/001  "
    })).unwrap()).unwrap();
    assert_eq!(imported["imported_from"]["provider"], "mygo:fixture-source");
    assert_eq!(imported["imported_from"]["id"], "collection:mix/001");
    assert_eq!(imported["tracks"][0]["source"]["plugin"], "fixture-source");
    let mut unsupported = media.track;
    unsupported.source.as_mut().unwrap().id = "unsupported-headers".into();
    assert!(MYGO.resolve(unsupported, Default::default()).is_err());

    let methods = if cfg!(target_os = "macos") {
        vec!["qr", "web"]
    } else {
        vec!["qr"]
    };
    for method in methods {
        let action = serde_json::from_value(
            json!({"action":"sign_in","source":"mygo:fixture-source","method":method}),
        )
        .unwrap();
        let started = music_sources::execute(action).unwrap();
        let job = started["job"].as_str().unwrap();
        let mut complete = false;
        for _ in 0..500 {
            let status = music_sources::login_state(job, false).unwrap();
            assert_ne!(status.status, "error", "{}", status.text);
            if status.status == "success" {
                complete = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(complete);
        std::thread::sleep(Duration::from_millis(10));
        let account = mygo::invoke("fixture-source", "account", json!({})).unwrap();
        assert_eq!(account["name"], method);
        assert_eq!(account["settingMethod"], method);
    }
    music_sources::execute(
        serde_json::from_value(json!({
            "action":"sign_out","source":"mygo:fixture-source"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!root.join("state/mygo-plugins/netease").exists());
}
