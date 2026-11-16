//! Compatibility for MyGo's FFmpeg audio decode request on macOS.
use anyhow::{Context, Result, ensure};
use musicx_core::{protocol::tr, storage::random_hex};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, DirBuilder, File},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

const EXECUTABLES: [&str; 3] = [
    "ffmpeg",
    "/opt/homebrew/bin/ffmpeg",
    "/usr/local/bin/ffmpeg",
];
const MAX_WAV_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) fn supported(process: &[Value]) -> bool {
    cfg!(target_os = "macos")
        && process.iter().all(|name| {
            name.as_str()
                .is_some_and(|name| EXECUTABLES.contains(&name))
        })
}

#[derive(Default)]
pub(crate) struct Playback {
    audio: Option<(Url, Arc<DecodedAudio>)>,
}

pub(crate) struct DecodedAudio {
    directory: PathBuf,
}

impl DecodedAudio {
    fn create() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!("musicx-audio-{}", random_hex::<16>()));
        DirBuilder::new().mode(0o700).create(&directory)?;
        Ok(Self { directory })
    }

    fn path(&self) -> PathBuf {
        self.directory.join("audio.wav")
    }
}

impl Drop for DecodedAudio {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path());
        let _ = fs::remove_dir(&self.directory);
    }
}

#[derive(Deserialize)]
struct Spawn {
    program: String,
    args: Vec<String>,
}

struct Decode {
    input: Url,
    output: Url,
    headers: BTreeMap<String, String>,
}

impl Decode {
    fn parse(args: &[String], grants: &[String]) -> Result<Self> {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let [
            "-nostdin",
            "-hide_banner",
            "-nostats",
            "-loglevel",
            "info",
            "-user_agent",
            agent,
            "-headers",
            headers,
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "5",
            "-rw_timeout",
            "15000000",
            "-i",
            input,
            "-vn",
            "-map",
            "0:a:0",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
            "-listen",
            "1",
            output,
        ] = args.as_slice()
        else {
            anyhow::bail!(tr!("Unsupported FFmpeg audio request"));
        };
        let input = Url::parse(input)?;
        let output = Url::parse(output)?;
        ensure!(
            crate::mygo::permitted(&input, grants)
                && output.scheme() == "http"
                && output.host_str() == Some("127.0.0.1")
                && output.port().is_some()
                && output.path() == "/"
                && output.query().is_none()
                && output.fragment().is_none()
                && output.username().is_empty()
                && output.password().is_none(),
            tr!("Unsupported FFmpeg audio address")
        );
        let referer = headers
            .strip_prefix("Referer: ")
            .and_then(|value| value.strip_suffix("\r\n"))
            .context(tr!("Unsupported FFmpeg audio request"))?;
        for value in [*agent, referer] {
            ensure!(value.len() <= 1024, tr!("Unsupported FFmpeg audio request"));
            reqwest::header::HeaderValue::from_str(value)?;
        }
        Ok(Self {
            input,
            output,
            headers: BTreeMap::from([
                ("user-agent".into(), (*agent).into()),
                ("referer".into(), referer.into()),
            ]),
        })
    }
}

impl Playback {
    pub(crate) fn spawn(
        &mut self,
        text: &str,
        permissions: &Value,
        grants: &[String],
        deadline: Instant,
    ) -> Result<Value> {
        let Spawn { program, args } = serde_json::from_str(text)?;
        let allowed = permissions.as_array().is_some_and(|entries| {
            supported(entries) && entries.iter().any(|entry| entry.as_str() == Some(&program))
        });
        ensure!(allowed, tr!("FFmpeg is not allowed for this source"));
        let executable = if program == "ffmpeg" {
            EXECUTABLES[1..]
                .iter()
                .map(Path::new)
                .find(|path| path.is_file())
        } else {
            Some(Path::new(&program)).filter(|path| path.is_file())
        }
        .context(tr!("FFmpeg is not installed"))?;
        if args == ["-version"] {
            let status = run(Command::new(executable).arg("-version"), deadline)?;
            return Ok(json!({"status":status.code().unwrap_or(1)}));
        }

        let Decode {
            input,
            output,
            headers,
        } = Decode::parse(&args, grants)?;
        let (input, _lease) =
            crate::stream_proxy::register_restricted(input, headers, grants.to_vec())?;
        let decoded = Arc::new(DecodedAudio::create()?);
        let mut command = Command::new(executable);
        command
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-xerror",
                "-abort_on",
                "empty_output",
                "-protocol_whitelist",
                "http,tcp",
                "-http_proxy",
                "",
                "-rw_timeout",
                "5000000",
                "-f",
                "mov",
                "-enable_drefs",
                "0",
                "-i",
            ])
            .arg(input.as_str())
            .args(["-map", "0:a:0", "-c:a", "pcm_s16le", "-f", "wav", "-fs"])
            .arg(MAX_WAV_BYTES.to_string())
            .arg(decoded.path());
        ensure!(
            run(&mut command, deadline)?.success(),
            tr!("FFmpeg could not decode the audio")
        );
        ensure!(
            (45..MAX_WAV_BYTES).contains(&File::open(decoded.path())?.metadata()?.len()),
            tr!("FFmpeg did not produce audio within the size limit")
        );
        self.audio = Some((output, decoded));
        // The plugin uses FFmpeg's input line as its readiness signal.
        Ok(json!({"stderr": "Input #0"}))
    }

    pub(crate) fn take(&mut self, value: &Value) -> Option<Arc<DecodedAudio>> {
        let (output, _) = self.audio.as_ref()?;
        let url = value["url"].as_str().and_then(|url| Url::parse(url).ok())?;
        if url == *output && value["type"] == "wav" {
            return self.audio.take().map(|(_, audio)| audio);
        }
        None
    }
}

pub(crate) fn file_url(audio: &DecodedAudio) -> Result<Url> {
    Url::from_file_path(audio.path()).map_err(|()| anyhow::anyhow!(tr!("Invalid audio URL")))
}

fn run(command: &mut Command, deadline: Instant) -> Result<std::process::ExitStatus> {
    ensure!(
        Instant::now() < deadline,
        tr!("FFmpeg audio decoding timed out")
    );
    let mut child = command
        .env_remove("http_proxy")
        .env_remove("HTTP_PROXY")
        .env_remove("all_proxy")
        .env_remove("ALL_PROXY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context(tr!("Could not start FFmpeg"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!(tr!("FFmpeg audio decoding timed out"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_supports_the_declared_audio_decoder() {
        assert!(!supported(&[json!("sh")]));
        assert!(!supported(&[json!("ffmpeg"), json!("/tmp/ffmpeg")]));
        assert_eq!(
            supported(&EXECUTABLES.map(|program| json!(program))),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn validates_decoder_input_output_and_headers() {
        let mut args = [
            "-nostdin",
            "-hide_banner",
            "-nostats",
            "-loglevel",
            "info",
            "-user_agent",
            "Fixture",
            "-headers",
            "Referer: https://example.com/\r\n",
            "-reconnect",
            "1",
            "-reconnect_streamed",
            "1",
            "-reconnect_delay_max",
            "5",
            "-rw_timeout",
            "15000000",
            "-i",
            "https://cdn.example.com/audio.m4s",
            "-vn",
            "-map",
            "0:a:0",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
            "-listen",
            "1",
            "http://127.0.0.1:32123/",
        ]
        .map(str::to_owned);
        let grants = ["*.example.com".into()];
        let decoded = Decode::parse(&args, &grants).unwrap();
        assert_eq!(decoded.headers["referer"], "https://example.com/");
        for (index, replacement) in [
            (
                8,
                "Referer: https://example.com/\r\nAuthorization: fixture\r\n",
            ),
            (18, "file:///tmp/audio.m4s"),
            (18, "https://example.org/audio.m4s"),
            (18, "https://cdn.example.com:8082/audio.m4s"),
            (21, "0:v:0"),
            (28, "http://127.0.0.1:32123/other"),
        ] {
            let original = std::mem::replace(&mut args[index], replacement.into());
            assert!(Decode::parse(&args, &grants).is_err(), "{replacement}");
            args[index] = original;
        }
    }

    #[test]
    fn matches_only_its_own_wav_and_cleans_up_with_its_lease() {
        let audio = Arc::new(DecodedAudio::create().unwrap());
        let path = audio.path();
        fs::write(&path, b"fixture").unwrap();
        let output = "http://127.0.0.1:32123/";
        let mut playback = Playback {
            audio: Some((Url::parse(output).unwrap(), audio)),
        };
        assert!(playback.take(&json!({"url":output,"type":"mp3"})).is_none());
        assert!(
            playback
                .take(&json!({"url":"http://127.0.0.1:32124/","type":"wav"}))
                .is_none()
        );
        let lease = playback.take(&json!({"url":output,"type":"wav"})).unwrap();
        assert!(path.is_file());
        drop(lease);
        assert!(!path.exists());
    }
}
