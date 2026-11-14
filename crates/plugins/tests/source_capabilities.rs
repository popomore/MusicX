use musicx_core::protocol::MusicSourceAction;
use musicx_plugins::{music_sources, mygo};
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn public_source_actions_preserve_identity_and_require_explicit_writes() {
    if std::env::var_os("MUSICX_CAPABILITY_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!(
            "musicx-capability-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "public_source_actions_preserve_identity_and_require_explicit_writes",
                "--nocapture",
            ])
            .env("MUSICX_CAPABILITY_CHILD", "1")
            .env("MUSICX_CAPABILITY_ROOT", &root)
            .env("MUSICX_DATA_DIR", root.join("state"))
            .env("MUSICX_LANGUAGE", "en")
            .output()
            .unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_CAPABILITY_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("plugin.json"),r#"{"id":"capability.fixture","name":"Fixture","version":"1.0.0","permissions":{"network":[]}}"#).unwrap();
    fs::write(fixture.join("index.js"),r#"
const track={id:'opaque:one',name:'Song',artists:[{id:'artist',name:'Artist'}],duration:42000,unavailable:true};
export function home(){return {title:'Home',sections:[{tracks:[track]},{playlists:[{id:'list',tracks:[track]}]}]};}
export function browse({page}){return {title:page,sections:[{tracks:[track]}]};}
export function stationNext({id}){return id==='envelope'?{tracks:[track]}:[track]}
export function stations(){return [{id:'fm',name:'Fixture FM'}]}
export function playlist(){return {playlist:{name:'Wrapped playlist',tracks:[track]}}}
export function createPlaylist({name}){host.storage.set('writes',(host.storage.get('writes')||0)+1);return {id:'created',name,tracks:[track]}}
export function comments(){return {comments:[{id:'c',text:'hello'}],more:false}}
export function qqReadSession(){return {token:'must-not-be-exposed'}}
export function inspect(){return host.storage.get('writes')||0}
export function stream(){return {url:'https://example.com/song.mp3'}}
"#).unwrap();
    mygo::install(&fixture).unwrap();
    let call = |method: &str, params| {
        music_sources::execute(MusicSourceAction::Invoke {
            source: "mygo:capability.fixture".into(),
            method: method.into(),
            params,
        })
    };
    let native_page = music_sources::execute(MusicSourceAction::Page {
        source: "mygo:capability.fixture".into(),
        method: "home".into(),
        params: json!({}),
    })
    .unwrap();
    assert_eq!(native_page["tracks"][0]["duration"], 42.0);
    assert_eq!(native_page["collections"][0]["kind"], "playlist");
    let wrapped = music_sources::execute(MusicSourceAction::Page {
        source: "mygo:capability.fixture".into(),
        method: "playlist".into(),
        params: json!({"id":"wrapped:collection/one"}),
    })
    .unwrap();
    assert_eq!(wrapped["title"], "Wrapped playlist");
    assert_eq!(wrapped["tracks"][0]["duration"], 42.0);
    assert_eq!(wrapped["tracks"][0]["source"]["id"], "opaque:one");
    let station_page = music_sources::execute(MusicSourceAction::Page {
        source: "mygo:capability.fixture".into(),
        method: "stations".into(),
        params: json!({}),
    })
    .unwrap();
    assert_eq!(station_page["collections"][0]["id"], "fm");
    let station = music_sources::execute(MusicSourceAction::Page {
        source: "mygo:capability.fixture".into(),
        method: "stationNext".into(),
        params: json!({"id":"fm"}),
    })
    .unwrap();
    assert_eq!(station["station"]["source"], "mygo:capability.fixture");
    assert_eq!(station["tracks"][0]["duration"], 42.0);
    let envelope = call("stationNext", json!({"id":"envelope"})).unwrap();
    assert_eq!(envelope["tracks"][0]["duration"], 42.0);
    assert_eq!(
        envelope["tracks"][0]["source"]["plugin"],
        "capability.fixture"
    );
    let page = call("home", json!({})).unwrap();
    let track = &page["sections"][0]["tracks"][0];
    assert_eq!(
        track["source"],
        json!({"plugin":"capability.fixture","id":"opaque:one"})
    );
    assert_eq!(track["duration"], 42.0);
    assert_eq!(track["unavailable"], true);
    assert_eq!(
        page["sections"][1]["playlists"][0]["tracks"][0]["source"],
        track["source"]
    );
    assert_eq!(
        mygo::invoke("capability.fixture", "inspect", json!({})).unwrap(),
        0
    );
    assert!(call("qqReadSession", json!({})).is_err());
    assert!(call("createPlaylist", json!({})).is_err());
    assert_eq!(
        mygo::invoke("capability.fixture", "inspect", json!({})).unwrap(),
        0
    );
    let created = call("createPlaylist", json!({"name":"Explicit cloud action"})).unwrap();
    assert_eq!(created["id"], "created");
    assert_eq!(created["tracks"][0]["source"], track["source"]);
    assert_eq!(
        mygo::invoke("capability.fixture", "inspect", json!({})).unwrap(),
        1
    );
    assert_eq!(
        call("stationNext", json!({"id":"fm"})).unwrap()[0]["source"],
        track["source"]
    );
    assert!(call("browse", json!({"page":""})).is_err());
    assert!(call("comments", json!({"trackId":"id","offset":-1})).is_err());
    assert_eq!(
        call("comments", json!({"trackId":"id","offset":0})).unwrap()["comments"][0]["text"],
        "hello"
    );
    assert!(
        !music_sources::get_source("mygo:capability.fixture")
            .unwrap()
            .host_capabilities
            .iter()
            .any(|c| c == "qqReadSession")
    );
    let manifest_path = root.join("state/mygo-plugins/capability.fixture/plugin.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["background"] = json!(true);
    fs::write(manifest_path, manifest.to_string()).unwrap();
    assert!(
        mygo::invoke("capability.fixture", "inspect", json!({}))
            .unwrap_err()
            .to_string()
            .contains("Background extensions"),
        "an already-installed background extension must also be blocked at invocation"
    );
}
