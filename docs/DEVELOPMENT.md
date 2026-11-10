# MusicX Development Guide

Building from source requires macOS 14 or later, Rust 1.97 or later, and the Xcode Command Line Tools. The root Cargo workspace builds the macOS app and shared crates. The iPhone client uses a separate workspace and lockfile. [iPhone build guide](../apps/ios/README.md)

## Build and run

From the repository root, run `./run`. The launcher builds the helper, NSH panel, and web sign-in component separately so each process uses its own Cargo dependency features. It prepares signed app bundles under `~/Library/Application Support/MusicX/runtime/` and starts the helper.

Set `CARGO_TARGET_DIR` to store build artifacts elsewhere. To try a change in a separate session, use an empty state directory and an unused port.

```sh
test_data=$(mktemp -d)
MINI_CLOUDMUSIC_DATA_DIR="$test_data" MINI_CLOUDMUSIC_PORT=45969 ./run
```

Use the same data directory and port when connecting to the test session. To read its token, pass `MINI_CLOUDMUSIC_DATA_DIR` to the `mcp-token` command. The test session starts with no plugins; add test sources explicitly.

## Package the macOS app

Run the packager from the repository root. It builds a DMG for the build machine's architecture with `MusicX.app` and an Applications shortcut. The app includes its panel and web sign-in helpers.

```sh
sh scripts/package-macos.sh
```

The default output is `target/distribution/MusicX-macos-<arch>.dmg`. Pass an output path to write it elsewhere. With no signing identity configured, the packager uses ad hoc signing for local testing.

For distribution, set `MUSICX_CODESIGN_IDENTITY` to your Developer ID Application identity and package to a new output path. Submit the resulting DMG using your existing notarytool Keychain profile, then staple and validate the ticket before publishing it.

```sh
MUSICX_CODESIGN_IDENTITY="Developer ID Application: Example Team (TEAMID)" \
  sh scripts/package-macos.sh target/distribution/MusicX-release.dmg
xcrun notarytool submit target/distribution/MusicX-release.dmg --keychain-profile musicx-notary --wait
xcrun stapler staple target/distribution/MusicX-release.dmg
xcrun stapler validate target/distribution/MusicX-release.dmg
```

[Apple's macOS packaging guide](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution) · [Notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

## Build and publish with GitHub Actions

The macOS CI workflow runs formatting and workspace tests on pull requests and pushes to `main`. Download the Apple Silicon DMG and its SHA-256 checksum from the successful run's **Artifacts** section. These builds use ad hoc signing for testing. [macOS CI](../.github/workflows/ci-macos.yml)

The release workflow runs when a stable version tag is pushed. GitHub Actions supplies the repository token for publishing. [Release workflow](../.github/workflows/release-macos.yml)

With all six Apple secrets unset, the workflow builds an ad hoc signed DMG and publishes it as a GitHub prerelease. macOS may block this preview on first launch because it lacks notarization. Configure all six repository Actions secrets to sign and notarize future tagged releases with Developer ID. A partial configuration fails before the build.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application certificate and private key exported as a `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password used to export the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full Developer ID Application signing identity |
| `APPLE_API_PRIVATE_KEY` | Contents of the App Store Connect team API private key `.p8` |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID |

MusicX uses manual stable tags. Tag a commit on `main` with a version that matches `apps/macos/helper/Cargo.toml` and `crates/platform-apple/Cargo.toml`. For version 0.2.0, push `v0.2.0`.

```sh
git tag -a v0.2.0 -m "MusicX 0.2.0"
git push origin v0.2.0
```

The macOS job validates the tagged commit and both package versions, then builds the DMG for the selected signing mode. For Developer ID builds it signs, notarizes, and staples the installer. The publish job verifies the checksum and uploads the installer, checksum, and `release-manifest.json` to a draft. It checks the GitHub asset digests before publishing and retains a copy of the manifest for 90 days. The manifest records the signing mode.

If publication fails after a successful build, rerun the failed jobs to reuse the same installer. The build artifact is retained for seven days. The publisher reuses matching uploads and stops if the draft contains conflicting assets. After configuring Apple credentials, use the next version tag for the signed release.

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

The settings panel copies a configuration for clients that use the `mcpServers` format. For CLI testing, retrieve the token with `cargo run --release --locked -- mcp-token`. Send it as an `Authorization: Bearer <token>` header to `http://127.0.0.1:45967/mcp` over Streamable HTTP.

| Tools | What they do |
| --- | --- |
| `list_music_plugins`, `plugin_search`, `plugin_play` | List installed plugins, search with an explicit plugin ID, or play/enqueue a plugin track |
| `manage_music_sources` | Browse sources, configure plugins, connect accounts, and import playlists |
| `manage_local_playlists` | Create, edit, pin, or play local playlists |
| `player_status`, `control_player` | Inspect and control the current session |
| `seek_player`, `set_volume` | Seek and set volume |
| `playback_history` | Read recent plays and progress, with a default of 20 entries and a maximum of 100 |

## Work with music sources

MusicX starts with the locally installed plugins, including an empty installation. Startup and listing installed sources do not install plugins or contact the catalog. Add a source in **Settings → Music Sources** on macOS or **Music Sources** on iPhone. Account sign-in, sign-out, configuration, and playlist import use that selected plugin's capabilities. MCP clients use `manage_music_sources` with `source=mygo:<plugin-id>`.

Search with `plugin_search` and an explicit `plugin` ID. Use the returned track ID and metadata with `plugin_play`; set `enqueue=true` to queue it. Generic playback, lyrics, and album requests carry the track's `source.plugin` and `source.id`. Legacy NetEase-only tools, account routes, link/ID playback, and the `login`/`logout` CLI commands have been removed. No source is inferred from a bare numeric ID. Existing local records are retained; re-add tracks without source metadata from a selected plugin to play them.

Install a compatible single-file JavaScript plugin from a local directory.

```sh
cargo run --release --locked -- plugin-install /path/to/plugin
```

Available actions depend on the plugin's capabilities. Repository installs prefer the latest Release's platform or generic ZIP, then its source archive. Repositories without a Release resolve the default branch to one fixed commit and download its ZIP. Packages are checked against MusicX's single-file JavaScript host and compiled without executing plugin code before installation; TypeScript, external modules, and unsupported host permissions remain unavailable. Every source uses the same installation flow; there is no built-in repository, source version, or checksum pin. [MyGo Music plugin directory](https://music.egoist.dev/zh/plugins)

Playback and sign-in call public plugin exports. Playing a full track preserves its supplied metadata, and enrichment matches both the plugin ID and the track ID. Source names and repository links come from the installed manifest. Playlist identifiers are opaque strings passed to the selected plugin; MusicX does not parse provider-specific links. Sign-in methods follow the installed plugin: QR sign-in needs `host.signInPrompt`, and web sign-in needs `host.webSignIn`.

The add-source page accepts `owner/repo` or a GitHub repository URL directly, even when discovery is unavailable. MCP clients can use `manage_music_sources` with `action=install` and `repository`. Installation records the repository and selected version locally; commit-based installs also record the full commit SHA.

### Use a custom plugin catalog

The default catalog uses the same GitHub topic search as the [MyGo plugin page](https://github.com/egoist/mygo-music/blob/main/website/src/plugins.ts):

```text
https://api.github.com/search/repositories?q=topic%3Amygo-music-plugin%20archived%3Afalse&sort=stars&per_page=100
```

The catalog is downloaded when first used and saved to `mygo_source_catalog.json` in MusicX's data directory (by default `~/Library/Application Support/MusicX/`). GitHub results are reduced to repository, name, and description metadata. Cached entries are displayed immediately. After one hour, the next browse schedules a background refresh while keeping the existing list usable. The Refresh Catalog action revalidates even a fresh cache. ETags are saved and sent with `If-None-Match`; a `304` response keeps the existing data. Failed refreshes preserve the last valid download and briefly back off automatic retries. Catalog data is not bundled or tracked in Git.

Repositories are checked when added. Set `MUSICX_PLUGIN_CATALOG_URL` when starting the helper to override the default HTTP(S) JSON endpoint.

```sh
MUSICX_PLUGIN_CATALOG_URL="https://example.com/mygo_source_catalog.json" ./run
```

Custom endpoints may return a GitHub repository search response (`{"items":[{"full_name":"owner/repo"}]}`) or an array of snapshot entries with `repo`, `commit`, `manifest`, `exports`, `manifest_sha256`, and `entry_sha256` fields. Snapshot entries require a full Git commit SHA and SHA-256 hashes for the manifest and entry script. An already-cached explicit snapshot pins those installs; ordinary repository installs do not fetch or require a catalog. Changing the URL downloads the new catalog instead of reusing the previous URL's cache.

Catalog requests have a 25-second timeout and a 2 MiB response limit. Fetch or parse errors are reported when no valid cached copy exists. Invalid responses do not replace the cache. Unset the variable to restore the official topic search. Playback and account settings for installed plugins use their local installation without reading or downloading the catalog.

`catalog` retains its array response for existing MCP clients. `catalog_view` and `refresh_catalog` return `sources`, `refreshing`, and `warning`; clients can poll `catalog_view` while `refreshing` is true to display the completed background update. ZIP downloads are limited to 16 MiB and 4,096 entries; only the checked manifest and JavaScript entry are written into the installation.

Stream headers follow the same AVPlayer policy for every plugin: `User-Agent` is supported, the navigation-only `Referer` hint is omitted, and other headers are rejected. Sources requiring additional headers may remain unavailable on this host.

`manage_local_playlists` accepts `action=import_source`, `source=mygo:<plugin-id>`, and `source_playlist_id`. The equivalent source action is `manage_music_sources` with `action=import_playlist` and `playlist_id`. Both create local snapshots without changing the source's playlist.

## Checks before opening a pull request

Run these checks from the repository root.

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

For UI changes, include the macOS version and describe the window, playback, or source flow you tested. The vendored NSH patch notes describe platform changes. [macOS patches](../vendor/nsh-platform-macos/PATCH.md) · [iOS patches](../vendor/nsh-platform-ios/PATCH.md)

In an interactive macOS session, run the native window lifecycle regression with `MUSICX_NATIVE_UI_TEST=1 cargo test --locked -p mini-cloudmusic-panel --test window_lifecycle`. It checks first-launch activation and reopening a minimized or hidden window; ordinary workspace tests skip this UI run.
