<p align="center">
  <img src="assets/musicx.svg" width="112" height="112" alt="MusicX icon">
</p>

<h1 align="center">MusicX</h1>

<p align="center"><strong>Pick a song in chat. Close the window and keep listening.</strong></p>

MusicX is a native macOS menu bar player with support for media keys and macOS Now Playing.

## Highlights

- **Control playback through MCP.** Ask your agent to find songs, play or queue tracks, pause, skip, or adjust the volume. [Connect an MCP client](#connect-an-mcp-client)
- **Add music sources with MyGo plugins.** Browse the catalog for compatible sources such as NetEase Cloud Music, Bodian Music, and Kugou Concept. Install them from **Settings → Music Sources**.

> “Play a track by Jay Chou and queue two more.”

## Install MusicX

MusicX requires macOS 14 or later. The first notarized download is in preparation. Published builds will appear on [GitHub Releases](https://github.com/popomore/MusicX/releases).

1. Open the `MusicX-macos-*.dmg` for your Mac.
2. Drag **MusicX** to **Applications** and open it there.
3. Use the MusicX menu bar icon to reopen the player after closing the window.

MusicX installs its default NetEase Cloud Music source on first launch. A dot on the menu bar icon indicates playback. Right-click and choose **Quit MusicX**, or press Command-Q with the panel focused, to end the session.

## A player that fits in the menu bar

- **See what's playing.** Open the album from the cover, follow timed lyrics, or choose Standard, High, or Lossless from the quality badge.
- **Keep your place.** Pick a track from an album, playlist, or history to play that collection in order. History saves progress for each play.
- **Make it yours.** Save tracks with the heart, organize local playlists, and import playlists from supported MyGo sources. Manage sources and sign-in under **Settings → Music Sources**.

## Connect an MCP client

Open **Settings → MCP Connection** in MusicX and choose **Copy MCP Configuration**. Paste the configuration into a client that supports Streamable HTTP. If your client asks for separate fields, use the address shown in MusicX and **Copy Token** to supply the bearer token.

The client can search, play, queue, seek, and work with sources, local playlists, and playback history. [MCP tool reference](docs/DEVELOPMENT.md#mcp-tools)

## Local data and upgrades

MusicX saves accounts, preferences, playlists, history, and the MCP token in `~/Library/Application Support/MusicX/`. Installed plugins live in `mygo-plugins/`.

Quit the old helper before upgrading. On startup, MusicX moves the legacy `Mini Cloud Music` directory into place if the new path is absent. When both directories exist, it uses the new path and preserves the old directory.

## Project guides

[Development guide](docs/DEVELOPMENT.md) · [Design guide](docs/DESIGN.md) · [Product overview](docs/PRODUCT.md) · [iPhone build guide](apps/ios/README.md)
