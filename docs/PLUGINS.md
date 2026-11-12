# MyGo plugins in MusicX

MusicX hosts a subset of the [MyGo Music plugin contract](https://github.com/egoist/mygo-music#plugins). This document describes MusicX's implementation, rather than promising that every plugin in the MyGo directory works. The compatibility baseline is MyGo **0.1.22**; the source comparison used upstream commit `30925cbcd3d420e93ce1c520640fc0fe9b5126da`.

## Supported capabilities

Capabilities come from the installed script's exports. A service may require an account, subscription, or configuration even when its plugin exposes the corresponding function.

| MyGo exports | MusicX behavior |
| --- | --- |
| `search` | MCP searches tracks, albums, artists, playlists, or shows, with offset pagination. Track results use MusicX's shared `Track` type and durations in seconds; other search kinds retain the provider's entity shape. |
| `stream` | Resolves audio for AVPlayer. Supports source quality selection, HTTP request headers, and optional `gain`/`peak` normalization. Actual codec and service compatibility still need runtime verification. |
| `lyrics` | LRC and translated LRC, or structured `lines` with line/word timing. The macOS lyrics view follows lines, highlights timed words, and seeks by line. |
| `album` | Album tracks, reachable from the playing track or a source page. |
| `artist` | Artist tracks and albums, accessible from track information or MCP. Artist IDs are retained alongside display names. |
| `signIn`, `signOut`, `account` | Account connection and disconnection. QR prompts work through `host.signInPrompt`; macOS supports temporary embedded web login and browser/OAuth callback login. |
| `setSetting` | Saves declared settings and notifies the plugin. Saved credentials are not echoed in source descriptions. |
| `playlists`, `playlist` | Lists provider playlists and imports complete playlists as local snapshots. |
| `liked` | Fetches all pages and imports cloud liked songs as a new local playlist. |
| `recent` | Imports the provider's returned recent tracks as a new local playlist. |
| `shows`, `show` | Lists podcasts and paginated episodes. Episodes play through the normal queue, retain show ID, description and publication date, and participate in local history/session restore. |

MusicX's heart and local playlist editor remain local. Importing likes does not enable cloud liking, and importing provider history does not create synthetic local playback events or upload local history. A recent-track import is a playlist, without provider play timestamps. Repeated imports create independent snapshots. Imports deduplicate source track identities while preserving order; failures and incomplete pagination do not save a partial playlist. The import limit is 10,000 tracks, with a two-minute budget checked between bounded source calls.

Selecting a track on an artist, source album, or podcast page plays a snapshot of that page's **loaded** tracks in order. Loading another podcast page does not mutate the active queue; select a track again to play the enlarged snapshot.

## Install, update, and remove

On macOS, open **Settings → Music Sources**. On iPhone, open **Music Sources**. Add a compatible source from the catalog or enter `owner/repo` / a GitHub repository URL. Installed sources work independently of catalog availability.

Repository installs use the latest Release's platform or generic ZIP, then its source archive. Without a Release, MusicX resolves the default branch to a fixed commit. ZIPs may contain one top-level plugin folder. Installation checks paths, sizes, host limitations, and script compilation before running any plugin code.

JavaScript and TypeScript single-file entries are supported. TypeScript is transpiled locally and installed as JavaScript; imports/re-exports of external modules remain unsupported. MusicX also installs `locales/en.json` and `locales/zh-CN.json`. It does not install executables or arbitrary extra assets.

Source details expose **More Options → Check for Updates**, **Update Source**, and **Remove Source**. Updates validate the new package and source ID before replacement, preserve settings/account storage, and keep the previous plugin if package validation or replacement fails. Removal requires a second confirmation in the UI, deletes plugin settings/account storage, and retains local playlists and history. Tracks from a removed source require that source to be added again before playback.

Local directory installs and updates are available from the helper:

```sh
cargo run --release --locked -- plugin-install /path/to/plugin
cargo run --release --locked -- plugin-update /path/to/plugin
```

Close MusicX before using the local-directory update command. `plugin-update` requires the same plugin ID to be installed already. Cached custom catalog snapshots continue to pin the corresponding repository to their specified commit and hashes; updating does not silently bypass that pin.

## MCP examples

These are parameter objects for the named tools, not shell commands. Opaque provider IDs must be passed unchanged, together with the selected source.

Search with `plugin_search`:

```json
{"plugin":"example","query":"artist name","kind":"artists","offset":0}
```

Use `manage_music_sources` to browse and import:

```json
{"action":"artist","source":"mygo:example","id":"artist:123"}
{"action":"album","source":"mygo:example","id":"album:123"}
{"action":"import_liked","source":"mygo:example"}
{"action":"import_recent","source":"mygo:example"}
{"action":"shows","source":"mygo:example"}
{"action":"show","source":"mygo:example","id":"show:123","offset":0}
```

Artist/album/show results use `SourcePage`: `id`, `title`, `description`, `cover_url`, `tracks`, `collections`, `more`, `next_offset`. A collection carries `kind` (`album` or `show`), `id`, `name`, `description`, and `cover_url`. For show pagination, pass the previous result's `next_offset` until `more` is false. Songs and episodes in `tracks` can be passed unchanged to `play_song` or `enqueue_song`.

Management examples:

```json
{"action":"check_update","source":"mygo:example"}
{"action":"update","source":"mygo:example"}
{"action":"uninstall","source":"mygo:example"}
{"action":"sign_in","source":"mygo:example","method":"oauth"}
```

Other sign-in methods are `qr` and `web`. Availability is reported by the installed source. Uninstall is an explicit destructive management operation; the MCP caller is responsible for requesting the intended source.

## Host compatibility

The host implements `platform`, `arch`, JSON `storage`, `setting`, `crypto.md5`, `crypto.aesEcbEncrypt`, `signInPrompt`, `webSignIn`, `openURL`, `waitForRedirect`, `t`, and `language`. It provides compatibility bindings for `fetch`, URL handling, promises and timers. It is neither a browser DOM nor a Node.js environment.

- **Login:** embedded web login runs in a temporary WebKit process on macOS. Browser callback login listens on `127.0.0.1` only, respects the sign-in deadline and cancellation, and only opens HTTPS pages within the plugin's network grants. OAuth state/PKCE generation and verification remain the plugin's responsibility. iPhone does not support these two macOS web-login flows.
- **Request headers:** manifest `requestHeaders` rules support image/audio kinds, explicit hosts or subdomain patterns, `{setting:key}` hosts, and `{setting:key}` / `{storage:key}` header values. Missing values are omitted. `stream.headers` takes precedence. Credential headers are not forwarded to another host on redirects. Transport-control headers such as `Host` and `Content-Length` cannot be supplied by a plugin.
- **Audio:** remote audio URLs must use HTTPS. HTTP URLs are rejected with an explicit error instead of being rewritten to HTTPS. AVPlayer receives a helper-owned loopback stream when headers beyond User-Agent are required. Its registration stays reserved while the corresponding audio item is loaded; unused enqueue resolutions cannot evict it. It supports byte ranges for seeking; private headers stay in helper memory. Timeouts protect connection setup and idle reads rather than limiting the total transfer duration. Packaged apps allow local networking for this helper connection; remote source URLs still follow the host restrictions below. This transport does not rewrite HLS/DASH manifests or their relative subresources.
- **Artwork:** `{size}` and encoded `%7Bsize%7D` placeholders resolve to 600 pixels. Artwork with applicable image request headers carries a plugin hint in its URL fragment; the helper fetches it with current credentials on both macOS and iPhone, using the iOS native transport bridge. The hint contains no credentials. Audio-only rules and unmatched hosts leave public covers on the ordinary image transport. Authenticated responses use no-store, while ordinary images preserve HTTP cache metadata. Artwork bodies are limited to 4 MiB.
- **Loudness:** macOS Playback settings offer optional normalization, disabled by default. Source `gain` in dB and linear `peak` are used for attenuation and peak protection, with a 4 dB ReplayGain preamp. AVPlayer's volume scalar is capped at unity; MusicX does not amplify above unity. The listener's volume preference is preserved.
- **Translations:** installed names, setting labels/help, and `host.t` use installed language files. `host.language()` defaults to `zh-CN`; `MUSICX_LANGUAGE=en` selects English plugin text when starting the helper. This does not translate the complete MusicX UI.
- **Manifest:** omitted `name`, `version`, `main`, and `playback` use ID, empty version, `index.js`, and `stream` respectively. `minAppVersion` is validated against the MyGo 0.1.22 protocol baseline, not MusicX's own version. A higher minimum is rejected with an explanation.
- **Errors:** MyGo's `signIn`, `setup`, `unavailable`, and `unsupported` error codes retain their meaning in user messages. MCP source calls include a `plugin_code` in error data when available.

Normal calls use a fresh 24 MiB QuickJS runtime with an 18-second deadline; sign-in can run for up to five minutes. Persist cross-call state through `host.storage`, rather than module globals, background timers, or long-lived processes. Plugin fetch responses remain bounded to 2 MiB.

## Differences intentionally retained

| MyGo capability | MusicX difference |
| --- | --- |
| `home`, `browse`, manifest `navigation`, account `page` | No plugin-defined home/sidebar/page renderer. |
| `suggest` | No search suggestions. |
| `isLiked`, `like`, `likeEpisode`, `played` | No cloud favorite writes or local-play reporting; likes/history are read-only imports. |
| `saveAlbum`, `savedAlbums`, `followArtist`, `savePlaylist` | No cloud collection/artist management. |
| Cloud playlist create/rename/delete/add/remove/move | Local playlist management only; no write-back. |
| `subscribeShow` | Podcast browsing/playback is supported; cloud subscriptions are not managed. |
| `stations`, `stationNext` | No radio/FM session. |
| `comments`, `likeComment`, `postComment` | No comments UI/API. |
| `playback:"remote"`, `remote*` | Only local stream playback. |
| `host.spawn`, `host.dir`, `host.dataDir`, `host.removeDataDir` | No external programs or general plugin filesystem API. Process permissions are rejected. |
| `network:["*"]`, HTTP, IP hosts, `.local`, custom ports | Retains explicit public HTTPS host grants on port 443; arbitrary-network/self-hosted/LAN plugins may be incompatible. |
| Local ZIP picker, install URL scheme, arbitrary bundled assets | GitHub packages and local-directory CLI only; only entry, manifest, settings and supported locale files are installed. |

## Verification boundaries

Isolated fixtures cover source identity, artist/album/podcast browsing, podcast pagination and metadata, complete liked/recent imports, failed-pagination atomicity, structured lyrics, TypeScript compilation, header rules, audio range transport, OAuth loopback completion, update preservation/rollback, minimum versions, and removal without deleting local collections. An opt-in macOS regression also decodes a generated WAV and seeks through the authenticated proxy with a real, muted AVPlayer. These checks establish host behavior, not the availability or correctness of every third-party service. Real account login, native rendering, codec playback, seeking and audible output require separate acceptance.
