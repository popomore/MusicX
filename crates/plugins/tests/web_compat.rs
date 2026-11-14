//! Exercises the real plugin runtime in an isolated child process.
use musicx_plugins::mygo;
use serde_json::json;
use std::{fs, process::Command, time::SystemTime};

#[test]
fn plugin_web_apis() {
    if std::env::var_os("MUSICX_WEB_COMPAT_CHILD").is_none() {
        let root = std::env::temp_dir().join(format!(
            "musicx-web-compat-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "plugin_web_apis", "--nocapture"])
            .env("MUSICX_WEB_COMPAT_CHILD", "1")
            .env("MUSICX_DATA_DIR", root.join("state"))
            .env("MUSICX_LANGUAGE", "en")
            .env("MUSICX_WEB_COMPAT_ROOT", &root)
            .output()
            .unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_WEB_COMPAT_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(
        fixture.join("plugin.json"),
        r#"{"id":"web-compat","version":"1.0.0","permissions":{"network":[]}}"#,
    )
    .unwrap();
    fs::write(
        fixture.join("index.js"),
        include_str!("fixtures/web_compat.js"),
    )
    .unwrap();
    mygo::install(&fixture).unwrap();
    let value = mygo::invoke("web-compat", "probe", json!({})).unwrap();
    assert_eq!(
        value["relative"],
        "https://example.com/search?q=%E4%B8%AD%E6%96%87&x=2"
    );
    assert_eq!(
        value["fields"],
        json!([
            "https://example.com",
            "/search",
            "?q=%E4%B8%AD%E6%96%87&x=2",
            "#part",
            "中文",
            true
        ])
    );
    assert_eq!(
        value["live"],
        json!([
            true,
            "https://example.com/next?a=2&a=3",
            ["2", "3"],
            "https://example.com/next?a=4",
            "https://other.example/a?x=5"
        ])
    );
    assert_eq!(
        value["params"],
        json!(["space a", ["1", "2"], true, "dup=3&space=space+a", 2])
    );
    assert_eq!(
        value["iterable"],
        json!(["x=1&x=2", ["x", "x"], ["1", "2"], [["x", "1"], ["x", "2"]]])
    );
    assert_eq!(
        value["utf8"],
        json!([
            [228, 184, 173, 230, 150, 135, 240, 159, 142, 181],
            "中文🎵",
            [239, 191, 189],
            "�(�",
            "A"
        ])
    );
    assert_eq!(value["streaming"], json!(["", "中", "", "文"]));
    assert_eq!(
        value["encode_into"],
        json!([{"read":1,"written":1},[65,0,0],{"read":2,"written":4},[240,159,142,181]])
    );
    assert_eq!(value["invalid"], json!([true, true, true, true]));
    assert_eq!(value["base64"], json!(["AP+AQQ==", [0, 255, 128, 65]]));
    let storage = mygo::invoke("web-compat", "storage_limits", json!({})).unwrap();
    assert_eq!(storage, json!({"rejected":true,"retained":96000}));
    assert_eq!(
        mygo::invoke("web-compat", "stored", json!({})).unwrap(),
        96000
    );
    let form = mygo::invoke("web-compat", "form_fetch", json!({})).unwrap();
    assert_eq!(form["request"]["body"], "q=a+b&lang=%E4%B8%AD%E6%96%87");
    assert_eq!(
        form["request"]["headers"]["content-type"],
        "application/x-www-form-urlencoded;charset=UTF-8"
    );
    assert_eq!(form["request"]["headers"]["x-test"], "yes");
    assert_eq!(
        form["cookies"],
        json!(["a=1; Expires=Thu, 01 Jan 2037 00:00:00 GMT", "b=2"])
    );
}
