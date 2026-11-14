# MyGo plugins in MusicX

MusicX hosts a subset of the [MyGo Music plugin contract](https://github.com/egoist/mygo-music#plugins). This document describes MusicX's implementation, rather than promising that every plugin in the MyGo directory works. The compatibility baseline is MyGo **0.1.24**, with the host-specific boundaries documented below. The [catalog audit and retest](reports/2026-11-15-plugin-compatibility.md) record exact verification evidence.

## Supported capabilities

Capabilities come from the installed script's exports. A service may require an account, subscription, or configuration even when its plugin exposes the corresponding function.

| MyGo exports | MusicX behavior |
| --- | --- |
| `search` | MCP searches tracks, albums, artists, playlists, or shows, with offset pagination. Track results use MusicX's shared `Track` type and durations in seconds; other search kinds retain the provider's entity shape. |
| `stream` | Resolves HTTP or HTTPS audio for AVPlayer. Supports source quality selection, HTTP request headers, and optional `gain`/`peak` normalization. Actual codec and service compatibility still need runtime verification. |
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

JavaScript and TypeScript single-file entries are supported. TypeScript is transpiled locally and installed as JavaScript; imports/re-exports of external modules remain unsupported. MusicX also installs `locales/en.json` and `locales/zh-CN.json`. Bounded bundled assets and executable bits are retained, but installing a file never authorizes executing it. Reserved settings/install-record names and primary-file aliases are protected case-insensitively.

Source details expose **More Options → Check for Updates**, **Update Source**, and **Remove Source**. Updates validate the new package and source ID before replacement, preserve settings/account storage, and keep the previous plugin if package validation or replacement fails. Removal requires a second confirmation in the UI, deletes plugin settings/account storage, and retains local playlists and history. Tracks from a removed source require that source to be added again before playback.

**Automatically Update Plugins** (自动更新插件) in Music Sources is enabled by default. While MusicX is running, the persistent host checks repository-installed plugins daily and installs available updates, even when the macOS player panel is closed. Checks begin 30 seconds after startup; failures retry after an hour without blocking other sources. The preference and per-source check times persist in `plugin-updates.json`, so restarting does not repeat recent checks. Turning the switch off also cancels downloaded updates that have not begun installation. Manual checks and updates remain available.

Automatic updates preserve settings and account storage, wait for unfinished sign-in flows, validate source identity and compatibility, and reject lower semantic plugin versions. A source modified during download is checked again later. Local-directory installs and local-directory updates stay manual, including when their manifest declares a repository. They resume automatic updates only after an explicit repository update or reinstall. Custom catalog pins and hashes apply to automatic updates as well.

Each repository install writes a host-owned `mygo-plugins/<id>/musicx-install.json`. It records the repository, declared plugin version, installed revision (Release tag or commit), a commit SHA when resolved, and SHA256 hashes of the installed manifest, compiled entry, supported locale files and bundled assets. These hashes exclude account storage and settings. Automatic updates require this record and verify the payload before checking and again before replacement; directly edited files pause automatic updates.

Older installs without this record stay manual, even if their manifest contains `sourceVersion` or `sourceCommit`. Use **More Options → Update Source** once to establish a repository baseline and enable automatic tracking. This action remains available when no newer version exists and replaces local plugin files. A local-directory update removes the host record and returns the source to manual management.

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

Artist/album/show results use `SourcePage`: `id`, `title`, `description`, `cover_url`, `tracks`, `collections`, `more`, `next_offset`. A collection carries `kind` (`album`, `artist`, `show`, `playlist`, `station` or `page`), `id`, `name`, `description`, and `cover_url`. For show pagination, pass the previous result's `next_offset` until `more` is false. Songs and episodes in `tracks` can be passed unchanged to `play_song` or `enqueue_song`.

Management examples:

```json
{"action":"check_update","source":"mygo:example"}
{"action":"update","source":"mygo:example"}
{"action":"auto_update_settings"}
{"action":"set_auto_update","enabled":false}
{"action":"uninstall","source":"mygo:example"}
{"action":"sign_in","source":"mygo:example","method":"oauth"}
```

Other sign-in methods are `qr` and `web`. Availability is reported by the installed source. Uninstall is an explicit destructive management operation; the MCP caller is responsible for requesting the intended source.

## Host compatibility

The host implements `platform`, `arch`, JSON `storage`, `setting`, `crypto.md5`, `crypto.aesEcbEncrypt`, `signInPrompt`, `webSignIn`, `openURL`, `waitForRedirect`, `t`, and `language`. It provides compatibility bindings for `fetch`, URL handling, promises and timers. It is neither a browser DOM nor a Node.js environment.

- **Login:** embedded web login runs in a temporary WebKit process on macOS. Browser callback login listens on `127.0.0.1` only, respects the sign-in deadline and cancellation, and only opens HTTPS pages within the plugin's network grants. OAuth state/PKCE generation and verification remain the plugin's responsibility. iPhone does not support these two macOS web-login flows.
- **Request headers:** manifest `requestHeaders` rules support image/audio kinds, explicit hosts or subdomain patterns, `{setting:key}` hosts, and `{setting:key}` / `{storage:key}` header values. URL-shaped host rules match that protocol; host-only rules remain HTTPS-only. HTTP audio headers require an explicit `http://` host rule, including settings-based hosts, so existing stored credentials are not silently sent over HTTP. Missing values are omitted. `stream.headers` takes precedence. Credential headers are not forwarded to another host on redirects. Transport-control headers such as `Host` and `Content-Length` cannot be supplied by a plugin.
- **Audio:** public HTTP URLs on port 80 and HTTPS URLs on port 443 retain their original protocol. An unencrypted source adds a warning to the current track's notice icon; its popover explains the HTTP connection. Playback errors use a distinct error icon in the same entry. AVPlayer receives a helper-owned loopback stream for HTTP audio or when headers beyond User-Agent are required. Audio header rules use the same explicit host grants for both protocols. HTTPS redirects cannot downgrade to HTTP. Its registration stays reserved while the corresponding audio item is loaded; unused enqueue resolutions cannot evict it. It supports byte ranges for seeking; private headers stay in helper memory. Timeouts protect connection setup and idle reads rather than limiting the total transfer duration. Packaged apps allow local networking for this helper connection without a global remote-HTTP exception. This transport does not rewrite HLS/DASH manifests or their relative subresources.
- **Artwork:** `{size}` and encoded `%7Bsize%7D` placeholders resolve to 600 pixels. Artwork with applicable image request headers carries a plugin hint in its URL fragment; the helper fetches it with current credentials on both macOS and iPhone, using the iOS native transport bridge. The hint contains no credentials. Audio-only rules and unmatched hosts leave public covers on the ordinary image transport. Authenticated responses use no-store, while ordinary images preserve HTTP cache metadata. Artwork bodies are limited to 4 MiB.
- **Loudness:** macOS Playback settings offer optional normalization, disabled by default. Source `gain` in dB and linear `peak` are used for attenuation and peak protection, with a 4 dB ReplayGain preamp. AVPlayer's volume scalar is capped at unity; MusicX does not amplify above unity. The listener's volume preference is preserved.
- **Translations:** installed names, setting labels/help, and `host.t` use installed language files. `host.language()` defaults to `zh-CN`; `MUSICX_LANGUAGE=en` selects English plugin text when starting the helper. This does not translate the complete MusicX UI.
- **Manifest:** omitted `name`, `version`, `main`, and `playback` use ID, empty version, `index.js`, and `stream` respectively. `minAppVersion` is validated against the MyGo 0.1.24 protocol baseline, not MusicX's own version. A higher minimum is rejected with an explanation.
- **FFmpeg on macOS:** verified with [Bilibili v0.4.2](https://github.com/GentleM4dman/mygo-music-bilibili/tree/43d15cca5f2eb3616bb7718a752b8e240b6c5cc5). Its manifest grants `*.bilivideo.com` and `*.bilivideo.cn`; older Bilibili packages that omit media-host grants cannot use this fallback. The plugin's version probe and MP4-to-WAV fallback run during playback resolution. The host looks for FFmpeg in the two standard Homebrew locations. It fetches audio through its proxy using the plugin's HTTPS host grants and decodes to a private temporary WAV. Decoding shares the playback resolution deadline, is limited to 256 MiB of output, and must finish before playback starts. The WAV is removed when the playback lease ends. The host recognizes the loopback URL returned by the plugin as a reference to that WAV. General `host.spawn` operations remain unsupported.
- **Errors:** MyGo's `signIn`, `setup`, `unavailable`, and `unsupported` error codes retain their meaning in user messages. MCP source calls include a `plugin_code` in error data when available.

Normal calls use a fresh 64 MiB QuickJS runtime with an 18-second deadline; sign-in can run for up to five minutes. Persist cross-call state through `host.storage`, rather than module globals, background timers, or long-lived processes. Plugin fetch responses are bounded to 8 MiB, persistent JSON storage to 4 MiB, and fetch runs on six bounded workers with at most 16 pending calls per VM. Aborting a request releases its slot and cancels queued work. UTF-8, URL/URLSearchParams, Headers, base64, form and binary request bodies and response arrayBuffer() are supplied.

## Source capabilities and permissions

`host_capabilities` is the supported subset of declared exports. `action=page` returns normalized native `SourcePage` results for `home`, `browse`, `stations`, `stationNext` and `playlist`; nested playlist envelopes and both radio response shapes preserve track identity and second-based timing. `SourcePage.station` starts a radio queue that refills automatically, retries unavailable candidates, and cancels stale requests after user controls or queue changes.

`action=invoke` accepts a public `method` and a `params` object. It supports suggestions, likes, collections, subscriptions, played reporting, comments, playlist edits and remote device selection. Internal helper exports are denied. Cloud writes and reporting are explicit; local hearts, local history and local playlists do not automatically write back. Comments and cloud edits currently use MCP rather than a dedicated native editor.

Remote playback controls the source's selected device through background RPC. Initial volume is applied before playback when the source supports it. Unsupported optional seek/volume controls preserve playback; failed startup retries the selected track. The device and provider still determine account, subscription and playback availability.

Sources with wildcard network declarations default to declared public domains plus exact HTTP(S) origins explicitly saved as server settings. Defaults shipped in a package do not grant server access. Explicit unrestricted-network and external-program permissions require `expected_fingerprint` from the descriptor's current `permission_fingerprint`; stale approval is rejected. Installation swaps, helper launch and revocation use the same permission lock. Content or version changes invalidate permission. Trust never grants it.

`host.spawn` supports stdin, line output, stderr, wait and kill on macOS after permission. Helpers must remain in the foreground of their original process group and must not detach/daemonize. Cleanup signals that group; it is not a sandbox or a guarantee to terminate detached descendants. Independent daemons, persistent watchers and background extensions remain unsupported. The separate constrained audio-only FFmpeg path remains available for compatible playback plugins. `host.dir`, `host.dataDir` and source-scoped `removeDataDir` are provided. Native file media stays inside that source's installed/data directories; generated audio is copied per loaded item to protect playback from reused cache filenames.

Packages are limited to 32 MiB and 4,096 ZIP entries, with 64 MiB per asset and 128 MiB total. Oversized source archives may fall back to a pinned entry/manifest/locales for a new self-contained source. Such incomplete fallbacks cannot replace an existing installation or install a helper-dependent source. Rejected updates leave the working installation intact.

| Remaining difference | Boundary |
| --- | --- |
| Plugin custom account/sidebar renderer | Native shared pages cover navigation, tracks, collections and radio, rather than arbitrary provider UI. |
| Suggestions and cloud editing UI | Public MCP methods are available; native autocomplete/comment/playlist editors are not added. |
| Background hooks, detached services and persistent module globals | Fresh VM per call; foreground helpers may live through a media lease. |
| npm/Node/external modules | Single-file JavaScript/TypeScript entries only. |
| Local ZIP picker and install URL scheme | GitHub packages and local-directory CLI remain the installation paths. |

## Verification boundaries

Isolated fixtures cover source identity, artist/album/podcast browsing, podcast pagination and metadata, complete liked/recent imports, failed-pagination atomicity, structured lyrics, TypeScript compilation, header rules, audio range transport, OAuth loopback completion, update preservation/rollback, minimum versions, and removal without deleting local collections. An opt-in macOS regression also decodes a generated WAV and seeks through the authenticated proxy with a real, muted AVPlayer. These checks establish host behavior, not the availability or correctness of every third-party service. Real account login, native rendering, codec playback, seeking and audible output require separate acceptance.
