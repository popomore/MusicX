use musicx_mcp as mcp;
use musicx_platform_apple::{desktop, player};
use musicx_plugins::{MYGO, configure_web_auth, mygo};
use musicx_protocol::tr;

use anyhow::{Context, Result, bail};
use std::process::ExitCode;

fn run() -> Result<ExitCode> {
    let web_auth_name = tr!("{} Web Sign-In", desktop::APP_NAME);
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
    if matches!(command.as_deref(), Some("plugin-install" | "plugin-update")) {
        let directory = args.next().ok_or_else(|| {
            anyhow::anyhow!(tr!(
                "Usage: musicx plugin-install/plugin-update <directory>"
            ))
        })?;
        if args.next().is_some() {
            bail!(tr!("Expected one plugin directory"));
        }
        println!(
            "{}",
            tr!(
                "Saved MyGo plugin {}",
                if command.as_deref() == Some("plugin-update") {
                    mygo::update_local(std::path::Path::new(&directory))?
                } else {
                    mygo::install(std::path::Path::new(&directory))?
                }
            )
        );
        return Ok(ExitCode::SUCCESS);
    }
    if args.next().is_some() {
        bail!(tr!(
            "Usage: musicx [start|mcp-token|plugin-install/plugin-update <directory>]"
        ));
    }
    match command.as_deref() {
        None | Some("start") => {
            let runtime = desktop::prepare_runtime(&web_auth_name)?;
            configure_web_auth(runtime.web_auth_executable.clone())?;
            player::configure(&MYGO)?;
            mcp::start(mcp::Services {
                submit: player::submit,
                spectrum: musicx_platform_apple::spectrum::snapshot,
                quit: desktop::quit,
                sources: &MYGO,
            })?;
            return Ok(desktop::run(runtime));
        }
        Some("mcp-token") => {
            println!("{}", mcp::read_token()?);
        }
        Some("--help" | "-h") => {
            println!(
                "{}",
                tr!("Usage: musicx [start|mcp-token|plugin-install/plugin-update <directory>]")
            )
        }
        _ => bail!(tr!(
            "Usage: musicx [start|mcp-token|plugin-install/plugin-update <directory>]"
        )),
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
