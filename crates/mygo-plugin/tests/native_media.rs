//! Remote-control exports are runtime compatibility, never native audio.
use musicx_core::protocol::{Quality, Track};
use musicx_mygo_plugin::{MYGO, music_sources, mygo};
use musicx_plugin::DataSource;
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn remote_only_sources_cannot_replace_musicx_audio() {
    if std::env::var_os("MUSICX_REMOTE_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!("musicx-remote-{}", std::process::id()));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "remote_only_sources_cannot_replace_musicx_audio",
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
    let script = r#"
function record(){host.storage.set('calls',(host.storage.get('calls')||0)+1)}
export function account(){record();return {id:'fixture-account'}}
export function remotePlay(){record()}
export function remotePause(){record()}
export function remoteResume(){record()}
export function remoteState(){record();return {playing:true}}
export function remoteDevices(){record();return []}
export function remoteUse(){record()}
export function inspect(){return host.storage.get('calls')||0}
export function home(){return {tracks:[{id:'opaque:track',name:'Remote metadata',duration:60000,artists:[]}]}}
"#;
    let track: Track = serde_json::from_value(json!({"id":1,"title":"Fixture","artist":"Artist","duration":60,"source":{"plugin":"remote.fixture","id":"0123456789012345678901"}})).unwrap();
    for id in ["remote.fixture", "spotify"] {
        let manifest = json!({"id":id,"name":"Remote fixture","version":"1.0.0","playback":"remote","permissions":{"network":[]}}).to_string();
        fs::write(fixture.join("plugin.json"), &manifest).unwrap();
        fs::write(fixture.join("index.js"), script).unwrap();
        mygo::install(&fixture).unwrap();
        let descriptor = music_sources::get_source(&format!("mygo:{id}")).unwrap();
        assert!(
            !descriptor.supports(musicx_core::protocol::SourceCapability::Playback),
            "a remote-only source must not advertise MusicX audio"
        );
        let mut selected = track.clone();
        selected.source.as_mut().unwrap().plugin = id.into();
        let result = MYGO.resolve(selected, Quality::Standard);
        assert!(
            result.is_err(),
            "remote control must never resolve to a playable Media"
        );
        assert!(descriptor.adapter_exports.iter().any(|c| c == "remotePlay"));
        assert!(descriptor.supports(musicx_core::protocol::SourceCapability::Account));
        let page = musicx_plugin::PluginSource::page(
            &MYGO,
            &format!("mygo:{id}"),
            musicx_core::protocol::SourcePageRequest::Recommendations {},
        )
        .unwrap();
        assert_eq!(page.tracks.len(), 1);
        assert!(
            !page.playable,
            "a metadata page must not advertise playback"
        );
        assert_eq!(
            mygo::invoke(id, "inspect", json!({})).unwrap(),
            0,
            "resolving remote-only audio must not read account credentials or control devices"
        );
        let installed = root.join("state/mygo-plugins").join(id);
        assert_eq!(
            fs::read_to_string(installed.join("index.js")).unwrap(),
            script
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(installed.join("plugin.json")).unwrap()).unwrap();
        assert_eq!(saved["playback"], "remote");
    }

    // A source that actually exports audio uses the same native Media contract,
    // even if its unchanged MyGo manifest also advertises remote control.
    for (id, audio_url, accepted) in [
        ("hybrid.fixture", "https://example.com/audio.mp3", true),
        ("marker.fixture", "musicx-remote://device", false),
    ] {
        fs::write(fixture.join("plugin.json"), json!({"id":id,"name":"Audio fixture","version":"1.0.0","playback":"remote","permissions":{"network":[]}}).to_string()).unwrap();
        fs::write(fixture.join("index.js"), format!("export function stream(){{return {{url:{audio_url:?}}}}};export function remotePlay(){{throw new Error('Must not control devices')}}")).unwrap();
        mygo::install(&fixture).unwrap();
        let mut selection = track.clone();
        selection.source.as_mut().unwrap().plugin = id.into();
        let media = MYGO.resolve(selection, Quality::Standard);
        assert_eq!(media.is_ok(), accepted);
        if let Ok(media) = media {
            assert_eq!(media.url.as_str(), audio_url);
        }
    }
}
