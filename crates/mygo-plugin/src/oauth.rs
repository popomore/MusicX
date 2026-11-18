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
                ensure!(!started.swap(true, Ordering::SeqCst), tr!("This sign-in is already waiting for a callback"));
                let job = login.clone().context(tr!("Start source sign-in first"))?;
                ensure!(!network.is_empty(), tr!("OAuth callback requires network permission"));
                let options: Options = serde_json::from_str(&input)?;
                ensure!(options.path.len() <= 512 && (options.path.is_empty() || options.path.starts_with('/')), tr!("Invalid callback path"));
                ensure!(options.page.len() <= 65536, tr!("Callback prompt is too long"));
                let listener = TcpListener::bind(("127.0.0.1", options.port)).context(tr!("Could not listen on the sign-in callback port"))?;
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
                                    let text = if options.page.is_empty() { tr!("Sign-in complete. You can close this page.") } else { &options.page };
                                    let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{text}", text.len());
                                    let mut query = serde_json::Map::new();
                                    for (key, value) in url.query_pairs() { query.entry(key.into_owned()).or_insert(json!(value)); }
                                    Ok(Some(Value::Object(query)))
                                })();
                                if let Ok(Some(query)) = connection { return Ok(query); }
                            }
                            std::thread::sleep(Duration::from_millis(25));
                        }
                        anyhow::bail!(tr!("Sign-in callback canceled or timed out"))
                    };
                    let received = receive().map_err(|_| tr!("Sign-in callback incomplete").into());
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
                    let job = job.as_ref().context(tr!("Start source sign-in first"))?;
                    let url = url::Url::parse(&address)?;
                    ensure!(
                        crate::mygo::permitted(&url, &grants),
                        tr!("Sign-in page is outside the source’s network permissions")
                    );
                    ensure!(!job.cancelled(), tr!("Sign-in canceled"));
                    #[cfg(target_os = "macos")]
                    {
                        ensure!(
                            std::process::Command::new("/usr/bin/open")
                                .arg(url.as_str())
                                .status()?
                                .success(),
                            tr!("Could not open sign-in page")
                        );
                        Ok(())
                    }
                    #[cfg(not(target_os = "macos"))]
                    anyhow::bail!(tr!(
                        "Browser callback sign-in is not supported on this platform"
                    ));
                };
                open()
                    .map_or_else(
                        |_| json!({"error":tr!("Could not open the source sign-in page")}),
                        |_| json!({}),
                    )
                    .to_string()
            })?,
        )?;
        Ok(Self { stop })
    }
}
