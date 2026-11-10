use musicx_mcp as mcp;
use musicx_platform_apple::{desktop, player};
use musicx_plugins::{MYGO, configure_web_auth, mygo};

use anyhow::{Context, Result, bail};
use std::process::ExitCode;

fn run() -> Result<ExitCode> {
    let web_auth_name = format!("{} 网页登录", desktop::APP_NAME);
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if command.as_deref() == Some("package-app") {
        let destination = args.next().context("Usage: musicx package-app <path>")?;
        if args.next().is_some() {
            bail!("Expected one app bundle path");
        }
        desktop::package_app(std::path::Path::new(&destination), &web_auth_name)?;
        return Ok(ExitCode::SUCCESS);
    }
    if command.as_deref() == Some("plugin-install") {
        let directory = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("用法：musicx plugin-install <目录>"))?;
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
        bail!("用法：musicx [start|mcp-token|plugin-install <目录>]");
    }
    match command.as_deref() {
        None | Some("start") => {
            let runtime = desktop::prepare_runtime(&web_auth_name)?;
            configure_web_auth(runtime.web_auth_executable.clone())?;
            player::configure(&MYGO)?;
            mcp::start(mcp::Services {
                submit: player::submit,
                quit: desktop::quit,
                sources: &MYGO,
            })?;
            return Ok(desktop::run(runtime));
        }
        Some("mcp-token") => {
            println!("{}", mcp::read_token()?);
        }
        Some("--help" | "-h") => {
            println!("用法：musicx [start|mcp-token|plugin-install <目录>]")
        }
        _ => bail!("用法：musicx [start|mcp-token|plugin-install <目录>]"),
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
