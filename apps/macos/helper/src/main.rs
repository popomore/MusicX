use musicx_core::{protocol, storage};
use musicx_mcp as mcp;
use musicx_platform_apple::{desktop, player};
use musicx_plugins::{MYGO, WEB_AUTH_APP_NAME, configure_web_auth, mygo, netease};

use anyhow::{Context, Result, bail};
use std::{net::TcpListener, process::ExitCode};

fn migrate_data_dir() -> Result<()> {
    storage::migrate_legacy_data(|| {
        TcpListener::bind(protocol::address()).context("请退出旧版播放器后再迁移 MusicX 数据")
    })
}

fn run() -> Result<ExitCode> {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("package-app") {
        let destination = args
            .next()
            .context("Usage: mini-cloudmusic package-app <path>")?;
        if args.next().is_some() {
            bail!("Expected one app bundle path");
        }
        desktop::package_app(std::path::Path::new(&destination), WEB_AUTH_APP_NAME)?;
        return Ok(ExitCode::SUCCESS);
    }
    if command.as_deref() == Some("plugin-install") {
        let directory = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("用法：mini-cloudmusic plugin-install <目录>"))?;
        if args.next().is_some() {
            bail!("只接受一个插件目录");
        }
        migrate_data_dir()?;
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
            migrate_data_dir()?;
            let runtime = desktop::prepare_runtime(WEB_AUTH_APP_NAME)?;
            configure_web_auth(runtime.web_auth_executable.clone())?;
            player::configure(&MYGO)?;
            mcp::start(mcp::Services {
                submit: player::submit,
                quit: desktop::quit,
                sources: &MYGO,
            })?;
            return Ok(desktop::run(runtime));
        }
        Some("login") => {
            migrate_data_dir()?;
            let runtime = desktop::prepare_runtime(WEB_AUTH_APP_NAME)?;
            configure_web_auth(runtime.web_auth_executable)?;
            netease::login()?;
        }
        Some("logout") => {
            migrate_data_dir()?;
            netease::logout()?;
            println!("已清除本地网易云登录会话。");
        }
        Some("mcp-token") => {
            migrate_data_dir()?;
            println!("{}", mcp::read_token()?);
        }
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
