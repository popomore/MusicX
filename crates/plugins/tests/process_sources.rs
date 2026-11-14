use musicx_core::protocol::{MusicSourceAction, Quality, Track};
use musicx_plugins::{assets, music_sources, mygo};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn helper_permission_is_explicit_payload_bound_and_playback_scoped() {
    if std::env::var_os("MUSICX_PROCESS_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!(
            "musicx-process-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "helper_permission_is_explicit_payload_bound_and_playback_scoped",
                "--nocapture",
            ])
            .env("MUSICX_PROCESS_CHILD", "1")
            .env("MUSICX_PROCESS_ROOT", &root)
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
    let root = PathBuf::from(std::env::var_os("MUSICX_PROCESS_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir_all(fixture.join("bin")).unwrap();
    fs::write(fixture.join("plugin.json"),json!({"id":"process.fixture","name":"Helper fixture","version":"1.0.0","permissions":{"process":["bin/helper-*"],"network":[]}}).to_string()).unwrap();
    let helper = fixture.join("bin/helper-echo");
    fs::write(
        &helper,
        "#!/bin/sh\nread value\nprintf '%s\\n' \"$value\"\nprintf 'fixture stderr' >&2\n",
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let server = fixture.join("bin/helper-server");
    fs::write(&server,r#"#!/usr/bin/python3
import http.server
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200);self.send_header('Content-Length','11');self.end_headers();self.wfile.write(b'audio-bytes')
    def log_message(self,*args):pass
server=http.server.HTTPServer(('127.0.0.1',0),Handler)
print('http://127.0.0.1:'+str(server.server_port)+'/audio',flush=True)
server.serve_forever()
"#).unwrap();
    fs::set_permissions(&server, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(fixture.join("index.js"),r#"
export async function echo(){const child=host.spawn(host.dir+'/bin/helper-echo',[]);child.write('hello\n');child.closeStdin();const line=await child.readLine();const eof=await child.readLine();const status=await child.wait();return {line,eof,status,pid:child.pid};}
export function forbidden(){return host.spawn('/bin/sh',['-c','exit 0']).pid}
export async function stream({id}){if(id==='local')return {url:'file://'+encodeURI(host.dir+'/clip.wav')};if(id==='outside')return {url:'file:///etc/hosts'};const child=host.spawn('bin/helper-server',[]);const url=await child.readLine();host.storage.set('serverPid',child.pid);return {url};}
export function serverPid(){return host.storage.get('serverPid')}
"#).unwrap();
    fs::write(fixture.join("clip.wav"), b"fixture-file-audio").unwrap();
    fs::write(fixture.join("cover.png"), b"fixture-image").unwrap();
    mygo::install(&fixture).unwrap();
    let installed = root.join("state/mygo-plugins/process.fixture");
    assert!(installed.join("bin/helper-echo").is_file());
    assert_ne!(
        fs::metadata(installed.join("bin/helper-echo"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(
        !music_sources::get_source("mygo:process.fixture")
            .unwrap()
            .process_granted
    );
    assert!(mygo::invoke("process.fixture", "echo", json!({})).is_err());
    let grant = |enabled| {
        music_sources::execute(MusicSourceAction::SetProcessPermission {
            source: "mygo:process.fixture".into(),
            enabled,
            expected_fingerprint: music_sources::get_source("mygo:process.fixture")
                .unwrap()
                .permission_fingerprint,
        })
        .unwrap()
    };
    grant(true);
    assert!(
        music_sources::get_source("mygo:process.fixture")
            .unwrap()
            .process_granted
    );
    let result = mygo::invoke("process.fixture", "echo", json!({})).unwrap();
    assert_eq!(result["line"], "hello");
    assert_eq!(result["eof"], json!(null));
    assert_eq!(result["status"], 0);
    assert!(mygo::invoke("process.fixture", "forbidden", json!({})).is_err());
    let track:Track=serde_json::from_value(json!({"id":1,"title":"Fixture","artist":"Fixture","duration":1,"source":{"plugin":"process.fixture","id":"one"}})).unwrap();
    let media = mygo::resolve_with_quality(track.clone(), Quality::Standard).unwrap();
    let pid = mygo::invoke("process.fixture", "serverPid", json!({}))
        .unwrap()
        .as_i64()
        .unwrap() as i32;
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    assert_eq!(
        reqwest::blocking::get(media.url.clone())
            .unwrap()
            .bytes()
            .unwrap()
            .as_ref(),
        b"audio-bytes"
    );
    grant(false);
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "revoking permission must terminate an active leased helper"
    );
    drop(media);
    grant(true);
    let replacement = mygo::resolve_with_quality(track.clone(), Quality::Standard).unwrap();
    let replacement_pid = mygo::invoke("process.fixture", "serverPid", json!({}))
        .unwrap()
        .as_i64()
        .unwrap() as i32;
    drop(replacement);
    assert_ne!(unsafe { libc::kill(replacement_pid, 0) }, 0);
    assert_ne!(
        unsafe { libc::kill(pid, 0) },
        0,
        "dropping the audio lease must terminate the helper"
    );
    let mut local_track = track.clone();
    local_track.source.as_mut().unwrap().id = "local".into();
    let local = mygo::resolve_with_quality(local_track, Quality::Standard).unwrap();
    let local_path = local.url.to_file_path().unwrap();
    assert_ne!(local_path, installed.join("clip.wav"));
    assert_eq!(fs::read(&local_path).unwrap(), b"fixture-file-audio");
    let reviewed = music_sources::get_source("mygo:process.fixture")
        .unwrap()
        .permission_fingerprint;
    fs::write(installed.join("clip.wav"), b"reused cache path").unwrap();
    assert!(
        music_sources::execute(MusicSourceAction::SetProcessPermission {
            source: "mygo:process.fixture".into(),
            enabled: true,
            expected_fingerprint: reviewed
        })
        .is_err(),
        "an old page must not authorize changed files"
    );
    assert_eq!(fs::read(&local_path).unwrap(), b"fixture-file-audio");
    drop(local);
    assert!(!local_path.exists());
    let mut outside = track;
    outside.source.as_mut().unwrap().id = "outside".into();
    assert!(mygo::resolve_with_quality(outside, Quality::Standard).is_err());
    assert_eq!(
        assets::image(
            "process.fixture",
            url::Url::from_file_path(installed.join("cover.png"))
                .unwrap()
                .as_str()
        )
        .unwrap(),
        b"fixture-image"
    );
    assert!(assets::image("process.fixture", "file:///etc/hosts").is_err());
    fs::write(installed.join("extra.txt"), "changed payload").unwrap();
    assert!(
        !music_sources::get_source("mygo:process.fixture")
            .unwrap()
            .process_granted
    );
    assert!(mygo::invoke("process.fixture", "echo", json!({})).is_err());
    grant(true);
    grant(false);
    assert!(
        !music_sources::get_source("mygo:process.fixture")
            .unwrap()
            .process_granted
    );
}
