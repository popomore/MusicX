# MusicX Design Guide

MusicX keeps the current track in focus. The macOS panel is a transient view of the playback session owned by MusicX Helper. System Now Playing, media keys, and MCP operate on that same session.

## macOS layout

- Use the warm surface and text colors in `assets/theme.json`, with system typography and album artwork as the main visual. The compact window is 320 × 512 points, with 24-point content insets and 248-point square artwork.
- Keep track titles prominent. Place the artist and the linked album on one line and anchor the audio quality badge at the trailing edge. Use a tint on hover and a pressed state to identify clickable text. The title opens song information and the large cover opens the album.
- Use a solid circular play or pause button and monochrome symbols for adjacent controls. Provide accessible labels and visible selected, pressed, and disabled states.
- The MusicX mark consists of two crossing audio tracks. Its menu bar version is a fixed-width monochrome symbol. A small dot indicates playback and the tooltip carries the track and playback state. Regenerate the artwork with `sh assets/build-icons.sh`.

## Details and navigation

The compact title bar uses the current track's thumbnail to open Now Playing. Use a queue icon as the fallback. The details toolbar switches between Now Playing, History, My Playlists, and Settings. Keep the drag area available in the title bar.

The 296-point details pane opens beside the 320-point player with a 12-point transparent gap. Both panes share one native window. Choose the side with more room on the current display, preferring the right on a tie. Keep the selected side while switching views and keep the main controls available throughout.

Expand the host before sliding the pane in, and shrink it after the pane slides out. Escape, the close control, and the gap dismiss details. The thumbnail also toggles the pane when Now Playing is selected. An interrupted entrance returns to the compact size. Honor the system Reduce Motion setting.

Both the NSWindow and its NSH host container must be transparent so the desktop shows through the rounded corners and the gap. Check this on a colored desktop or with native layer inspection.

## Playback views

- **Queue.** Show the queue name, source type, and track count. Mark the active position with a subdued fill and a play or pause symbol. Locate the current track on entry, then let the listener browse freely and provide a way to return to it.
- **Lyrics.** Keep one line in the player and use a separate details view for the full lyrics. Parse LRC timestamps, align translations by time, and allow seeking by line while preserving play or pause state. Scrolling pauses follow mode. Provide a return-to-current control and present untimed lyrics as plain text.
- **Track information.** Show the song, artist, album, source link, and an action to add the track to a local playlist. Use album details for the album's track list.
- **Audio quality.** Anchor the popover to the badge. Separate the reported stream format and bitrate from the user's preference. Treat missing fields as unknown. During a quality switch, show its state and hold the requested seek position until the audio engine completes the seek. An older seek callback must never overwrite a newer position.

## Playlists and sources

Use full-row buttons with a title and secondary metadata for track lists. Use small thumbnails with a musical placeholder for missing artwork. Queue, history, album, and local playlist tracks use 36-point covers on macOS. Playlist summaries use 40-point covers. Source playlists use the artwork provided by the plugin. Local playlist previews use the first available track cover.

My Playlists opens in browse mode. Show the name, track count, and common play or add actions. Put rename, remove, and delete in management mode, with a clear way to finish. Show an inline name field when creating a playlist. The heart beside the track title toggles Liked Songs and remains in sync with edits to that playlist.

Settings groups controls under Playback, MCP Connection, and Music Sources. MCP Connection shows the local endpoint and lets users copy a client configuration or bearer token. Source details show configuration fields, account actions, and import options according to capabilities. The add-source catalog shows compatibility and reasons an action is unavailable. Use secure fields for credentials and offer replacement controls for saved secrets. Cancel an unfinished sign-in when its view closes. Web sign-in runs in a temporary WebKit process and shows the website's domain.

## Runtime boundaries

- `crates/core` owns session state and persistence. The core drives audio through `AudioOutput` and obtains media through `DataSource`. `crates/protocol` holds the shared types.
- `crates/platform-apple` adapts playback to AVPlayer and publishes system Now Playing metadata. The macOS helper assembles the platform, plugin bridge, and MCP service.
- `crates/plugin-api` defines the Rust source contract. `crates/plugins` adapts MyGo plugins and handles source configuration and accounts. `crates/mcp` exposes player actions and the panel's local routes.
- Fetch the full queue when its version changes and render only visible rows in long lists. Share playback progress with lyrics and run the lyric display timer only when needed. Limit the panel's artwork cache to 4 MiB and release its resources on close.
- The distributed macOS app ships the panel and web sign-in components inside its bundle. Source builds stage their app bundles under `runtime/` in `~/Library/Application Support/MusicX/`. Installed plugins live under `mygo-plugins/` in the same data directory. The iPhone app uses its own sandbox. [Local data and upgrades](../README.md#local-data-and-upgrades)
