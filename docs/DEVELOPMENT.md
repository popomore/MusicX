# MusicX Development Guide

Building from source requires macOS 14 or later, Rust 1.97 or later, and the Xcode Command Line Tools. The root Cargo workspace builds the macOS app and shared crates. The iPhone client uses a separate workspace and lockfile. [iPhone build guide](../apps/ios/README.md)

## Build and run

From the repository root, run `./run`. The launcher builds the helper, NSH panel, and web sign-in component separately so each process uses its own Cargo dependency features. It prepares signed app bundles under `~/Library/Application Support/MusicX/runtime/` and starts the helper.

Set `CARGO_TARGET_DIR` to store build artifacts elsewhere. To try a change in a separate session, use an empty state directory and an unused port.

```sh
test_data=$(mktemp -d)
MINI_CLOUDMUSIC_DATA_DIR="$test_data" MINI_CLOUDMUSIC_PORT=45969 ./run
```

Use the same data directory and port when connecting to the test session. To read its token, pass `MINI_CLOUDMUSIC_DATA_DIR` to the `mcp-token` command. The test session installs its own default plugin on first launch.

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

The release workflow uses the shared Meta source and asset Actions. The release unit defines the source branch, tag prefix, and version file. GitHub's private Action sharing is limited to eligible repositories, so MusicX checks out the pinned Meta Actions with a read-only token in the tag workflow. [Release workflow](../.github/workflows/release-macos.yml) · [Release unit](../.release/units/musicx-macos.json)

Set these repository Actions secrets before publishing a release. The Apple secret names match Mokisu Desktop's Developer ID workflow.

| Secret | Value |
| --- | --- |
| `META_RELEASE_READ_TOKEN` | Token with Contents read access scoped to `popomore-org/meta` |
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

Meta validates the tag against the release unit and resolves its source commit. The macOS job signs and notarizes the DMG. Meta then verifies the uploaded assets and publishes them with `release-manifest.json`. The workflow retains source and asset receipts for 90 days.

If publication fails after a successful build, rerun the failed jobs to reuse the validated installer from that run. The build artifact is retained for seven days.

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

Available actions depend on the plugin's capabilities. Catalog installs use a fixed Git commit. Known plugins retain the bundled SHA-256 checks; newly discovered plugins are resolved to a commit and checked for host compatibility before installation. [MyGo Music plugin directory](https://music.egoist.dev/zh/plugins)

## Checks before opening a pull request

Run these checks from the repository root.

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

For UI changes, include the macOS version and describe the window, playback, or source flow you tested. The vendored NSH patch notes describe platform changes. [macOS patches](../vendor/nsh-platform-macos/PATCH.md) · [iOS patches](../vendor/nsh-platform-ios/PATCH.md)
