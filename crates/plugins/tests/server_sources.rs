//! A user-configured server may be private; another origin never inherits access.
use musicx_core::protocol::{MusicSourceAction, Quality, Track};
use musicx_plugins::{assets, music_sources, mygo};
use serde_json::json;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    time::Duration,
};

#[test]
fn configured_server_fetch_artwork_and_audio_are_origin_bound() {
    if std::env::var_os("MUSICX_SERVER_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!(
            "musicx-server-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "configured_server_fetch_artwork_and_audio_are_origin_bound",
                "--nocapture",
            ])
            .env("MUSICX_SERVER_CHILD", "1")
            .env("MUSICX_SERVER_ROOT", &root)
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let base = server.clone();
    let worker = std::thread::spawn(move || {
        for _ in 0..5 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            let header_end = request.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
            let header = String::from_utf8(request[..header_end].to_vec()).unwrap();
            if header.starts_with("PROPFIND ") {
                while request.len() < header_end + 4 {
                    let n = socket.read(&mut buffer).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buffer[..n]);
                }
                assert_eq!(&request[header_end..header_end + 4], &[0, 255, 128, 1]);
            }
            let request = header;
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("x-token: fixture-key\r\n")
            );
            let path = request.split_whitespace().nth(1).unwrap();
            let body=match path {
                "/search"=>json!({"tracks":[{"id":"one","name":"Song","artists":[{"id":"artist","name":"Artist"}],"duration":42000,"imageUrl":format!("{base}/cover")}],"more":false}).to_string().into_bytes(),
                "/cover"=>b"image-bytes".to_vec(),
                "/binary"=>vec![0,255,128,1],
                "/audio"=>b"audio-bytes".to_vec(),
                "/redirect"=>{write!(socket,"HTTP/1.1 302 Redirect\r\nLocation: http://127.0.0.1:9/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();continue;}
                _=>panic!("unexpected path: {path}"),
            };
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        }
    });
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_SERVER_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("plugin.json"),json!({"id":"private.server","version":"1.0.0","permissions":{"network":["*"]},"settings":[{"key":"server","label":"Server","value":server},{"key":"apiKey","label":"API key"}],"requestHeaders":[{"hosts":["{setting:server}"],"kinds":["audio","image"],"headers":{"X-Token":"{setting:apiKey}"}}]}).to_string()).unwrap();
    fs::write(fixture.join("index.js"),r#"
export async function search(){return (await fetch(host.setting('server')+'/search',{headers:{'X-Token':host.setting('apiKey')}})).json()}
export function stream({quality}){host.storage.set('lastQuality',quality);return {url:host.setting('server')+'/audio'}}
export async function probe({url}){return (await fetch(url,{headers:{'X-Token':host.setting('apiKey')}})).status}
export function qualityProbe(){return host.storage.get('lastQuality')}
export async function binaryProbe(){const response=await fetch(host.setting('server')+'/binary',{method:'PROPFIND',headers:{'X-Token':host.setting('apiKey')},body:new Uint8Array([0,255,128,1])});return Array.from(new Uint8Array(await response.arrayBuffer()))}
"#).unwrap();
    mygo::install(&fixture).unwrap();
    assert!(
        mygo::search("private.server", "query", 0).is_err(),
        "a manifest default is not a user grant"
    );
    let settings = music_sources::configure(
        "mygo:private.server",
        std::collections::BTreeMap::from([
            ("server".into(), server.clone()),
            ("apiKey".into(), "fixture-key".into()),
        ]),
    )
    .unwrap();
    assert!(!settings.to_string().contains("fixture-key"));
    let search = mygo::search("private.server", "query", 0).unwrap();
    let track: Track = serde_json::from_value(search["tracks"][0].clone()).unwrap();
    let cover = track.cover_url.as_ref().unwrap();
    assert!(cover.starts_with(&server));
    assert!(cover.contains("musicx-plugin=private.server"));
    assert_eq!(
        assets::image("private.server", cover).unwrap(),
        b"image-bytes"
    );
    let media = mygo::resolve_with_quality(track, Quality::Hires).unwrap();
    assert!(media.uses_http);
    assert_eq!(
        reqwest::blocking::get(media.url.clone())
            .unwrap()
            .bytes()
            .unwrap()
            .as_ref(),
        b"audio-bytes"
    );
    assert_eq!(
        mygo::invoke("private.server", "qualityProbe", json!({})).unwrap(),
        "hires"
    );
    assert!(
        mygo::invoke(
            "private.server",
            "probe",
            json!({"url":"http://127.0.0.1:9/private"})
        )
        .is_err()
    );
    assert_eq!(
        mygo::invoke(
            "private.server",
            "probe",
            json!({"url":format!("{server}/redirect")})
        )
        .unwrap(),
        302
    );
    assert_eq!(
        mygo::invoke("private.server", "binaryProbe", json!({})).unwrap(),
        json!([0, 255, 128, 1])
    );
    worker.join().unwrap();
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    let other_url = format!("http://{}", other.local_addr().unwrap());
    assert!(mygo::invoke("private.server", "probe", json!({"url":other_url})).is_err());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = other.accept().unwrap();
        let mut bytes = [0u8; 1024];
        assert!(socket.read(&mut bytes).unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    music_sources::execute(MusicSourceAction::SetNetworkPermission {
        source: "mygo:private.server".into(),
        enabled: true,
    })
    .unwrap();
    assert!(
        music_sources::get_source("mygo:private.server")
            .unwrap()
            .network_granted
    );
    assert_eq!(
        mygo::invoke("private.server", "probe", json!({"url":other_url})).unwrap(),
        204
    );
    worker.join().unwrap();
    fs::write(
        root.join("state/mygo-plugins/private.server/extra.txt"),
        "modified",
    )
    .unwrap();
    assert!(
        !music_sources::get_source("mygo:private.server")
            .unwrap()
            .network_granted
    );
    assert!(
        mygo::invoke(
            "private.server",
            "probe",
            json!({"url":"http://127.0.0.1:9/private"})
        )
        .is_err()
    );
    let _ = MusicSourceAction::List;
}
