//! Range-capable loopback transport for AVPlayer's unsupported request headers.
use anyhow::{Context, Result, ensure};
use reqwest::{Url, blocking::Client, redirect::Policy};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct Stream {
    url: Url,
    headers: BTreeMap<String, String>,
    used: Instant,
}
struct Proxy {
    port: u16,
    streams: Arc<Mutex<BTreeMap<String, Stream>>>,
}
static PROXY: OnceLock<std::result::Result<Proxy, String>> = OnceLock::new();

fn start() -> Result<Proxy> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let streams = Arc::new(Mutex::new(BTreeMap::<String, Stream>::new()));
    let entries = streams.clone();
    std::thread::spawn(move || {
        let active = Arc::new(AtomicUsize::new(0));
        for incoming in listener.incoming() {
            let Ok(mut socket) = incoming else {
                break;
            };
            if active.fetch_add(1, Ordering::SeqCst) >= 8 {
                active.fetch_sub(1, Ordering::SeqCst);
                let _ = socket.write_all(
                    b"HTTP/1.1 503 Busy\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                );
                continue;
            }
            let entries = entries.clone();
            let active = active.clone();
            std::thread::spawn(move || {
                let _ = serve(socket, &entries);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    Ok(Proxy { port, streams })
}

pub(crate) fn register(url: Url, headers: BTreeMap<String, String>) -> Result<Url> {
    let proxy = PROXY
        .get_or_init(|| start().map_err(|_| "无法启动音频传输".into()))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let key = musicx_core::storage::random_hex::<32>();
    let mut streams = proxy
        .streams
        .lock()
        .map_err(|_| anyhow::anyhow!("音频传输不可用"))?;
    while streams.len() >= 64 {
        if let Some(oldest) = streams
            .iter()
            .min_by_key(|(_, s)| s.used)
            .map(|(k, _)| k.clone())
        {
            streams.remove(&oldest);
        }
    }
    streams.insert(
        key.clone(),
        Stream {
            url,
            headers,
            used: Instant::now(),
        },
    );
    Ok(Url::parse(&format!(
        "http://127.0.0.1:{}/{key}",
        proxy.port
    ))?)
}

fn serve(mut socket: TcpStream, streams: &Mutex<BTreeMap<String, Stream>>) -> Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(socket.try_clone()?.take(16384));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let parts: Vec<_> = line.split_whitespace().collect();
    ensure!(
        parts.len() == 3 && matches!(parts[0], "GET" | "HEAD"),
        "无效请求"
    );
    let head = parts[0] == "HEAD";
    let key = parts[1].strip_prefix('/').context("无效地址")?.to_owned();
    let mut range = None;
    loop {
        line.clear();
        ensure!(reader.read_line(&mut line)? > 0, "请求头不完整");
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.trim_end().split_once(':')
            && name.eq_ignore_ascii_case("range")
        {
            let value = value.trim();
            ensure!(
                value.starts_with("bytes=") && value.len() < 128,
                "无效音频范围"
            );
            range = Some(value.to_owned());
        }
    }
    let Some(mut stream) = ({
        let mut map = streams
            .lock()
            .map_err(|_| anyhow::anyhow!("音频传输不可用"))?;
        map.get_mut(&key).map(|s| {
            s.used = Instant::now();
            s.clone()
        })
    }) else {
        socket.write_all(
            b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        )?;
        return Ok(());
    };
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(Policy::none())
        .build()?;
    for _ in 0..5 {
        let mut request = if head {
            client.head(stream.url.clone())
        } else {
            client.get(stream.url.clone())
        };
        for (name, value) in &stream.headers {
            request = request.header(name, value);
        }
        if let Some(value) = &range {
            request = request.header("Range", value);
        }
        let mut response = request.send().context("音频下载失败")?;
        if response.status().is_redirection() {
            let next = stream.url.join(
                response
                    .headers()
                    .get("location")
                    .context("音频跳转无地址")?
                    .to_str()?,
            )?;
            ensure!(
                musicx_core::metadata::public_url(next.as_str()).is_some(),
                "音频跳转地址不支持"
            );
            if next.host_str() != stream.url.host_str() {
                stream
                    .headers
                    .retain(|k, _| k.eq_ignore_ascii_case("user-agent"));
            }
            stream.url = next;
            continue;
        }
        write!(
            socket,
            "HTTP/1.1 {}\r\nConnection: close\r\n",
            response.status()
        )?;
        for name in [
            "content-type",
            "content-length",
            "content-range",
            "accept-ranges",
        ] {
            if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
                write!(socket, "{name}: {value}\r\n")?;
            }
        }
        socket.write_all(b"\r\n")?;
        if !head {
            std::io::copy(&mut response, &mut socket)?;
        }
        return Ok(());
    }
    anyhow::bail!("音频跳转过多")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forwards_private_headers_and_seek_ranges() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = upstream.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = upstream.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            let request = request.to_ascii_lowercase();
            assert!(request.contains("authorization: fixture"));
            assert!(request.contains("referer: https://example.com/"));
            assert!(request.contains("range: bytes=2-4"));
            socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 2-4/6\r\n\r\ncde").unwrap();
        });
        let url = register(
            format!("http://{address}/audio").parse().unwrap(),
            BTreeMap::from([
                ("authorization".into(), "fixture".into()),
                ("referer".into(), "https://example.com/".into()),
            ]),
        )
        .unwrap();
        let response = Client::new()
            .get(url)
            .header("Range", "bytes=2-4")
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 206);
        assert_eq!(response.headers()["content-range"], "bytes 2-4/6");
        assert_eq!(response.text().unwrap(), "cde");
        server.join().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires the macOS AVFoundation runtime and Swift"]
    fn native_avplayer_decodes_and_seeks_proxy_audio() {
        use std::sync::atomic::AtomicBool;
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let ended = stop.clone();
        let mut wav = b"RIFF".to_vec();
        let length = 16000u32 * 2 * 4;
        wav.extend((length + 36).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(16000u32.to_le_bytes());
        wav.extend(32000u32.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(length.to_le_bytes());
        wav.resize(44 + length as usize, 0);
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let server = std::thread::spawn(move || {
            while !ended.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    let read = reader.read_line(&mut line).unwrap_or(0);
                    if read == 0 || line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: native-fixture")
                );
                count.fetch_add(1, Ordering::SeqCst);
                let range = request.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                });
                let (start, end) = range
                    .as_deref()
                    .and_then(|s| s.split_once('-'))
                    .map(|(a, b)| {
                        (
                            a.parse::<usize>().unwrap_or(0),
                            b.parse::<usize>()
                                .unwrap_or(wav.len() - 1)
                                .min(wav.len() - 1),
                        )
                    })
                    .unwrap_or((0, wav.len() - 1));
                let response = format!(
                    "HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Type: audio/wav\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{end}/{}\r\n\r\n",
                    end - start + 1,
                    wav.len()
                );
                let _ = socket.write_all(response.as_bytes());
                if !request.starts_with("HEAD ") {
                    let _ = socket.write_all(&wav[start..=end]);
                }
            }
        });
        let url = register(
            format!("http://{address}/fixture.wav").parse().unwrap(),
            BTreeMap::from([("authorization".into(), "native-fixture".into())]),
        )
        .unwrap();
        let script = std::env::temp_dir().join(format!(
            "musicx-native-audio-{}.swift",
            musicx_core::storage::random_hex::<8>()
        ));
        std::fs::write(&script, r#"
import AVFoundation
import Foundation
let player = AVPlayer(url: URL(string: CommandLine.arguments[1])!)
player.isMuted = true
player.play()
var sought = false
var seeked = false
let deadline = Date().addingTimeInterval(12)
while Date() < deadline {
    RunLoop.current.run(until: Date().addingTimeInterval(0.05))
    if player.currentItem?.status == .failed { fputs("AVPlayer rejected proxy audio\n", stderr); exit(2) }
    if player.currentTime().seconds > 0.15 && !sought {
        sought = true
        player.seek(to: CMTime(seconds: 2, preferredTimescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero) { done in seeked = done }
    }
    if seeked && player.currentTime().seconds > 2.1 { print("AVPlayer decoded, advanced and sought through the authenticated proxy (muted)"); exit(0) }
}
fputs("AVPlayer decode/seek timed out\n", stderr)
exit(3)
"#).unwrap();
        let output = std::process::Command::new("/usr/bin/swift")
            .arg(&script)
            .arg(url.as_str())
            .output()
            .unwrap();
        stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
        let _ = std::fs::remove_file(script);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(requests.load(Ordering::SeqCst) > 0);
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    }
}
