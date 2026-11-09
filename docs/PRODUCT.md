# MusicX Product Overview

MusicX is a compact native player for people who choose music with an agent and listen while working on a Mac. It connects music sources to a single playback session shared by the agent, the player window, and macOS media controls. The iPhone app is a standalone player with its own session and local data.

## Playback on macOS

The menu bar icon shows playback state and opens the compact player. Closing the window releases its resources while MusicX Helper keeps playing and recording history. The system's Now Playing controls and media keys remain available. To end the session and stop the helper, use Quit MusicX.

The player shows the cover, track title, artist, album, audio quality badge, progress, and basic controls. The quality popover separates the reported stream quality from the listener's preference. The optional one-line lyric preview opens full lyrics, which follow playback and support seeking when the source provides timestamps. Track information links back to its source and album. Queue, history, playlists, and settings open beside the player so playback controls remain available.

Search and discovery are exposed through MCP. Use MCP Connection in settings to copy the endpoint configuration and connect a client. The player panel focuses on the current session and music already available in albums, playlists, and history.

## Queue, library, and history

| Collection | Behavior |
| --- | --- |
| Current queue | Tracks added individually or captured from an album, playlist, or history snapshot. Previous, next, and automatic advance follow the queue order. |
| My Playlists | Saved local collections. Create, rename, pin, or delete playlists and add or remove tracks. Editing a saved playlist leaves the current queue intact. |
| Playback history | A chronological log with separate progress for each play and a completion indicator. |
| Liked Songs | A local playlist managed by the heart button. Track edits persist across launches. The playlist name and collection are protected. |

Changing sources, liking a track, and editing a playlist are independent of queue progression. A failed track selection preserves the existing queue. Quality changes keep the current track and playback position while requesting a new stream.

## Music sources and accounts

MusicX supports compatible MyGo Music plugins, including NetEase Cloud Music. A source can offer search, playback, lyrics, album information, account sign-in, or playlist import depending on its capabilities. The settings view surfaces the available actions for each source.

Connect accounts with QR sign-in or, where supported on macOS, an embedded web sign-in window. Credentials belong to the source's local account session. Imported playlists are local snapshots that remain available after signing out. Playback of their tracks still depends on the source and its account requirements.

## Local data and upgrades

Install the macOS app by dragging MusicX from its disk image to Applications. Accounts, playlists, history, preferences, and the MCP token live in `~/Library/Application Support/MusicX/`. Installed plugins live in `mygo-plugins/`.

On upgrade, the old `Mini Cloud Music` directory is moved into place when the new path is absent and the old helper has stopped. When both directories exist, MusicX uses the new path and preserves the legacy directory. [Startup and migration details](../README.md#local-data-and-upgrades)

## iPhone

The iPhone app has touch navigation and independent background playback. It supports track selection by link or ID and browsing personal playlists with the device's own sign-in. Its MCP listener is local to the phone, and its account data and plugins live in the app sandbox. [iPhone build guide](../apps/ios/README.md)
