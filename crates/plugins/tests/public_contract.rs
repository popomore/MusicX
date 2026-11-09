//! Exercise the public boundary in a child process so tests never share user data.
use musicx_plugin_api::DataSource;
use musicx_plugins::{MYGO, music_sources, mygo, netease};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn public_plugin_exports_work_without_private_netease_symbols() {
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
        .env("MINI_CLOUDMUSIC_DATA_DIR", directory.join("state"))
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
    let root = PathBuf::from(std::env::var_os("MUSICX_PUBLIC_CONTRACT_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir(&fixture).unwrap();
    fs::write(fixture.join("plugin.json"), serde_json::to_vec(&json!({
        "id":"netease", "name":"Public contract fixture", "version":"9.9.9", "main":"index.js", "playback":"stream"
    })).unwrap()).unwrap();
    fs::write(fixture.join("index.js"), r#"
export function search() {
  return {tracks:[{id:'123',name:'公开接口曲名',artists:[{name:'歌手'}],duration:60000,
    album:{id:'album',name:'专辑',imageUrl:'https://example.com/cover.jpg'}}]};
}
export function stream({id,quality}) {
  host.storage.set('stream',{id,quality});
  return {url:'https://example.com/'+id+'.mp3',type:'mp3'};
}
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

    let rows = netease::search("fixture".into(), 1).unwrap();
    let media = netease::resolve_with_quality(123, Default::default()).unwrap();
    assert_eq!(media.track.title, "公开接口曲名");
    assert_eq!(media.track.artist, "歌手");
    assert_eq!(media.url.as_str(), "https://example.com/123.mp3");
    let mut supplied = rows[0].clone();
    supplied.source = None;
    supplied.title = "调用方给定的标题".into();
    let media = MYGO
        .resolve_selection(supplied, Default::default())
        .unwrap();
    assert_eq!(media.track.title, "调用方给定的标题");
    let unknown = netease::resolve_with_quality(456, Default::default()).unwrap();
    assert_eq!(unknown.track.title, "456");
    assert_eq!(unknown.url.as_str(), "https://example.com/456.mp3");
    assert!(!root.join("state/mygo_source_catalog.json").exists());

    let methods = if cfg!(target_os = "macos") {
        vec!["qr", "web"]
    } else {
        vec!["qr"]
    };
    for method in methods {
        let action = serde_json::from_value(
            json!({"action":"sign_in","source":"mygo:netease","method":method}),
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
        let account = mygo::invoke("netease", "account", json!({})).unwrap();
        assert_eq!(account["name"], method);
        assert_eq!(account["settingMethod"], method);
    }
}
