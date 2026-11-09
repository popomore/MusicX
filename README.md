<p align="center">
  <img src="assets/musicx.svg" width="112" height="112" alt="MusicX icon">
</p>

<h1 align="center">MusicX</h1>

<p align="center"><strong>Pick a song in chat. Close the window and keep listening.</strong></p>

MusicX is a native macOS menu bar player. Ask an MCP client to find and play a track, then use the media keys or macOS Now Playing to control it. Open the panel when you want the cover, lyrics, or queue.

> “Play a track by Jay Chou and queue two more.”

## Quick start

You need macOS 14 or later, Rust 1.97 or later, and the Xcode Command Line Tools.

Run MusicX from the repository root.

```sh
./run
```

The launcher builds the app and installs the pinned NetEase Cloud Music plugin if needed. Click the menu bar icon to open the player. A dot on the icon indicates playback. Right-click and choose **Quit MusicX**, or press Command-Q with the panel focused, to end the session.

## A player that fits in the menu bar

- **See what's playing.** Open the album from the cover, follow timed lyrics, or choose Standard, High, or Lossless from the quality badge.
- **Keep your place.** Pick a track from an album, playlist, or history to play that collection in order. History saves progress for each play.
- **Make it yours.** Save tracks with the heart, organize local playlists, and import playlists from supported MyGo sources. Manage sources and sign-in under **Settings → Music Sources**.

## Connect an MCP client

With MusicX running, get the local bearer token.

```sh
cargo run --release --locked -- mcp-token
```

Connect your client to `http://127.0.0.1:45967/mcp` using Streamable HTTP. Adapt the keys to your client.

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

The client can search, play, queue, seek, and work with sources, local playlists, and playback history. [MCP tool reference](docs/DEVELOPMENT.md#mcp-tools)

## Local data and upgrades

MusicX saves accounts, preferences, playlists, history, and the MCP token in `~/Library/Application Support/MusicX/`. Installed plugins live in `mygo-plugins/`.

Quit the old helper before upgrading. On startup, MusicX moves the legacy `Mini Cloud Music` directory into place if the new path is absent. When both directories exist, it uses the new path and preserves the old directory.

## Project guides

[Development guide](docs/DEVELOPMENT.md) · [Design guide](docs/DESIGN.md) · [Product overview](docs/PRODUCT.md) · [iPhone build guide](apps/ios/README.md)
