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
        ,"permissions":{"network":["*.example.com"]}
        ,"settings":[{"key":"oauthPort","label":"OAuth port"},{"key":"defaultHost","label":"Default host","value":"https://private.example.com"},{"key":"headerValue","label":"Header value","value":"default-fixture"}]
        ,"requestHeaders":[{"hosts":["{setting:defaultHost}"],"kinds":["image","audio"],"headers":{"authorization":"Bearer {storage:apiToken}","x-default":"{setting:headerValue}"}}]
    })).unwrap()).unwrap();
    fs::create_dir(fixture.join("locales")).unwrap();
    fs::write(
        fixture.join("locales/zh-CN.json"),
        r#"{"Hello source":"来源你好"}"#,
    )
    .unwrap();
    fs::write(fixture.join("index.js"), r#"
export function search({kind='tracks'}={}) {
  if(kind!=='tracks') return {[kind]:[{id:kind+':one',name:'Fixture '+kind}],more:false};
  return {tracks:[{id:'track:123',name:'公开接口曲名',artists:[{id:'artist:one',name:'歌手'}],duration:60000,
    album:{id:'album',name:'专辑',imageUrl:'https://example.com/cover.jpg'}}],total:1,hasMore:false};
}
export function stream({id,quality}) {
  host.storage.set('stream',{id,quality});
  if(id==='unsupported-headers') return {url:'https://example.com/track.mp3',headers:{Authorization:'fixture'}};
  return {url:'https://example.com/'+id+'.mp3',type:'mp3',headers:{'User-Agent':'Fixture Player'}};
}
export function playlists() {
  return [{id:'collection:mix/001',name:'Fixture playlist',trackCount:1}];
}
export function playlist({id}) {
  host.storage.set('playlistId',id);
  return {id,name:'Fixture playlist',trackCount:1,tracks:search().tracks};
}
export function lyrics() { return {lines:[{start:1000,end:2000,words:[{start:1000,end:1500,text:'Hello'},{start:1500,end:2000,text:' world'}],translation:'你好'}]}; }
export function album() { return {album:{id:'album',name:'Fixture album'},tracks:search().tracks}; }
export function artist({id}) { return {artist:{id,name:'Fixture artist'},tracks:search().tracks,albums:[{id:'album',name:'Fixture album'}]}; }
export function shows() { return [{id:'show:one',name:'Fixture podcast',publisher:'Fixture publisher'}]; }
export function show({id,offset}) { return {show:{id,name:'Fixture podcast'},episodes:[{...search().tracks[0],id:'episode:'+offset,showId:id,description:'An episode',published:'2026-11-10'}],more:offset===0}; }
export function liked({offset}) { return {tracks:[{...search().tracks[0],id:host.storage.get('badPages')?'same':'liked:'+offset}],more:host.storage.get('badPages')||offset===0}; }
export function recent() { return [{...search().tracks[0],id:'recent:one'}]; }
export function setSetting({key,value}) { host.storage.set(key,value); }
export function translated() { return {text:host.t('Hello source'),language:host.language()}; }
export function requiresSignIn() { throw Object.assign(new Error('fixture sign-in needed'),{code:'signIn'}); }
export function signOut() { host.storage.remove('method'); return {}; }
export async function signIn({method}) {
  if(method==='oauth') {
    if(false) host.openURL('https://private.example.com/login');
    const query=await host.waitForRedirect({port:Number(host.setting('oauthPort')),path:'/callback',timeout:5000});
    host.storage.set('callbackCode',query.code);
  }
  if(host.setting('nativeFixtureLogin')==='enabled') {
    if(method==='web') await host.webSignIn({url:'https://example.com'});
    else host.signInPrompt({text:'fixture'});
  }
  host.storage.set('method',method);
  host.storage.set('settingMethod',host.setting('loginMethod'));
  return {id:'fixture-user',name:method};
}
export function account() {
  return {id:'fixture-user',name:host.storage.get('method'),settingMethod:host.storage.get('settingMethod'),callbackCode:host.storage.get('callbackCode')};
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
        Some("https://example.com/cover.jpg#musicx-plugin=fixture-source")
    );
    assert_eq!(track.source.as_ref().unwrap().id, "track:123");
    assert_eq!(track.artists[0].id, "artist:one");
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
    assert_eq!(info.lyrics.lines[0].start, 1000);
    assert_eq!(info.lyrics.lines[0].words[1].start, 1500);
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
    let mut protected = media.track;
    protected.source.as_mut().unwrap().id = "unsupported-headers".into();
    let resolved = MYGO.resolve(protected, Default::default()).unwrap();
    assert_eq!(resolved.url.host_str(), Some("127.0.0.1"));
    assert!(resolved.user_agent.is_none());
    let execute =
        |action: serde_json::Value| music_sources::execute(serde_json::from_value(action).unwrap());
    assert_eq!(
        mygo::search_kind("fixture-source", "test", "artists", 0).unwrap()["artists"][0]["id"],
        "artists:one"
    );
    let artist =
        execute(json!({"action":"artist","source":"mygo:fixture-source","id":"artist:one"}))
            .unwrap();
    assert_eq!(artist["title"], "Fixture artist");
    assert_eq!(artist["tracks"][0]["source"]["plugin"], "fixture-source");
    assert_eq!(artist["collections"][0]["kind"], "album");
    let shows = execute(json!({"action":"shows","source":"mygo:fixture-source"})).unwrap();
    assert_eq!(shows["collections"][0]["name"], "Fixture podcast");
    let show =
        execute(json!({"action":"show","source":"mygo:fixture-source","id":"show:one"})).unwrap();
    assert_eq!(show["tracks"][0]["episode"]["show_id"], "show:one");
    assert_eq!(show["more"], true);
    assert_eq!(show["next_offset"], 1);
    let second =
        execute(json!({"action":"show","source":"mygo:fixture-source","id":"show:one","offset":1}))
            .unwrap();
    assert_eq!(second["more"], false);
    let liked = execute(json!({"action":"import_liked","source":"mygo:fixture-source"})).unwrap();
    assert_eq!(liked["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(liked["imported_from"]["id"], "collection:liked");
    let recent = execute(json!({"action":"import_recent","source":"mygo:fixture-source"})).unwrap();
    assert_eq!(recent["tracks"][0]["source"]["id"], "recent:one");
    let before = musicx_core::local_library::lists().unwrap().len();
    mygo::invoke(
        "fixture-source",
        "setSetting",
        json!({"key":"badPages","value":true}),
    )
    .unwrap();
    assert!(execute(json!({"action":"import_liked","source":"mygo:fixture-source"})).is_err());
    assert_eq!(musicx_core::local_library::lists().unwrap().len(), before);
    assert_eq!(
        mygo::invoke("fixture-source", "translated", json!({})).unwrap()["text"],
        "来源你好"
    );
    let error = mygo::invoke("fixture-source", "requiresSignIn", json!({})).unwrap_err();
    assert_eq!(mygo::error_code(&error), Some("signIn"));
    mygo::invoke(
        "fixture-source",
        "setSetting",
        json!({"key":"apiToken","value":"fixture-private"}),
    )
    .unwrap();
    let headers = musicx_plugins::assets::headers(
        "fixture-source",
        &"https://private.example.com/image".parse().unwrap(),
        "image",
    )
    .unwrap();
    assert_eq!(headers["authorization"], "Bearer fixture-private");
    assert_eq!(headers["x-default"], "default-fixture");
    music_sources::configure(
        "mygo:fixture-source",
        std::collections::BTreeMap::from([("headerValue".into(), "saved-override".into())]),
    )
    .unwrap();
    assert_eq!(
        musicx_plugins::assets::headers(
            "fixture-source",
            &"https://private.example.com/image".parse().unwrap(),
            "image"
        )
        .unwrap()["x-default"],
        "saved-override"
    );
    assert!(
        musicx_plugins::assets::headers(
            "fixture-source",
            &"https://other.example.com/image".parse().unwrap(),
            "image"
        )
        .unwrap()
        .is_empty()
    );

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
    #[cfg(target_os = "macos")]
    {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        music_sources::configure(
            "mygo:fixture-source",
            std::collections::BTreeMap::from([("oauthPort".into(), port.to_string())]),
        )
        .unwrap();
        let started =
            execute(json!({"action":"sign_in","source":"mygo:fixture-source","method":"oauth"}))
                .unwrap();
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap();
        let mut answered = false;
        for _ in 0..200 {
            if let Ok(response) = client
                .get(format!(
                    "http://127.0.0.1:{port}/callback?code=fixture-code"
                ))
                .send()
            {
                assert!(response.status().is_success());
                answered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(answered);
        let job = started["job"].as_str().unwrap();
        for _ in 0..200 {
            let state = music_sources::login_state(job, false).unwrap();
            if state.status == "success" {
                break;
            }
            assert_ne!(state.status, "error", "{}", state.text);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            music_sources::login_state(job, false).unwrap().status,
            "success"
        );
        assert_eq!(
            mygo::invoke("fixture-source", "account", json!({})).unwrap()["callbackCode"],
            "fixture-code"
        );
    }
    music_sources::execute(
        serde_json::from_value(json!({
            "action":"sign_out","source":"mygo:fixture-source"
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(!root.join("state/mygo-plugins/netease").exists());

    // An update replaces compiled code, while retaining configuration and storage.
    let script = fs::read_to_string(fixture.join("index.js")).unwrap();
    fs::write(
        fixture.join("index.js"),
        format!("{script}\nexport function revisionProbe() {{ return 2; }}"),
    )
    .unwrap();
    mygo::update_local(&fixture).unwrap();
    assert_eq!(
        mygo::invoke("fixture-source", "revisionProbe", json!({})).unwrap(),
        2
    );
    assert_eq!(
        musicx_plugins::assets::headers(
            "fixture-source",
            &"https://private.example.com/cover".parse().unwrap(),
            "image"
        )
        .unwrap()["authorization"],
        "Bearer fixture-private"
    );
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.join("plugin.json")).unwrap()).unwrap();
    manifest["minAppVersion"] = json!("99.0.0");
    fs::write(
        fixture.join("plugin.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(mygo::update_local(&fixture).is_err());
    assert_eq!(
        mygo::invoke("fixture-source", "revisionProbe", json!({})).unwrap(),
        2
    );
    let lists_before_removal = musicx_core::local_library::lists().unwrap().len();
    execute(json!({"action":"uninstall","source":"mygo:fixture-source"})).unwrap();
    assert!(!root.join("state/mygo-fixture-source-storage.json").exists());
    assert_eq!(
        musicx_core::local_library::lists().unwrap().len(),
        lists_before_removal
    );

    let typed = root.join("typed");
    fs::create_dir(&typed).unwrap();
    fs::write(
        typed.join("plugin.json"),
        r#"{"id":"fixture-typed","main":"index.ts"}"#,
    )
    .unwrap();
    fs::write(typed.join("index.ts"), "interface Input {id:string}; host.storage.set('executed',true); export function stream({id}:Input):{url:string} { return {url:'https://example.com/'+id}; }").unwrap();
    mygo::install(&typed).unwrap();
    assert!(!root.join("state/mygo-fixture-typed-storage.json").exists());
    assert_eq!(
        mygo::invoke("fixture-typed", "stream", json!({"id":"typed.mp3"})).unwrap()["url"],
        "https://example.com/typed.mp3"
    );
    let defaults = root.join("defaults");
    fs::create_dir(&defaults).unwrap();
    fs::write(defaults.join("plugin.json"), r#"{"id":"fixture-defaults"}"#).unwrap();
    fs::write(
        defaults.join("index.js"),
        "export function stream() { return {url:'https://example.com/default.mp3'}; }",
    )
    .unwrap();
    mygo::install(&defaults).unwrap();
    assert_eq!(
        music_sources::get_source("mygo:fixture-defaults")
            .unwrap()
            .name,
        "fixture-defaults"
    );
}
