use crate::music_sources::LoginJob;
use anyhow::{Context, Result, ensure};
use musicx_core::protocol::tr;
use rquickjs::{Ctx, Function};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Options {
    port: u16,
    #[serde(default)]
    path: String,
    #[serde(default)]
    timeout: u64,
    #[serde(default)]
    page: String,
}
type Answer = Arc<Mutex<Option<std::result::Result<Value, String>>>>;
pub(crate) struct Bindings {
    stop: Arc<AtomicBool>,
}
impl Drop for Bindings {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
impl Bindings {
    pub fn install(
        ctx: &Ctx<'_>,
        job: Option<Arc<LoginJob>>,
        grants: Vec<String>,
        deadline: Instant,
    ) -> rquickjs::Result<Self> {
        let answer: Answer = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));
        let result = answer.clone();
        let cancelled = stop.clone();
        let login = job.clone();
        let network = grants.clone();
        ctx.globals().set("__redirectStart", Function::new(ctx.clone(), move |input: String| {
            let run = || -> Result<()> {
                ensure!(!started.swap(true, Ordering::SeqCst), tr!("本次登录已经等待回调"));
                let job = login.clone().context(tr!("请先发起来源登录"))?;
                ensure!(!network.is_empty(), tr!("OAuth 回调需要网络权限"));
                let options: Options = serde_json::from_str(&input)?;
                ensure!(options.path.len() <= 512 && (options.path.is_empty() || options.path.starts_with('/')), tr!("无效的回调路径"));
                ensure!(options.page.len() <= 65536, tr!("回调提示过长"));
                let listener = TcpListener::bind(("127.0.0.1", options.port)).context(tr!("无法监听登录回调端口"))?;
                listener.set_nonblocking(true)?;
                let until = if options.timeout == 0 { deadline } else { deadline.min(Instant::now() + Duration::from_millis(options.timeout.min(300000))) };
                let answer = result.clone(); let stop = cancelled.clone();
                std::thread::spawn(move || {
                    let receive = || -> Result<Value> {
                        while Instant::now() < until && !stop.load(Ordering::SeqCst) && !job.cancelled() {
                            if let Ok((mut socket, _)) = listener.accept() {
                                // A speculative or malformed connection must not consume
                                // the callback listener intended for the real redirect.
                                let connection = (|| -> Result<Option<Value>> {
                                    socket.set_nonblocking(false)?;
                                    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
                                    socket.set_write_timeout(Some(Duration::from_millis(500)))?;
                                    let mut line = String::new();
                                    BufReader::new(socket.try_clone()?.take(8192)).read_line(&mut line)?;
                                    let parts: Vec<_> = line.split_whitespace().collect();
                                    if !line.ends_with('\n') || parts.len() != 3 || parts[0] != "GET" || !parts[1].starts_with('/') { return Ok(None); }
                                    let url = url::Url::parse(&format!("http://127.0.0.1{}", parts[1]))?;
                                    if !options.path.is_empty() && url.path() != options.path {
                                        let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
                                        return Ok(None);
                                    }
                                    let text = if options.page.is_empty() { tr!("已完成登录，可以关闭此页面。") } else { &options.page };
                                    let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{text}", text.len());
                                    let mut query = serde_json::Map::new();
                                    for (key, value) in url.query_pairs() { query.entry(key.into_owned()).or_insert(json!(value)); }
                                    Ok(Some(Value::Object(query)))
                                })();
                                if let Ok(Some(query)) = connection { return Ok(query); }
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        anyhow::bail!(tr!("登录回调已取消或超时"))
                    };
                    let received = receive().map_err(|_| tr!("登录回调未完成").into());
                    if let Ok(mut answer) = answer.lock() { *answer = Some(received); }
                });
                Ok(())
            };
            run().map_or_else(|e| json!({"error":e.to_string()}), |_| json!({})).to_string()
        })?)?;
        ctx.globals().set(
            "__redirectPoll",
            Function::new(ctx.clone(), move || {
                match answer.lock().ok().and_then(|mut a| a.take()) {
                    Some(Ok(value)) => json!({"value":value}),
                    Some(Err(error)) => json!({"error":error}),
                    None => json!({"pending":true}),
                }
                .to_string()
            })?,
        )?;
        ctx.globals().set(
            "__openURL",
            Function::new(ctx.clone(), move |address: String| {
                let open = || -> Result<()> {
                    let job = job.as_ref().context(tr!("请先发起来源登录"))?;
                    let url = url::Url::parse(&address)?;
                    ensure!(
                        crate::mygo::permitted(&url, &grants),
                        tr!("登录页面不在来源网络权限内")
                    );
                    ensure!(!job.cancelled(), tr!("登录已取消"));
                    #[cfg(target_os = "macos")]
                    {
                        ensure!(
                            std::process::Command::new("/usr/bin/open")
                                .arg(url.as_str())
                                .status()?
                                .success(),
                            tr!("无法打开登录页面")
                        );
                        Ok(())
                    }
                    #[cfg(not(target_os = "macos"))]
                    anyhow::bail!(tr!("此平台尚未接入浏览器回调登录"));
                };
                open()
                    .map_or_else(
                        |_| json!({"error":tr!("无法打开来源登录页")}),
                        |_| json!({}),
                    )
                    .to_string()
            })?,
        )?;
        Ok(Self { stop })
    }
}
