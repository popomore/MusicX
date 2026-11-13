//! A fresh helper must expose source management without installing a default source.
use musicx_core::{
    player::{Action, Reply, Status},
    protocol,
};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[test]
fn starts_without_plugins_or_a_catalog() {
    let directory = std::env::temp_dir().join(format!(
        "musicx-empty-startup-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "startup_child", "--nocapture"])
        .env("MUSICX_STARTUP_CHILD", "1")
        .env("MUSICX_DATA_DIR", &directory)
        .env("MUSICX_PORT", port.to_string())
        .env("MUSICX_PLUGIN_CATALOG_URL", "not-a-url")
        .output()
        .unwrap();
    if directory.exists() {
        fs::remove_dir_all(directory).unwrap();
    }
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn submit(_: Action, reply: Reply) {
    if let Some(reply) = reply {
        let _ = reply.send(Ok(Status::default()));
    }
}

#[test]
fn startup_child() {
    if std::env::var("MUSICX_STARTUP_CHILD").as_deref() != Ok("1") {
        return;
    }
    musicx_mcp::start(musicx_mcp::Services {
        submit,
        spectrum: protocol::spectrum::Spectrum::default,
        quit: || {},
        sources: &musicx_plugins::MYGO,
    })
    .unwrap();
    let token = musicx_mcp::read_token().unwrap();
    let mut stream = TcpStream::connect(protocol::address()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "GET /ui/music-sources HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(response.split_once("\r\n\r\n").unwrap().1, "[]");
    let directory = protocol::data_dir().unwrap();
    assert_eq!(
        fs::read_dir(directory.join("mygo-plugins"))
            .unwrap()
            .count(),
        0
    );
    assert!(!directory.join("mygo_source_catalog.json").exists());
}
