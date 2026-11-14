//! Isolated, one-operation-per-process smoke test driver for third-party plugins.
//! Always set MUSICX_DATA_DIR to a disposable directory before running this example.
use musicx_plugins::{music_sources, mygo};
use serde_json::{Value, json};
use std::{path::Path, time::Instant};

fn execute(args: &[String]) -> anyhow::Result<Value> {
    anyhow::ensure!(
        std::env::var_os("MUSICX_DATA_DIR").is_some(),
        "Set MUSICX_DATA_DIR to an isolated test directory"
    );
    match args.first().map(String::as_str) {
        Some("install-local") => {
            let id = mygo::install(Path::new(&args[1]))?;
            Ok(serde_json::to_value(music_sources::get_source(&format!(
                "mygo:{id}"
            ))?)?)
        }
        Some("install-repo") => Ok(serde_json::to_value(music_sources::install(&args[1])?)?),
        Some("describe") => Ok(serde_json::to_value(music_sources::get_source(&format!(
            "mygo:{}",
            args[1]
        ))?)?),
        Some("action") => music_sources::execute(serde_json::from_str(&args[1])?),
        Some("search") => mygo::search_kind(
            &args[1],
            &args[2],
            args.get(3).map(String::as_str).unwrap_or("tracks"),
            args.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
        ),
        Some("invoke") => mygo::invoke(
            &args[1],
            &args[2],
            serde_json::from_str(args.get(3).map(String::as_str).unwrap_or("{}"))?,
        ),
        Some("resolve") => {
            let track = serde_json::from_str(&args[1])?;
            let quality = match args.get(3).map(String::as_str) {
                Some("high") => musicx_core::protocol::Quality::High,
                Some("lossless") => musicx_core::protocol::Quality::Lossless,
                Some("hires") => musicx_core::protocol::Quality::Hires,
                _ => musicx_core::protocol::Quality::Standard,
            };
            let media = mygo::resolve_with_quality(track, quality)?;
            let probe = if args.get(2).is_some_and(|v| v == "probe" || v == "decode") {
                let ffprobe = ["/opt/homebrew/bin/ffprobe", "/usr/local/bin/ffprobe"]
                    .into_iter()
                    .find(|p| Path::new(p).exists())
                    .unwrap_or("ffprobe");
                let mut command = std::process::Command::new(ffprobe);
                command.args([
                    "-v",
                    "error",
                    "-rw_timeout",
                    "15000000",
                    "-read_intervals",
                    "%+#8",
                    "-show_entries",
                    "format=duration,format_name:stream=codec_name,sample_rate,channels",
                    "-of",
                    "json",
                ]);
                if let Some(agent) = &media.user_agent {
                    command.args(["-user_agent", agent]);
                }
                let output = command.arg(media.url.as_str()).output()?;
                let mut value = json!({"ok":output.status.success(),"data":serde_json::from_slice::<Value>(&output.stdout).unwrap_or(Value::Null),"error":String::from_utf8_lossy(&output.stderr)});
                if args.get(2).is_some_and(|v| v == "decode") && output.status.success() {
                    let ffmpeg = Path::new(ffprobe).with_file_name("ffmpeg");
                    let mut command = std::process::Command::new(ffmpeg);
                    command.args(["-nostdin", "-v", "error", "-rw_timeout", "15000000"]);
                    if let Some(agent) = &media.user_agent {
                        command.args(["-user_agent", agent]);
                    }
                    let decoded = command
                        .args([
                            "-i",
                            media.url.as_str(),
                            "-t",
                            "2",
                            "-vn",
                            "-f",
                            "null",
                            "-",
                        ])
                        .output()?;
                    value["decode_ok"] = json!(decoded.status.success());
                    value["decode_error"] = json!(String::from_utf8_lossy(&decoded.stderr));
                }
                value
            } else {
                Value::Null
            };
            Ok(
                json!({"track":media.track,"url_host":media.url.host_str(),"url_scheme":media.url.scheme(),"uses_http":media.uses_http,"quality":media.quality,"gain_db":media.gain_db,"peak":media.peak,"probe":probe}),
            )
        }
        _ => anyhow::bail!(
            "Expected install-local/install-repo/describe/action/search/invoke/resolve"
        ),
    }
}

fn main() {
    let start = Instant::now();
    let args: Vec<_> = std::env::args().skip(1).collect();
    let result = match std::panic::catch_unwind(|| execute(&args)) {
        Ok(Ok(data)) => json!({"ok":true,"elapsed_ms":start.elapsed().as_millis(),"data":data}),
        Ok(Err(error)) => {
            json!({"ok":false,"elapsed_ms":start.elapsed().as_millis(),"error":format!("{error:#}"),"plugin_code":mygo::error_code(&error)})
        }
        Err(_) => json!({"ok":false,"elapsed_ms":start.elapsed().as_millis(),"error":"host panic"}),
    };
    println!("{}", serde_json::to_string(&result).unwrap());
    if result["ok"] != true {
        std::process::exit(1);
    }
}
