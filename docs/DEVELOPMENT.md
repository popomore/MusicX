# MusicX Development Guide

Use the macOS prerequisites in the README. The root Cargo workspace builds the macOS app and shared crates. The iPhone client uses a separate workspace and lockfile. [Quick start](../README.md#quick-start) · [iPhone build guide](../apps/ios/README.md)

## Build and run

From the repository root, run `./run`. The launcher builds the helper, NSH panel, and web sign-in component separately so each process uses its own Cargo dependency features. It prepares signed app bundles under `~/Library/Application Support/MusicX/runtime/` and starts the helper.

Set `CARGO_TARGET_DIR` to store build artifacts elsewhere. To try a change in a separate session, use an empty state directory and an unused port.

```sh
test_data=$(mktemp -d)
MINI_CLOUDMUSIC_DATA_DIR="$test_data" MINI_CLOUDMUSIC_PORT=45969 ./run
```

Use the same data directory and port when connecting to the test session. To read its token, pass `MINI_CLOUDMUSIC_DATA_DIR` to the `mcp-token` command. The test session installs its own default plugin on first launch.

## Repository layout

| Path | Responsibility |
| --- | --- |
| `crates/core` | Playback state, queues, history, local playlists, and persistence |
| `crates/protocol` | Shared types and the local control protocol |
| `crates/platform-apple` | AVPlayer, system Now Playing, and macOS menu bar integration |
| `crates/plugin-api` | Rust interfaces for search, streams, track information, and albums |
| `crates/plugins` | MyGo bridge, source management, and account adapters |
| `crates/mcp` | Streamable HTTP, MCP tools, and local panel routes |
| `apps/macos/helper` | Entry point that wires the runtime together |
| `apps/macos/panel` | NSH player window and details views |
| `apps/macos/webauth` | Temporary web sign-in component |
| `apps/ios` | Standalone iPhone app |
| `assets` | App artwork and theme tokens |
| `vendor` | Patched NSH platform code |

The helper wires the player core to the Apple `AudioOutput` and MyGo `DataSource` implementations. The panel and MCP server send commands to the same playback session. The design guide describes how this affects the window lifecycle, queues, and artwork. [Design guide](DESIGN.md)

## MCP tools

Connect the client to the running helper first. [MCP setup](../README.md#connect-an-mcp-client)

| Tools | What they do |
| --- | --- |
| `search_songs` | Search NetEase Cloud Music by track or artist |
| `my_playlists`, `playlist_tracks` | Read paginated NetEase playlists and tracks |
| `play_song`, `enqueue_song` | Play or queue a NetEase track |
| `list_music_plugins`, `plugin_search`, `plugin_play` | List installed MyGo plugins, search a plugin, or play a plugin track |
| `manage_music_sources` | Browse sources, configure plugins, connect accounts, and import playlists |
| `manage_local_playlists` | Create, edit, pin, or play local playlists |
| `player_status`, `control_player` | Inspect and control the current session |
| `seek_player`, `set_volume` | Seek and set volume |
| `playback_history` | Read recent plays and progress, with a default of 20 entries and a maximum of 100 |

## Work with music sources

Use the terminal to test NetEase QR sign-in and sign-out.

```sh
cargo run --release --locked -- login
cargo run --release --locked -- logout
```

Install a compatible single-file JavaScript plugin from a local directory.

```sh
cargo run --release --locked -- plugin-install /path/to/plugin
```

Available actions depend on the plugin's capabilities. Catalog installs use a pinned Git commit and SHA-256 checks for the manifest and entry script. [MyGo Music plugin directory](https://music.egoist.dev/zh/plugins)

## Checks before opening a pull request

Run these checks from the repository root.

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

For UI changes, include the macOS version and describe the window, playback, or source flow you tested. The vendored NSH patch notes describe platform changes. [macOS patches](../vendor/nsh-platform-macos/PATCH.md) · [iOS patches](../vendor/nsh-platform-ios/PATCH.md)

In an interactive macOS session, run the native window lifecycle regression with `MUSICX_NATIVE_UI_TEST=1 cargo test --locked -p mini-cloudmusic-panel --test window_lifecycle`. It checks first-launch activation and reopening a minimized or hidden window; ordinary workspace tests skip this UI run.
