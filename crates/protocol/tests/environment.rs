//! Exercise current names and upgrade aliases in isolated processes.
use musicx_protocol::{ADDRESS, address, data_dir, env_var, legacy_data_dir};
use std::{path::PathBuf, process::Command};

fn check(case: &str, values: &[(&str, &str)]) {
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "environment_child", "--nocapture"])
        .env_clear()
        .env("MUSICX_ENV_TEST_CASE", case)
        .envs(values.iter().copied())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn current_environment_names_configure_every_component() {
    check(
        "current",
        &[
            ("MUSICX_DATA_DIR", "/tmp/musicx-current-data"),
            ("MUSICX_PORT", "45970"),
            ("MUSICX_WEB_AUTH", "/tmp/musicx-current-webauth"),
        ],
    );
}

#[test]
fn legacy_environment_names_remain_upgrade_aliases() {
    check(
        "legacy",
        &[
            ("MINI_CLOUDMUSIC_DATA_DIR", "/tmp/musicx-legacy-data"),
            ("MINI_CLOUDMUSIC_PORT", "45971"),
            ("MINI_CLOUDMUSIC_WEB_AUTH", "/tmp/musicx-legacy-webauth"),
        ],
    );
}

#[test]
fn current_environment_names_take_precedence() {
    check(
        "current",
        &[
            ("MUSICX_DATA_DIR", "/tmp/musicx-current-data"),
            ("MUSICX_PORT", "45970"),
            ("MUSICX_WEB_AUTH", "/tmp/musicx-current-webauth"),
            ("MINI_CLOUDMUSIC_DATA_DIR", "/tmp/musicx-legacy-data"),
            ("MINI_CLOUDMUSIC_PORT", "45971"),
            ("MINI_CLOUDMUSIC_WEB_AUTH", "/tmp/musicx-legacy-webauth"),
        ],
    );
}

#[test]
fn default_paths_retain_the_historical_migration_source() {
    check("defaults", &[]);
}

#[test]
fn invalid_current_port_does_not_reactivate_a_legacy_override() {
    check(
        "invalid-port",
        &[("MUSICX_PORT", "0"), ("MINI_CLOUDMUSIC_PORT", "45971")],
    );
}

#[test]
fn environment_child() {
    let Ok(case) = std::env::var("MUSICX_ENV_TEST_CASE") else {
        return;
    };
    match case.as_str() {
        "current" | "legacy" => {
            let (directory, port, webauth) = if case == "current" {
                (
                    "/tmp/musicx-current-data",
                    45970,
                    "/tmp/musicx-current-webauth",
                )
            } else {
                (
                    "/tmp/musicx-legacy-data",
                    45971,
                    "/tmp/musicx-legacy-webauth",
                )
            };
            assert_eq!(data_dir(), Some(PathBuf::from(directory)));
            assert_eq!(address().port(), port);
            assert_eq!(env_var("MUSICX_WEB_AUTH"), Some(webauth.into()));
            assert!(
                legacy_data_dir().is_none(),
                "custom data paths must never migrate default user data"
            );
        }
        "defaults" => {
            let current = data_dir().unwrap();
            assert!(current.ends_with("Library/Application Support/MusicX"));
            assert_eq!(
                legacy_data_dir(),
                Some(current.with_file_name("Mini Cloud Music"))
            );
            assert_eq!(address(), ADDRESS.parse().unwrap());
            assert!(env_var("MUSICX_WEB_AUTH").is_none());
        }
        "invalid-port" => assert_eq!(address(), ADDRESS.parse().unwrap()),
        _ => panic!("unexpected environment fixture"),
    }
}
