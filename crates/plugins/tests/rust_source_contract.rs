//! Execute original fixture exports through the public, typed Rust adapter.
use musicx_core::protocol::*;
use musicx_plugin_api::PluginSource;
use musicx_plugins::{MYGO, mygo};
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn mygo_implements_typed_search_account_playlist_and_pages() {
    if std::env::var_os("MUSICX_TYPED_ADAPTER_CHILD").is_none() {
        let root =
            std::env::temp_dir().join(format!("musicx-typed-adapter-{}", std::process::id()));
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "mygo_implements_typed_search_account_playlist_and_pages",
                "--nocapture",
            ])
            .env("MUSICX_TYPED_ADAPTER_CHILD", "1")
            .env("MUSICX_DATA_DIR", root.join("state"))
            .env("MUSICX_TYPED_ADAPTER_ROOT", &root)
            .output()
            .unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("MUSICX_TYPED_ADAPTER_ROOT").unwrap());
    let fixture = root.join("fixture");
    fs::create_dir_all(&fixture).unwrap();
    fs::write(fixture.join("plugin.json"),json!({"id":"typed.fixture","name":"Typed fixture","version":"1.0.0","permissions":{"network":[]}}).to_string()).unwrap();
    let script = r#"
const track={id:'opaque:track/one',name:'Song',artists:[{id:'opaque:artist/one',name:'Artist'}],duration:42000,unavailable:true};
export function search({kind,query,offset}){if(query!=='fixture'||offset!==7)throw new Error('wrong input');return kind==='tracks'?{tracks:[track],hasMore:true,privateToken:'must-not-leak'}:{[kind]:[{id:'opaque:'+kind+'/one',name:'Entity',artists:track.artists,description:'Description',publisher:'Publisher',imageUrl:'https://example.com/cover.png',releaseDate:'2026-01-01',trackCount:1}],more:true}}
export function account(){return {id:'opaque:account',name:'Fixture account',privateToken:'must-not-leak'}}
export function playlists(){return [{id:'opaque:list/one',name:'List',trackCount:1}]}
export function playlist({id}){if(id!=='opaque:list/one')throw new Error('wrong playlist');return {playlist:{name:'Nested list',trackCount:1,tracks:[track]}}}
export function home(){return {title:'Home',sections:[{tracks:[track]}]}}
export function stationNext(){return [track]}
export function stream(){return {url:'https://example.com/song.mp3'}}
export function remotePlay(){throw new Error('must not control a device')}
export function privateSessionHelper(){return {token:'must-not-leak'}}
export function createPlaylist(){throw new Error('must not write cloud data')}
"#;
    fs::write(fixture.join("index.js"), script).unwrap();
    mygo::install(&fixture).unwrap();
    let source: &dyn PluginSource = &MYGO;
    let descriptor = source.source("mygo:typed.fixture").unwrap();
    assert!(descriptor.supports(SourceCapability::Search));
    assert!(descriptor.supports(SourceCapability::Playback));
    assert!(descriptor.supports(SourceCapability::Recommendations));
    assert!(
        descriptor
            .adapter_exports
            .iter()
            .any(|e| e == "privateSessionHelper")
    );
    assert!(
        !serde_json::to_string(&descriptor.host_capabilities)
            .unwrap()
            .contains("privateSessionHelper")
    );
    for kind in [
        SourceSearchKind::Tracks,
        SourceSearchKind::Albums,
        SourceSearchKind::Artists,
        SourceSearchKind::Playlists,
        SourceSearchKind::Shows,
    ] {
        let page = source
            .search(
                "typed.fixture",
                SourceSearchRequest {
                    query: "fixture".into(),
                    kind,
                    offset: 7,
                },
            )
            .unwrap();
        assert_eq!(page.source, "mygo:typed.fixture");
        assert!(page.more);
        if kind == SourceSearchKind::Tracks {
            assert_eq!(page.tracks[0].track.duration, 42.0);
            assert_eq!(
                page.tracks[0].track.source.as_ref().unwrap().id,
                "opaque:track/one"
            );
            assert_eq!(page.tracks[0].unavailable, Some(true));
        } else {
            let rows = match kind {
                SourceSearchKind::Albums => &page.albums,
                SourceSearchKind::Artists => &page.artists,
                SourceSearchKind::Playlists => &page.playlists,
                SourceSearchKind::Shows => &page.shows,
                _ => unreachable!(),
            };
            assert!(rows[0].id.starts_with("opaque:"));
            assert_eq!(rows[0].name, "Entity");
            assert_eq!(rows[0].artists[0].id, "opaque:artist/one");
            assert_eq!(rows[0].track_count, Some(1));
        }
        assert!(
            !serde_json::to_string(&page)
                .unwrap()
                .contains("must-not-leak")
        );
    }
    assert_eq!(
        source
            .account("mygo:typed.fixture")
            .unwrap()
            .name
            .as_deref(),
        Some("Fixture account")
    );
    assert_eq!(
        source.playlists("mygo:typed.fixture").unwrap()[0].id,
        "opaque:list/one"
    );
    let snapshot = source
        .playlist("mygo:typed.fixture", "opaque:list/one")
        .unwrap();
    assert_eq!(snapshot.origin.name, "Nested list");
    assert_eq!(snapshot.tracks[0].duration, 42.0);
    let page = source
        .page("mygo:typed.fixture", SourcePageRequest::Recommendations {})
        .unwrap();
    assert_eq!(page.tracks[0].duration, 42.0);
    assert_eq!(
        fs::read_to_string(root.join("state/mygo-plugins/typed.fixture/index.js")).unwrap(),
        script
    );
}
