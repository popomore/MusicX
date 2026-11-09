# MusicX

MusicX is a native macOS menu bar player written in Rust. Use an agent to find and queue music, then control playback from the MusicX panel, macOS Now Playing, or the media keys. AVPlayer handles audio while a small background process manages the session.

## Quick start

### Requirements

- macOS 14 or later
- Rust 1.97 or later
- Xcode Command Line Tools

Run the launcher from the repository root.

```sh
./run
```

The launcher builds the helper, NSH panel, and web sign-in component separately and prepares signed app bundles in `~/Library/Application Support/MusicX/runtime/`. The first launch installs the pinned NetEase Cloud Music plugin if needed. Set `CARGO_TARGET_DIR` to use a different build directory.

Click the MusicX menu bar icon to open the panel. A dot on the icon indicates playback, and hovering shows the track and its state. Closing the panel releases the window while the helper keeps playing. To stop the helper and close the panel, right-click the menu bar icon and choose **Quit MusicX**, or press Command-Q with the panel focused.

For the standalone phone client, see the [iPhone build guide](apps/ios/README.md).

## Using the player

- Click the small cover in the title bar to open **Now Playing**. Use the details toolbar to switch between the queue, playback history, local playlists, and settings.
- Click the track title for song information and source links. Click the large cover or the album name for other tracks on the album.
- Click the lyric preview for the full lyrics. Timed lyrics follow playback and support seeking by line.
- Click the quality badge to choose Standard, High, or Lossless. The popover shows the current stream's reported quality separately from your preference.
- Click the heart to add or remove the current track from **Liked Songs**. Local playlists can be created, pinned, edited, and deleted. Imported playlists are editable local copies.

Playing a track from an album, history, or a playlist creates a queue from that collection's current contents. Previous, next, and automatic advance follow that queue. History records progress for each play and marks completed entries.

## Connect an MCP client

MusicX serves Streamable HTTP at `http://127.0.0.1:45967/mcp` on the local loopback interface. With the helper running, retrieve the token from another terminal.

```sh
cargo run --release --locked -- mcp-token
```

Supply it as a bearer token. The exact configuration format depends on the client. This example uses the common `mcpServers` format.

```json
{
  "mcpServers": {
    "musicx": {
      "type": "http",
      "url": "http://127.0.0.1:45967/mcp",
      "headers": {
        "Authorization": "Bearer <local MCP token>"
      }
    }
  }
}
```

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

## Music sources

Open **Settings → Music Sources** to install or configure a source, connect an account, or import a playlist. Supported sources offer QR sign-in, web sign-in, or both. For NetEase Cloud Music, the terminal also supports QR sign-in and sign-out.

```sh
cargo run --release --locked -- login
cargo run --release --locked -- logout
```

Install a compatible single-file JavaScript plugin from a local directory.

```sh
cargo run --release --locked -- plugin-install /path/to/plugin
```

Available actions depend on the plugin's capabilities. Catalog installs use a pinned Git commit and SHA-256 checks for the manifest and entry script. [MyGo Music plugin directory](https://music.egoist.dev/zh/plugins)

## Local data and upgrades

MusicX stores account data, the MCP token, playback history, playlists, and preferences in `~/Library/Application Support/MusicX/`. The `mygo-plugins/` subdirectory contains installed plugins, and `runtime/` contains generated app bundles.

When upgrading from an older build, quit the old helper before starting MusicX. If the new directory is absent, MusicX moves the old `Mini Cloud Music` directory into place. If both directories exist, it uses `MusicX` and leaves the old data in place.

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
| `apps/ios` | Standalone iPhone app with its own Cargo workspace |
| `assets` | App artwork and theme tokens |
| `vendor` | Patched NSH platform code |

## Contributing

The product overview describes project scope, and the design guide covers interaction and runtime conventions. [Product overview](docs/PRODUCT.md) · [Design guide](docs/DESIGN.md)

Run the macOS workspace checks from the repository root.

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

For UI changes, include the macOS version and describe the playback, window, or source flow you tested. The iPhone workspace has a separate lockfile and build process. [iPhone build guide](apps/ios/README.md)

The vendored NSH patch notes describe platform changes. [macOS patches](vendor/nsh-platform-macos/PATCH.md) · [iOS patches](vendor/nsh-platform-ios/PATCH.md)
