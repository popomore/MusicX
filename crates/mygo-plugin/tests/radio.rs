//! Real script adapters must choose personal routes rather than flattening home.
use musicx_mygo_plugin::{MYGO, mygo};
use musicx_plugin::PluginSource;
use std::{fs, process::Command};

#[test]
fn radio_selects_personal_fm_and_daily_pages_and_filters_unavailable_tracks() {
    if std::env::var_os("MUSICX_RADIO_ADAPTER_CHILD").is_none() {
        let root =
            std::env::temp_dir().join(format!("musicx-radio-adapter-{}", std::process::id()));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "radio_selects_personal_fm_and_daily_pages_and_filters_unavailable_tracks",
                "--nocapture",
            ])
            .env("MUSICX_RADIO_ADAPTER_CHILD", "1")
            .env("MUSICX_DATA_DIR", root.join("state"))
            .env("MUSICX_RADIO_ADAPTER_ROOT", &root)
            .output()
            .unwrap();
        if root.exists() {
            fs::remove_dir_all(root).unwrap();
        }
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_RADIO_ADAPTER_ROOT").unwrap());
    let base = r#"
const song={id:'opaque:song',name:'Personal song',artists:[{id:'artist',name:'Artist'}],duration:180000};
const blocked={...song,id:'blocked',unavailable:true};
export function stream(){throw new Error('must not play audio')}
"#;
    let install = |id: &str, script: &str| {
        let folder = root.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("plugin.json"),
            serde_json::json!({"id":id,"name":id,"version":"1.0.0","permissions":{"network":[]}})
                .to_string(),
        )
        .unwrap();
        fs::write(folder.join("index.js"), format!("{base}\n{script}")).unwrap();
        mygo::install(&folder).unwrap();
    };
    install(
        "radio.fm",
        r#"
export function stations(){return [{id:'scene-1',name:'Public scene'},{id:'DEFAULT',name:'Private radio'}]}
export function stationNext({id}){if(id!=='DEFAULT')throw new Error('wrong station');return [song,blocked]}
export function home(){throw new Error('must not load the mixed home page')}
"#,
    );
    let tracks = MYGO.radio_tracks("mygo:radio.fm").unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].source.as_ref().unwrap().plugin, "radio.fm");
    assert_eq!(tracks[0].source.as_ref().unwrap().id, "opaque:song");
    assert_eq!(tracks[0].duration, 180.0);

    install(
        "radio.daily",
        r#"
export function home(){return {tracks:[{...song,id:'public'}],sections:[{title:'New songs',morePage:'new',tracks:[{...song,id:'new'}]},{title:'Daily picks',morePage:'daily',tracks:[{...song,id:'preview'}]}]}}
export function browse({page}){if(page!=='daily')throw new Error('unrelated page');return {sections:[{tracks:[song,blocked]}]}}
"#,
    );
    let tracks = MYGO.radio_tracks("mygo:radio.daily").unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].source.as_ref().unwrap().id, "opaque:song");

    install(
        "radio.fallback",
        r#"
export function stations(){return [{id:'normal',name:'Private FM'}]}
export function stationNext(){throw new Error('FM unavailable')}
export function home(){return {sections:[{links:[{page:'daily',title:'Daily picks'}]}]}}
export function browse({page}){if(page!=='daily')throw new Error('unrelated page');return {tracks:[song]}}
"#,
    );
    assert_eq!(MYGO.radio_tracks("mygo:radio.fallback").unwrap().len(), 1);
    install(
        "radio.public",
        r#"
export function stations(){return [{id:'news',name:'News radio'}]}
export function stationNext(){throw new Error('must not fetch public radio')}
export function home(){return {tracks:[song],sections:[{morePage:'recent',tracks:[song]},{morePage:'new',tracks:[song]}]}}
export function browse(){throw new Error('must not browse public pages')}
"#,
    );
    assert!(MYGO.radio_tracks("mygo:radio.public").unwrap().is_empty());
    install(
        "radio.section",
        r#"
export function home(){return {sections:[{morePage:'daily',tracks:[song,blocked]},{morePage:'new',tracks:[{...song,id:'new'}]}]}}
"#,
    );
    assert_eq!(MYGO.radio_tracks("mygo:radio.section").unwrap().len(), 1);
}
