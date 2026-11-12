use super::*;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Cursor, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    sync::Arc,
};

const SCRIPT: &str = r#"
export function stream() { return {url:'https://example.com/track.mp3',type:'mp3'}; }
export function seed() { host.storage.set('account',{id:'fixture-user',name:'Fixture'}); }
export function account() { return host.storage.get('account'); }
export function configured() { return host.setting('apiToken'); }
export async function signIn() {
  if(false) host.signInPrompt({text:'fixture'});
  await new Promise(resolve => setTimeout(resolve,5000));
  return {id:'fixture-user'};
}
"#;

fn manifest(id: &str, version: &str) -> Value {
    json!({
        "id":id, "name":id, "version":version, "main":"index.js",
        "repository":format!("https://github.com/example/{id}"),
        "sourceVersion":format!("v{version}"),
        "settings":[{"key":"apiToken","label":"API token"}]
    })
}

fn fixture(root: &Path, id: &str, version: &str) -> std::path::PathBuf {
    let dir = root.join(format!("fixture-{id}"));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plugin.json"), manifest(id, version).to_string()).unwrap();
    fs::write(dir.join("index.js"), SCRIPT).unwrap();
    dir
}

#[derive(Default)]
struct ServerState {
    requests: Vec<String>,
    version: String,
    fail_bad: bool,
    invalid_id: bool,
    invalid_script: bool,
    on_download: Option<Box<dyn FnOnce() + Send>>,
}

fn server() -> (String, Arc<Mutex<ServerState>>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(Mutex::new(ServerState {
        version: "2.0.0".into(),
        fail_bad: true,
        ..Default::default()
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let shared = state.clone();
    let cancelled = stop.clone();
    let base = api.clone();
    std::thread::spawn(move || {
        while !cancelled.load(Ordering::SeqCst) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                    break;
                }
            }
            let path = request.split_whitespace().nth(1).unwrap();
            let mut state = shared.lock().unwrap();
            state.requests.push(path.into());
            let (status, body, hook) = if path.starts_with("/repos/example/") {
                let id = path.split('/').nth(3).unwrap();
                if id == "bad" && state.fail_bad {
                    ("503 Unavailable", Vec::new(), None)
                } else {
                    (
                        "200 OK",
                        json!({
                            "tag_name":format!("v{}",state.version),
                            "zipball_url":format!("{base}/package/{id}.zip"), "assets":[]
                        })
                        .to_string()
                        .into_bytes(),
                        None,
                    )
                }
            } else {
                let id = path
                    .strip_prefix("/package/")
                    .unwrap()
                    .strip_suffix(".zip")
                    .unwrap();
                let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                let options = zip::write::SimpleFileOptions::default();
                zip.start_file("plugin.json", options).unwrap();
                let id = if state.invalid_id { "different" } else { id };
                zip.write_all(manifest(id, &state.version).to_string().as_bytes())
                    .unwrap();
                zip.start_file("index.js", options).unwrap();
                zip.write_all(if state.invalid_script {
                    b"export function ("
                } else {
                    SCRIPT.as_bytes()
                })
                .unwrap();
                (
                    "200 OK",
                    zip.finish().unwrap().into_inner(),
                    state.on_download.take(),
                )
            };
            drop(state);
            if let Some(hook) = hook {
                hook();
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    (api, state, stop)
}

#[test]
fn background_updates_preserve_data_and_respect_policy() {
    let root = std::env::temp_dir().join(format!(
        "musicx-plugin-updates-{}-{}",
        std::process::id(),
        storage::random_hex::<8>()
    ));
    fs::create_dir(&root).unwrap();
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "plugin_updates::tests::updates_child",
            "--nocapture",
        ])
        .env("MUSICX_UPDATES_CHILD", "1")
        .env("MUSICX_DATA_DIR", root.join("state"))
        .env("MUSICX_UPDATES_ROOT", &root)
        .env("MUSICX_PLUGIN_CATALOG_URL", "not-a-url")
        .output()
        .unwrap();
    fs::remove_dir_all(root).unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn updates_child() {
    if std::env::var("MUSICX_UPDATES_CHILD").as_deref() != Ok("1") {
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_UPDATES_ROOT").unwrap());
    let good = fixture(&root, "good", "1.0.0");
    mygo::install_repository(&good, false, None).unwrap();
    mygo::install_repository(&fixture(&root, "bad", "1.0.0"), false, None).unwrap();
    let legacy = fixture(&root, "legacy", "1.0.0");
    let mut legacy_manifest = manifest("legacy", "1.0.0");
    legacy_manifest
        .as_object_mut()
        .unwrap()
        .remove("sourceVersion");
    legacy_manifest["sourceCommit"] = json!("a".repeat(40));
    fs::write(legacy.join("plugin.json"), legacy_manifest.to_string()).unwrap();
    mygo::install_repository(&legacy, false, None).unwrap();
    let legacy_dir = mygo::plugin_root().unwrap().join("legacy");
    fs::remove_file(legacy_dir.join(crate::installation::FILE)).unwrap();
    assert_eq!(
        music_sources::source_version(
            &mygo::manifest(&mygo::plugin_root().unwrap().join("legacy")).unwrap()
        ),
        "a".repeat(40)
    );
    // Pre-marker local copies may contain sourceVersion, but must stay manual.
    mygo::install_repository(&fixture(&root, "untracked-local", "1.0.0"), false, None).unwrap();
    let untracked_dir = mygo::plugin_root().unwrap().join("untracked-local");
    fs::remove_file(untracked_dir.join(crate::installation::FILE)).unwrap();
    // A script edited before the scan must not cause a repository request.
    mygo::install_repository(&fixture(&root, "edited", "1.0.0"), false, None).unwrap();
    let edited_dir = mygo::plugin_root().unwrap().join("edited");
    let initial_edit = format!("{SCRIPT}\n// existing local edit\n");
    fs::write(edited_dir.join("index.js"), &initial_edit).unwrap();
    let local = fixture(&root, "local", "1.0.0");
    fs::copy(
        mygo::plugin_root()
            .unwrap()
            .join("good")
            .join(crate::installation::FILE),
        local.join(crate::installation::FILE),
    )
    .unwrap();
    mygo::install(&local).unwrap();
    assert!(
        !mygo::plugin_root()
            .unwrap()
            .join("local")
            .join(crate::installation::FILE)
            .exists()
    );
    assert!(
        !mygo::manifest(&mygo::plugin_root().unwrap().join("local"))
            .unwrap()
            .metadata
            .contains_key("sourceVersion")
    );
    let good_dir = mygo::plugin_root().unwrap().join("good");
    mygo::invoke("good", "seed", json!({})).unwrap();
    music_sources::configure(
        "mygo:good",
        [("apiToken".into(), "fixture-token".into())].into(),
    )
    .unwrap();
    let settings_before = fs::read(good_dir.join("settings.json")).unwrap();
    let account_path = storage::state_dir().unwrap().join("mygo-good-storage.json");
    let account_before = fs::read(&account_path).unwrap();
    let (api, server, stop) = server();
    let now = 1_700_000_000;

    assert!(settings().unwrap().enabled);
    let action = crate::protocol::MusicSourceAction::SetAutoUpdate { enabled: false };
    assert_eq!(
        music_sources::execute(action).unwrap(),
        json!({"enabled":false})
    );
    run_due(now, &api).unwrap();
    assert!(server.lock().unwrap().requests.is_empty());
    set_enabled(true).unwrap();
    run_due(now, &api).unwrap();
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.0.0");
    assert_eq!(mygo::manifest(&legacy_dir).unwrap().version, "1.0.0");
    assert_eq!(mygo::manifest(&untracked_dir).unwrap().version, "1.0.0");
    assert_eq!(
        fs::read_to_string(edited_dir.join("index.js")).unwrap(),
        initial_edit
    );
    assert!(
        !server
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|path| path.contains("legacy")
                || path.contains("untracked-local")
                || path.contains("edited"))
    );
    // An explicit repository update creates a trusted baseline for old installs.
    mygo::install_repository(&legacy, true, None).unwrap();
    run_due(now + 1, &api).unwrap();
    let upgraded = mygo::manifest(&legacy_dir).unwrap();
    assert_eq!(upgraded.version, "2.0.0");
    assert_eq!(upgraded.metadata["sourceVersion"], "v2.0.0");
    let record: Value =
        serde_json::from_slice(&fs::read(legacy_dir.join(crate::installation::FILE)).unwrap())
            .unwrap();
    assert_eq!(record["repository"], "https://github.com/example/legacy");
    assert_eq!(record["version"], "2.0.0");
    assert_eq!(record["revision"], "v2.0.0");
    assert_eq!(record["files"]["index.js"].as_str().unwrap().len(), 64);
    assert!(
        !record["files"]
            .as_object()
            .unwrap()
            .contains_key("settings.json")
    );
    assert_eq!(
        fs::read(good_dir.join("settings.json")).unwrap(),
        settings_before
    );
    assert_eq!(fs::read(&account_path).unwrap(), account_before);
    assert_eq!(
        mygo::invoke("good", "account", json!({})).unwrap()["id"],
        "fixture-user"
    );
    assert_eq!(
        mygo::invoke("good", "configured", json!({})).unwrap(),
        "fixture-token"
    );
    assert_eq!(load().unwrap().checks.len(), 3);
    mygo::uninstall("legacy").unwrap();
    let count = server.lock().unwrap().requests.len();
    run_due(now + 60, &api).unwrap();
    run_due(now + RETRY_INTERVAL - 1, &api).unwrap();
    assert_eq!(server.lock().unwrap().requests.len(), count);
    run_due(now + RETRY_INTERVAL, &api).unwrap();
    assert_eq!(server.lock().unwrap().requests.len(), count + 1);
    server.lock().unwrap().fail_bad = false;
    run_due(now + 2 * RETRY_INTERVAL, &api).unwrap();
    assert_eq!(
        mygo::manifest(&mygo::plugin_root().unwrap().join("bad"))
            .unwrap()
            .version,
        "2.0.0"
    );
    mygo::uninstall("bad").unwrap();

    // An unchanged release checks its version without downloading another ZIP.
    let downloads = || {
        server
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.starts_with("/package/good"))
            .count()
    };
    let before = downloads();
    run_due(now + CHECK_INTERVAL, &api).unwrap();
    assert_eq!(downloads(), before);

    // An identity mismatch, invalid script and older release all keep version 2.
    {
        let mut s = server.lock().unwrap();
        s.version = "3.0.0".into();
        s.invalid_id = true;
    }
    let mut tick = now + 2 * CHECK_INTERVAL;
    run_due(tick, &api).unwrap();
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.0.0");
    {
        let mut s = server.lock().unwrap();
        s.invalid_id = false;
        s.invalid_script = true;
    }
    tick += RETRY_INTERVAL;
    run_due(tick, &api).unwrap();
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.0.0");
    {
        let mut s = server.lock().unwrap();
        s.invalid_script = false;
        s.version = "1.0.0".into();
    }
    tick += RETRY_INTERVAL;
    run_due(tick, &api).unwrap();
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.0.0");

    // Switching off during a download prevents the final replacement.
    {
        let mut s = server.lock().unwrap();
        s.version = "3.0.0".into();
        s.on_download = Some(Box::new(|| {
            set_enabled(false).unwrap();
        }));
    }
    tick += RETRY_INTERVAL;
    run_due(tick, &api).unwrap();
    assert!(!settings().unwrap().enabled);
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.0.0");
    set_enabled(true).unwrap();

    // Direct script edits during a download also survive automatic replacement.
    let edited = format!("{SCRIPT}\n// keep this local edit\n");
    let edited_path = good_dir.join("index.js");
    let edited_copy = edited.clone();
    server.lock().unwrap().on_download = Some(Box::new(move || {
        fs::write(edited_path, edited_copy).unwrap();
    }));
    tick += RETRY_INTERVAL;
    run_due(tick, &api).unwrap();
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.0.0");
    assert_eq!(
        fs::read_to_string(good_dir.join("index.js")).unwrap(),
        edited
    );
    let count = server.lock().unwrap().requests.len();
    run_due(tick + CHECK_INTERVAL, &api).unwrap();
    assert_eq!(server.lock().unwrap().requests.len(), count);
    mygo::install_repository(&fixture(&root, "good", "2.0.0"), true, None).unwrap();

    // A local edit made while downloading wins, and opts out of future checks.
    let local_edit = fixture(&root, "good", "2.5.0");
    server.lock().unwrap().on_download = Some(Box::new(move || {
        mygo::update_local(&local_edit).unwrap();
    }));
    tick += RETRY_INTERVAL;
    run_due(tick, &api).unwrap();
    assert_eq!(mygo::manifest(&good_dir).unwrap().version, "2.5.0");
    let count = server.lock().unwrap().requests.len();
    run_due(tick + CHECK_INTERVAL, &api).unwrap();
    assert_eq!(server.lock().unwrap().requests.len(), count);

    // Removing a source during download never resurrects it.
    mygo::install_repository(&fixture(&root, "good", "2.0.0"), true, None).unwrap();
    server.lock().unwrap().on_download = Some(Box::new(|| {
        mygo::uninstall("good").unwrap();
    }));
    tick += CHECK_INTERVAL;
    run_due(tick, &api).unwrap();
    assert!(!good_dir.exists());
    assert!(
        mygo::plugin_root().unwrap().read_dir().unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with('.'))
    );

    // A running sign-in skips its plugin before any repository request.
    mygo::install_repository(&fixture(&root, "login", "1.0.0"), false, None).unwrap();
    let login = music_sources::begin_sign_in("mygo:login", None).unwrap();
    assert!(music_sources::auth_active("login"));
    let count = server.lock().unwrap().requests.len();
    run_due(tick + CHECK_INTERVAL, &api).unwrap();
    assert_eq!(server.lock().unwrap().requests.len(), count);
    music_sources::login_state(&login.job, true).unwrap();
    for _ in 0..100 {
        if !music_sources::auth_active("login") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!music_sources::auth_active("login"));

    // Corrupt policy data fails closed instead of silently enabling updates.
    fs::write(storage::state_dir().unwrap().join(FILE), "invalid").unwrap();
    let count = server.lock().unwrap().requests.len();
    assert!(run_due(tick + CHECK_INTERVAL, &api).is_err());
    assert_eq!(server.lock().unwrap().requests.len(), count);
    stop.store(true, Ordering::SeqCst);
}
