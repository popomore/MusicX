//! Runs WebKit only in a short-lived helper, never in the audio or NSH process.
use crate::music_sources::LoginJob;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

pub const APP_NAME: &str = "MusicX 网页登录";
static EXECUTABLE: OnceLock<PathBuf> = OnceLock::new();

pub fn configure(executable: PathBuf) -> Result<()> {
    EXECUTABLE
        .set(executable)
        .map_err(|_| anyhow::anyhow!("网页登录组件已经配置"))
}

pub fn sign_in(
    mut request: Value,
    grants: &[String],
    name: &str,
    job: Arc<LoginJob>,
    deadline: Instant,
) -> Result<Value> {
    ensure!(cfg!(target_os = "macos"), "网页登录目前仅支持Mac");
    validate(&request, grants)?;
    request["name"] = json!(name.chars().take(80).collect::<String>());
    let executable = std::env::var_os("MUSICX_WEB_AUTH")
        .map(PathBuf::from)
        .or_else(|| EXECUTABLE.get().cloned())
        .context("缺少 MusicX 网页登录组件")?;
    ensure!(executable.is_file(), "缺少 MusicX 网页登录组件");
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("无法打开网页登录窗口")?;
    let result = (|| {
        let mut input = child.stdin.take().context("登录通道不可用")?;
        serde_json::to_writer(&mut input, &request)?;
        input.flush()?;
        drop(input);
        loop {
            if job.cancelled() || Instant::now() >= deadline {
                anyhow::bail!("网页登录已取消或超时");
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
            .context("登录通道已关闭")?
            .take(32769)
            .read_to_string(&mut text)?;
        ensure!(text.len() <= 32768, "登录返回数据过大");
        let value: Value = serde_json::from_str(&text).context("网页登录未返回账号")?;
        if value["cancelled"] == true {
            job.cancel();
            anyhow::bail!("网页登录已取消");
        }
        ensure!(value.get("error").is_none(), "网页登录未完成，请重试");
        let cookie = value["cookie"].as_str().context("网页登录未完成")?;
        ensure!(
            !cookie.is_empty() && cookie.len() <= 16384 && !cookie.contains(['\r', '\n']),
            "无效登录数据"
        );
        Ok(json!({"cookie":cookie}))
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}
fn validate(request: &Value, grants: &[String]) -> Result<()> {
    for key in ["url", "cookiesOf"] {
        let raw = request[key].as_str().context("缺少网页登录地址")?;
        let url = url::Url::parse(raw)?;
        ensure!(
            raw.len() <= 8192 && crate::mygo::permitted(&url, grants),
            "网页登录地址超出来源网络权限"
        );
    }
    let until = request["until"].as_array().context("缺少登录完成条件")?;
    ensure!(
        !until.is_empty()
            && until.len() <= 16
            && until
                .iter()
                .all(|s| s.as_str().is_some_and(|s| !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c)))),
        "无效登录完成条件"
    );
    ensure!(
        request["userAgent"]
            .as_str()
            .is_none_or(|s| s.len() <= 1024 && !s.contains(['\r', '\n'])),
        "无效浏览器标识"
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
