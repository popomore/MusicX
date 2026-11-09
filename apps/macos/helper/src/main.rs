use musicx_mcp as mcp;
use musicx_platform_apple::{desktop, player};
use musicx_plugins::{MYGO, WEB_AUTH_APP_NAME, mygo, netease};

use anyhow::{Result, bail};
use std::process::ExitCode;

fn run() -> Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("plugin-install") {
        let directory = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("用法：mini-cloudmusic plugin-install <目录>"))?;
        if args.next().is_some() {
            bail!("只接受一个插件目录");
        }
        println!(
            "已安装 MyGo 插件 {}",
            mygo::install(std::path::Path::new(&directory))?
        );
        return Ok(ExitCode::SUCCESS);
    }
    if args.next().is_some() {
        bail!("用法：mini-cloudmusic [start|login|logout|mcp-token|plugin-install <目录>]");
    }
    match command.as_deref() {
        None | Some("start") => {
            desktop::bundle_if_needed(WEB_AUTH_APP_NAME)?;
            player::configure(&MYGO)?;
            mcp::start(mcp::Services {
                submit: player::submit,
                quit: desktop::quit,
                sources: &MYGO,
            })?;
            return Ok(desktop::run());
        }
        Some("login") => netease::login()?,
        Some("logout") => {
            netease::logout()?;
            println!("已清除本地网易云登录会话。");
        }
        Some("mcp-token") => println!("{}", mcp::read_token()?),
        Some("--help" | "-h") => {
            println!("用法：mini-cloudmusic [start|login|logout|mcp-token|plugin-install <目录>]")
        }
        _ => bail!("用法：mini-cloudmusic [start|login|logout|mcp-token|plugin-install <目录>]"),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}
