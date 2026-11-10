//! Current configuration only; obsolete environment names must have no effect.
use musicx_protocol::{ADDRESS, address, data_dir};
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
fn current_environment_names_configure_data_and_port() {
    check(
        "current",
        &[
            ("MUSICX_DATA_DIR", "/tmp/musicx-current-data"),
            ("MUSICX_PORT", "45970"),
        ],
    );
}

#[test]
fn obsolete_environment_names_are_ignored() {
    check(
        "defaults",
        &[
            ("MINI_CLOUDMUSIC_DATA_DIR", "/tmp/musicx-obsolete-data"),
            ("MINI_CLOUDMUSIC_PORT", "45971"),
        ],
    );
}

#[test]
fn current_configuration_is_unaffected_by_obsolete_names() {
    check(
        "current",
        &[
            ("MUSICX_DATA_DIR", "/tmp/musicx-current-data"),
            ("MUSICX_PORT", "45970"),
            ("MINI_CLOUDMUSIC_DATA_DIR", "/tmp/musicx-obsolete-data"),
            ("MINI_CLOUDMUSIC_PORT", "45971"),
        ],
    );
}

#[test]
fn default_configuration_uses_only_musicx_paths() {
    check("defaults", &[]);
}

#[test]
fn invalid_current_port_uses_the_default() {
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
        "current" => {
            assert_eq!(data_dir(), Some(PathBuf::from("/tmp/musicx-current-data")));
            assert_eq!(address().port(), 45970);
        }
        "defaults" => {
            assert!(
                data_dir()
                    .unwrap()
                    .ends_with("Library/Application Support/MusicX")
            );
            assert_eq!(address(), ADDRESS.parse().unwrap());
        }
        "invalid-port" => assert_eq!(address(), ADDRESS.parse().unwrap()),
        _ => panic!("unexpected environment fixture"),
    }
}
