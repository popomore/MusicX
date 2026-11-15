//! Adapter-owned acquisition, followed by the ordinary MusicX Media -> AVPlayer path.
//! Third-party remote-control exports and package files are never changed or invoked.
use anyhow::{Context, Result, ensure};
#[cfg(target_os = "macos")]
use musicx_core::protocol::AudioQuality;
use musicx_core::protocol::{Quality, Track, tr};
use musicx_plugin_api::Media;
#[cfg(target_os = "macos")]
use std::sync::Arc;
use std::sync::Mutex;
use zeroize::Zeroizing;

/// Scoped to one authorized account read. Neither plugin storage schemas nor a
/// public credential export are part of the MusicX source contract.
#[derive(Default)]
pub(crate) struct Capture(Mutex<Option<Zeroizing<String>>>);

impl Capture {
    pub(crate) fn candidate(
        url: &url::Url,
        method: &reqwest::Method,
        headers: &std::collections::HashMap<String, String>,
    ) -> Option<Zeroizing<String>> {
        if method != reqwest::Method::GET
            || url.scheme() != "https"
            || url.host_str() != Some("api.spotify.com")
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/v1/me"
            || url.query().is_some()
        {
            return None;
        }
        let header = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))?
            .1;
        let (scheme, token) = header.split_once(' ')?;
        (scheme.eq_ignore_ascii_case("bearer")
            && !token.is_empty()
            && token.len() <= 8192
            && token.bytes().all(|b| b.is_ascii_graphic()))
        .then(|| Zeroizing::new(token.to_owned()))
    }

    pub(crate) fn accept(&self, token: Option<Zeroizing<String>>, status: u16) {
        if status == 200
            && let Some(token) = token
        {
            *self.0.lock().unwrap() = Some(token);
        }
    }
}

pub(crate) fn supports(id: &str) -> bool {
    cfg!(target_os = "macos") && id == "spotify"
}

/// Native acquisition needs streaming consent in addition to the unmodified
/// plugin's Web API scopes. Spotify presents consent; identity and PKCE stay intact.
pub(crate) fn authorization_url(id: &str, address: &url::Url) -> url::Url {
    if !supports(id)
        || address.scheme() != "https"
        || address.host_str() != Some("accounts.spotify.com")
        || address.port_or_known_default() != Some(443)
        || !address.username().is_empty()
        || address.password().is_some()
        || address.path() != "/authorize"
        || !address
            .query_pairs()
            .any(|(key, value)| key == "response_type" && value == "code")
    {
        return address.clone();
    }
    let mut pairs: Vec<(String, String)> = address
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut scopes: Vec<String> = pairs
        .iter()
        .filter(|(key, _)| key == "scope")
        .flat_map(|(_, value)| value.split_whitespace().map(str::to_owned))
        .collect();
    if scopes.iter().any(|s| s == "streaming") {
        return address.clone();
    }
    scopes.push("streaming".into());
    pairs.retain(|(key, _)| key != "scope");
    pairs.push(("scope".into(), scopes.join(" ")));
    let mut address = address.clone();
    address.query_pairs_mut().clear().extend_pairs(pairs);
    address
}

pub(crate) fn resolve(track: Track, quality: Quality) -> Result<Media> {
    let source = track
        .source
        .as_ref()
        .context(tr!("Plugin track identifier missing"))?;
    ensure!(
        supports(&source.plugin),
        tr!("The source did not provide audio for MusicX playback")
    );
    #[cfg(target_os = "macos")]
    {
        // MyGo keeps its existing authentication and refresh behavior. Only a
        // successful /me read in this invocation can authorize native acquisition.
        let capture = Arc::new(Capture::default());
        ensure!(
            crate::mygo::audio_account(&source.plugin, capture.clone())?,
            tr!("Connect this source account before playing")
        );
        let token = capture.0.lock().unwrap().take().context(tr!(
            "This source account does not provide native audio access"
        ))?;
        let request = Request {
            access: token.to_string(),
            id: spotify_id(&source.id)?.to_owned(),
            high: quality != Quality::Standard,
        };
        let path = DECODER
            .get()
            .context(tr!("The MusicX audio decoder is unavailable"))?;
        let file = acquire(path, &request, std::time::Duration::from_secs(120), MAX_PCM)?;
        Ok(Media {
            url: file.url()?,
            track,
            uses_http: false,
            user_agent: None,
            // PCM is a transport format, not evidence of lossless source audio.
            quality: AudioQuality {
                requested: quality,
                format: Some("PCM".into()),
                sample_rate_hz: Some(44_100),
                bit_depth: Some(16),
                ..Default::default()
            },
            gain_db: None,
            peak: None,
            lease: Some(file),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = quality;
        anyhow::bail!(tr!("The MusicX audio decoder is unavailable"))
    }
}

#[cfg(target_os = "macos")]
mod spotify;
#[cfg(target_os = "macos")]
pub use spotify::worker;

#[cfg(target_os = "macos")]
const MAX_PCM: u64 = 256 * 1024 * 1024;
#[cfg(target_os = "macos")]
static DECODER: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
pub fn configure(executable: std::path::PathBuf) -> Result<()> {
    ensure!(
        executable.is_absolute() && executable.is_file(),
        tr!("The MusicX audio decoder is unavailable")
    );
    DECODER
        .set(executable)
        .map_err(|_| anyhow::anyhow!(tr!("The MusicX audio decoder is already configured")))
}

#[cfg(target_os = "macos")]
#[derive(serde::Serialize, serde::Deserialize)]
struct Request {
    access: String,
    id: String,
    high: bool,
}
#[cfg(target_os = "macos")]
impl Drop for Request {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.access.zeroize();
    }
}

#[cfg(target_os = "macos")]
fn spotify_id(id: &str) -> Result<&str> {
    let id = id.strip_prefix("spotify:track:").unwrap_or(id);
    ensure!(
        id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()),
        tr!("This source track cannot be decoded by MusicX")
    );
    Ok(id)
}

#[cfg(target_os = "macos")]
fn wav_header(bytes: u32) -> [u8; 44] {
    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&2u16.to_le_bytes());
    header[24..28].copy_from_slice(&44_100u32.to_le_bytes());
    header[28..32].copy_from_slice(&176_400u32.to_le_bytes());
    header[32..34].copy_from_slice(&4u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&bytes.to_le_bytes());
    header
}

/// The child has no UI, audio-device backend, public RPC surface or disk token
/// cache. Its lifetime and output file belong to the resolving Media lease.
#[cfg(target_os = "macos")]
fn acquire(
    executable: &std::path::Path,
    request: &Request,
    timeout: std::time::Duration,
    maximum: u64,
) -> Result<Arc<crate::plugin_files::AudioFile>> {
    use std::{
        io::{Seek, SeekFrom, Write},
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    ensure!(
        ACTIVE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < 2)
                .then_some(n + 1))
            .is_ok(),
        tr!("Too many audio requests; try again")
    );
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::Release);
        }
    }
    let _permit = Permit;
    let (file, mut output) = crate::plugin_files::AudioFile::create("wav")?;
    output.write_all(&wav_header(0))?;
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Child(
        Command::new(executable)
            .arg("--decode-audio")
            .stdin(Stdio::piped())
            .stdout(output.try_clone()?)
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .context(tr!("The MusicX audio decoder is unavailable"))?,
    );
    let input = Zeroizing::new(serde_json::to_vec(request)?);
    // Small bounded IPC cannot block on a running decoder's stdin buffer.
    child
        .0
        .stdin
        .take()
        .context(tr!("The MusicX audio decoder is unavailable"))?
        .write_all(&input)?;
    let deadline = Instant::now() + timeout;
    loop {
        ensure!(
            output.metadata()?.len() <= maximum + 44,
            tr!("Generated audio exceeds 256 MiB")
        );
        if let Some(status) = child.0.try_wait()? {
            ensure!(
                status.code() != Some(2),
                tr!("Reconnect this source and approve audio playback access")
            );
            ensure!(
                status.success(),
                tr!("The source could not provide audio for MusicX playback")
            );
            break;
        }
        ensure!(
            Instant::now() < deadline,
            tr!("Audio acquisition timed out")
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let bytes = output.metadata()?.len().saturating_sub(44);
    ensure!(
        bytes > 0 && bytes <= maximum && bytes.is_multiple_of(4),
        tr!("The source returned incomplete audio")
    );
    output.seek(SeekFrom::Start(0))?;
    output.write_all(&wav_header(bytes as u32))?;
    output.flush()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn native_playback_scope_preserves_plugin_oauth_identity_and_pkce() {
        let original: url::Url = "https://accounts.spotify.com/authorize?client_id=fixture&response_type=code&redirect_uri=http%3A%2F%2F127.0.0.1%3A43110%2Fcallback&state=a%2Bb&code_challenge=challenge&code_challenge_method=S256&scope=user-read-playback-state%20user-library-read".parse().unwrap();
        let authorized = authorization_url("spotify", &original);
        for (key, value) in original.query_pairs().filter(|(k, _)| k != "scope") {
            assert_eq!(
                authorized.query_pairs().find(|(k, _)| k == &key).unwrap().1,
                value
            );
        }
        let scope = authorized
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(
            scope,
            "user-read-playback-state user-library-read streaming"
        );
        assert_eq!(authorization_url("spotify", &authorized), authorized);
        assert_eq!(authorization_url("another-source", &original), original);
        for address in [
            "http://accounts.spotify.com/authorize?response_type=code",
            "https://accounts.spotify.com.evil.test/authorize?response_type=code",
            "https://accounts.spotify.com/api/token",
            "https://accounts.spotify.com/authorize?response_type=token",
        ] {
            let address: url::Url = address.parse().unwrap();
            assert_eq!(authorization_url("spotify", &address), address);
        }
    }
    #[test]
    fn credentials_are_scoped_to_a_successful_spotify_account_read() {
        let headers = std::collections::HashMap::from([(
            "Authorization".into(),
            "Bearer private-fixture".into(),
        )]);
        for address in [
            "http://api.spotify.com/v1/me",
            "https://api.spotify.com.evil.test/v1/me",
            "https://api.spotify.com/v1/me/player",
            "https://api.spotify.com/v1/me?token=bad",
            "https://api.spotify.com:444/v1/me",
            "https://user@api.spotify.com/v1/me",
        ] {
            assert!(
                Capture::candidate(&address.parse().unwrap(), &reqwest::Method::GET, &headers)
                    .is_none()
            );
        }
        let url = "https://api.spotify.com/v1/me".parse().unwrap();
        assert!(Capture::candidate(&url, &reqwest::Method::POST, &headers).is_none());
        let capture = Capture::default();
        capture.accept(
            Capture::candidate(&url, &reqwest::Method::GET, &headers),
            401,
        );
        assert!(capture.0.lock().unwrap().is_none());
        capture.accept(
            Capture::candidate(&url, &reqwest::Method::GET, &headers),
            200,
        );
        assert_eq!(
            capture.0.lock().unwrap().take().unwrap().as_str(),
            "private-fixture"
        );
        assert!(Capture::default().0.lock().unwrap().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn acquisition_owns_audio_and_stops_failed_or_timed_out_decoders() {
        use std::{
            fs,
            os::unix::fs::{MetadataExt, PermissionsExt},
            process::Command,
            time::Duration,
        };
        if std::env::var_os("MUSICX_AUDIO_CHILD").is_none() {
            let root = std::env::temp_dir().join(format!(
                "musicx-acquisition-{}",
                musicx_core::storage::random_hex::<8>()
            ));
            let result = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_audio::tests::acquisition_owns_audio_and_stops_failed_or_timed_out_decoders", "--nocapture"])
                .env("MUSICX_AUDIO_CHILD", "1").env("MUSICX_DATA_DIR", &root)
                .env("MUSICX_LANGUAGE", "en").output().unwrap();
            if root.exists() {
                fs::remove_dir_all(root).unwrap();
            }
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let root = musicx_core::storage::state_dir().unwrap();
        let program = root.join("fixture-decoder");
        let script = |body: &str| {
            fs::write(&program, format!("#!/usr/bin/env python3\nimport sys,json,time,os,struct,math\nr=json.load(sys.stdin)\nassert r['access']=='private-fixture'\nassert len(sys.argv)==2 and sys.argv[1]=='--decode-audio'\n{body}\n")).unwrap();
            fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        };
        let request = Request {
            access: "private-fixture".into(),
            id: "0123456789012345678901".into(),
            high: false,
        };
        script(
            "sys.stdout.buffer.write(b''.join(struct.pack('<hh',int(1000*math.sin(i/10)),int(1000*math.sin(i/10))) for i in range(44100*3)))",
        );
        let audio = acquire(&program, &request, Duration::from_secs(4), MAX_PCM).unwrap();
        let path = audio.url().unwrap().to_file_path().unwrap();
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 44 + 3 * 176_400);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            3 * 176_400
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        let retained = audio.clone();
        drop(audio);
        assert!(path.exists());
        // Opt-in local acceptance artifact; the default test leaves no media.
        if let Some(export) = std::env::var_os("MUSICX_AUDIO_QA_WAV") {
            fs::copy(&path, export).unwrap();
        }
        drop(retained);
        assert!(!path.exists());
        for body in [
            "sys.stdout.buffer.write(b'partial');sys.exit(1)",
            "sys.stdout.buffer.write(b'x')",
            "sys.stdout.buffer.write(b'x'*2048)",
            "time.sleep(10)",
            "sys.exit(2)",
        ] {
            script(body);
            let started = std::time::Instant::now();
            let error = acquire(&program, &request, Duration::from_millis(150), 1024)
                .err()
                .expect("failed acquisition");
            if body == "sys.exit(2)" {
                assert!(error.to_string().contains("Reconnect this source"));
            }
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(
                fs::read_dir(root.join("generated-audio")).unwrap().count(),
                0
            );
        }
        assert!(spotify_id("spotify:track:0123456789012345678901").is_ok());
        for id in [
            "spotify:episode:0123456789012345678901",
            "../token",
            "",
            "https://evil.test/",
        ] {
            assert!(spotify_id(id).is_err());
        }
    }
}
