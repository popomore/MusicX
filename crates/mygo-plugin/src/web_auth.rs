//! Runs WebKit only in a short-lived helper, never in the audio or NSH process.
use crate::music_sources::LoginJob;
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

pub fn configure(executable: PathBuf) -> Result<()> {
    EXECUTABLE
        .set(executable)
        .map_err(|_| anyhow::anyhow!(tr!("Web sign-in component already configured")))
}

pub fn sign_in(
    mut request: Value,
    grants: &[String],
    name: &str,
    job: Arc<LoginJob>,
    deadline: Instant,
) -> Result<Value> {
    ensure!(
        cfg!(target_os = "macos"),
        tr!("Web sign-in is currently available on Mac only")
    );
    validate(&request, grants)?;
    request["name"] = json!(name.chars().take(80).collect::<String>());
    let executable = std::env::var_os("MUSICX_WEB_AUTH")
        .map(PathBuf::from)
        .or_else(|| EXECUTABLE.get().cloned())
        .context(tr!("MusicX web sign-in component missing"))?;
    ensure!(
        executable.is_file(),
        tr!("MusicX web sign-in component missing")
    );
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context(tr!("Could not open web sign-in window"))?;
    let result = (|| {
        let mut input = child
            .stdin
            .take()
            .context(tr!("Sign-in channel unavailable"))?;
        serde_json::to_writer(&mut input, &request)?;
        input.flush()?;
        drop(input);
        loop {
            if job.cancelled() || Instant::now() >= deadline {
                anyhow::bail!(tr!("Web sign-in canceled or timed out"));
            }
            if child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut text = String::new();
        child
            .stdout
            .take()
            .context(tr!("Sign-in channel closed"))?
            .take(32769)
            .read_to_string(&mut text)?;
        ensure!(text.len() <= 32768, tr!("Sign-in response is too large"));
        let value: Value =
            serde_json::from_str(&text).context(tr!("Web sign-in returned no account"))?;
        if value["cancelled"] == true {
            job.cancel();
            anyhow::bail!(tr!("Web sign-in canceled"));
        }
        ensure!(
            value.get("error").is_none(),
            tr!("Web sign-in incomplete; please try again")
        );
        let cookie = value["cookie"]
            .as_str()
            .context(tr!("Web sign-in incomplete"))?;
        ensure!(
            !cookie.is_empty() && cookie.len() <= 16384 && !cookie.contains(['\r', '\n']),
            tr!("Invalid sign-in data")
        );
        Ok(json!({"cookie":cookie}))
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}
fn validate(request: &Value, grants: &[String]) -> Result<()> {
    for key in ["url", "cookiesOf"] {
        let raw = request[key]
            .as_str()
            .context(tr!("Web sign-in URL missing"))?;
        let url = url::Url::parse(raw)?;
        ensure!(
            raw.len() <= 8192 && crate::mygo::permitted(&url, grants),
            tr!("Web sign-in URL is outside the source’s network permissions")
        );
    }
    let until = request["until"]
        .as_array()
        .context(tr!("Sign-in completion condition missing"))?;
    ensure!(
        !until.is_empty()
            && until.len() <= 16
            && until
                .iter()
                .all(|s| s.as_str().is_some_and(|s| !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)))),
        tr!("Invalid web sign-in completion condition")
    );
    ensure!(
        request["userAgent"]
            .as_str()
            .is_none_or(|s| s.len() <= 1024 && !s.contains(['\r', '\n'])),
        tr!("Invalid browser identifier")
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_cannot_return_another_sites_credentials() {
        let good = json!({"url":"https://music.163.com/#/login","cookiesOf":"https://music.163.com/","until":["MUSIC_U"]});
        assert!(validate(&good, &["music.163.com".into()]).is_ok());
        for bad in [
            "http://music.163.com/",
            "https://evil.example/",
            "https://user:pass@music.163.com/",
        ] {
            let mut request = good.clone();
            request["cookiesOf"] = json!(bad);
            assert!(validate(&request, &["music.163.com".into()]).is_err());
        }
    }
}
