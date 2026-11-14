//! Exercise the native adapter against actual isolated JavaScript exports.
use musicx_core::protocol::{Quality, Track};
use musicx_plugin_api::DataSource;
use musicx_plugins::{MYGO, mygo};
use serde_json::{Value, json};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

fn wait(check: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(4);
    while !check() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(check());
}

#[test]
fn remote_adapter_calls_real_exports_and_rejects_stream_url_impersonation() {
    if std::env::var_os("MUSICX_REMOTE_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!(
            "musicx-remote-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "remote_adapter_calls_real_exports_and_rejects_stream_url_impersonation",
                "--nocapture",
            ])
            .env("MUSICX_REMOTE_CHILD", "1")
            .env("MUSICX_REMOTE_ROOT", &root)
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
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_REMOTE_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("plugin.json"),json!({"id":"remote.fixture","name":"Remote fixture","version":"1.0.0","playback":"remote","permissions":{"network":[]}}).to_string()).unwrap();
    fs::write(fixture.join("index.js"),r#"
function record(method,args){const calls=host.storage.get('calls')||[];calls.push({method,args});host.storage.set('calls',calls)}
export function remoteVolume(args){record('remoteVolume',args);host.storage.set('volume',args.percent)}
export function remotePlay(args){record('remotePlay',args);host.storage.set('id',args.ids[args.start]);host.storage.set('position',args.position);host.storage.set('playing',true)}
export function remoteSeek(args){record('remoteSeek',args);host.storage.set('position',args.position)}
export function remotePause(){record('remotePause',{});host.storage.set('playing',false)}
export function remoteResume(){record('remoteResume',{});host.storage.set('playing',true)}
export function remoteState(){record('remoteState',{});return {trackId:host.storage.get('id'),position:host.storage.get('position'),duration:60000,playing:host.storage.get('playing'),volume:host.storage.get('volume')}}
export function inspect(){return host.storage.get('calls')||[]}
"#).unwrap();
    mygo::install(&fixture).unwrap();
    let track:Track=serde_json::from_value(json!({"id":1,"title":"Fixture","artist":"Artist","duration":60,"source":{"plugin":"remote.fixture","id":"opaque:one"}})).unwrap();
    let media = MYGO.resolve(track.clone(), Quality::Standard).unwrap();
    assert_eq!(media.url.scheme(), musicx_plugin_api::REMOTE_SCHEME);
    let session = MYGO.remote(&track).unwrap();
    let calls = || {
        mygo::invoke("remote.fixture", "inspect", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    };
    session.start(true, 0.2).unwrap();
    wait(|| session.snapshot().playing);
    let log = calls();
    assert_eq!(
        log[0],
        json!({"method":"remoteVolume","args":{"percent":20}})
    );
    assert_eq!(
        log[1],
        json!({"method":"remotePlay","args":{"ids":["opaque:one"],"start":0,"position":0}})
    );
    let (done, complete) = std::sync::mpsc::channel();
    session.seek(
        2.5,
        Box::new(move |ok| {
            let _ = done.send(ok);
        }),
    );
    assert!(complete.recv_timeout(Duration::from_secs(3)).unwrap());
    session.set_volume(0.3);
    wait(|| {
        calls()
            .iter()
            .any(|c| c["method"] == "remoteVolume" && c["args"]["percent"] == 30)
    });
    assert!(
        calls()
            .iter()
            .any(|c| c["method"] == "remoteSeek" && c["args"]["position"] == 2500)
    );
    wait(|| calls().iter().any(|c| c["method"] == "remoteState"));
    assert_eq!(session.snapshot().duration, Some(60.0));
    session.pause();
    wait(|| !session.snapshot().playing);
    session.play();
    wait(|| calls().iter().any(|c| c["method"] == "remoteResume"));
    drop(session);
    wait(|| {
        calls()
            .iter()
            .filter(|c| c["method"] == "remotePause")
            .count()
            >= 2
    });
    let basic_script=fs::read_to_string(fixture.join("index.js")).unwrap().replace("export function remoteVolume(args){record('remoteVolume',args);host.storage.set('volume',args.percent)}","").replace("export function remoteSeek(args){record('remoteSeek',args);host.storage.set('position',args.position)}","");
    let mut basic_manifest: Value =
        serde_json::from_slice(&fs::read(fixture.join("plugin.json")).unwrap()).unwrap();
    basic_manifest["id"] = json!("remote.basic");
    fs::write(fixture.join("plugin.json"), basic_manifest.to_string()).unwrap();
    fs::write(fixture.join("index.js"), basic_script).unwrap();
    mygo::install(&fixture).unwrap();
    let mut basic_track = track.clone();
    basic_track.source.as_mut().unwrap().plugin = "remote.basic".into();
    MYGO.resolve(basic_track.clone(), Quality::Standard)
        .unwrap();
    let basic = MYGO.remote(&basic_track).unwrap();
    basic.start(true, 0.0).unwrap();
    wait(|| basic.snapshot().playing);
    basic.set_volume(0.1);
    let (done, complete) = std::sync::mpsc::channel();
    basic.seek(
        5.0,
        Box::new(move |ok| {
            let _ = done.send(ok);
        }),
    );
    assert!(!complete.recv_timeout(Duration::from_secs(3)).unwrap());
    assert!(basic.snapshot().playing && basic.snapshot().error.is_none());
    assert_eq!(basic.snapshot().volume, 1.0);
    drop(basic);
    wait(|| {
        mygo::invoke("remote.basic", "inspect", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["method"] == "remotePause")
    });
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(fixture.join("plugin.json")).unwrap()).unwrap();
    manifest["id"] = json!("stream.fixture");
    manifest["playback"] = json!("stream");
    fs::write(fixture.join("plugin.json"), manifest.to_string()).unwrap();
    fs::write(
        fixture.join("index.js"),
        "export function stream(){return {url:'musicx-remote://device'}}",
    )
    .unwrap();
    mygo::install(&fixture).unwrap();
    let mut impostor = track;
    impostor.source.as_mut().unwrap().plugin = "stream.fixture".into();
    assert!(MYGO.resolve(impostor, Quality::Standard).is_err());
}
