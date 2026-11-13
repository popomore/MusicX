# MusicX Development Guide

Building from source requires macOS 14 or later, Rust 1.97 or later, and the Xcode Command Line Tools. The root Cargo workspace builds the macOS app and shared crates. The iPhone client uses a separate workspace and lockfile. [iPhone build guide](../apps/ios/README.md)

## Build and run

From the repository root, run `./run`. The launcher builds the helper, NSH panel, and web sign-in component separately so each process uses its own Cargo dependency features. It prepares signed app bundles under `~/Library/Application Support/MusicX/runtime/` and starts the helper.

Set `CARGO_TARGET_DIR` to store build artifacts elsewhere. For native testing alongside an installed copy, build with a separate application identity, an empty state directory, and an unused loopback address.

```sh
qa_root=$(mktemp -d)
export CARGO_TARGET_DIR="$qa_root/build"
export MUSICX_BUILD_APP_NAME="MusicX QA"
export MUSICX_BUILD_APP_ID="local.musicx.qa"
export MUSICX_BUILD_ADDRESS="127.0.0.1:45969"
export MUSICX_DATA_DIR="$qa_root/data"
./run
```

Use unique application names and IDs for concurrent test builds. These three `MUSICX_BUILD_*` settings are compiled into the executables: they scope the playback, panel, and web sign-in bundle IDs, the panel's NSH cache identity, the reopen notification, the default data directory, and the listener. Packaged test apps retain their identity when opened from Finder. The visible player's bundle filename matches its application name, including inside the packaged app.

The test helper and UI use the same compiled address and state root, forming their own backend/UI pair alongside the installed app. UI storage and image caches stay under that instance's data directory.

Use the same build settings and data directory when connecting to the test session or running `./run mcp-token`. The test session starts with no plugins; add test sources explicitly.

Runtime overrides are `MUSICX_DATA_DIR`, `MUSICX_PORT`, and `MUSICX_WEB_AUTH`. Only these current names are supported; previous names are ignored, and previous data directories are not migrated. Production builds leave `MUSICX_BUILD_*` unset and use the MusicX name, `local.musicx` IDs, and the default listener.

## Audio spectrum checks

`cargo test --locked -p musicx-platform-apple` includes both the FFT/PCM unit
tests and an isolated real AVPlayer integration test. The integration test pumps
the main run loop, plays generated stereo WAV fixtures with a silent lead-in and
volume zero, and checks pause, seek, track replacement, and shutdown. It requires an operational
macOS audio service. `cargo test --locked -p musicx-panel --test localization`
also checks the production spectrum canvas, state changes, and stale-track
filtering without an audio device.

For native visual QA, use the separate build identity described above, play a
supported stream, and open the title bar's spectrum button. Check changing bars,
pause clearing, both UI languages, and closing the view. Some streams cannot
provide a PCM tap; those display an unavailable state instead of synthetic bars.

## UI languages

MusicX supports English and Simplified Chinese on macOS and iPhone. Language in
settings defaults to the system's preferred supported language, falling back to
English. Choosing a language applies immediately and saves it in `language` in
the app's data directory. The panel and background helper share this preference.
`MUSICX_LANGUAGE=en` or `MUSICX_LANGUAGE=zh-CN` overrides it for isolated QA runs.

Use `musicx_protocol::tr!` (or `crate::protocol::tr!` in the shared UI) for app
messages. English source messages are gettext-style keys shared by the English
and Simplified Chinese catalogs in `crates/protocol/src/locales/`. English is
the fallback when a key is missing. Provide parameters explicitly:
`tr!("View artist {}", name)` or `tr!("Like not saved: {e}"; e = error)`.
Named parameters can be reordered in translations and user content is never
interpreted as a template. Use the shared `i18n::tracks`, `playlists` and
`relative_time` helpers for counts and relative timestamps. Catalog tests verify
parameter parity; the headless localization test exercises real settings buttons,
language persistence and refresh without resetting the playback model.

Plugins receive the same `en` or `zh-CN` locale and use their own language files,
with the other supported locale as a fallback for missing keys. Track titles,
lyrics, user playlist names, and third-party errors retain their original text.
The protected Liked Songs playlist has a localized display name; its persisted
identity and name are preserved. MCP tool schemas retain their static descriptions.

## Package the macOS app

Run the packager from the repository root. It builds a DMG for the build machine's architecture with `MusicX.app` and an Applications shortcut. The app includes its panel and web sign-in helpers.

```sh
sh scripts/package-macos.sh
```

The default output is `target/distribution/MusicX-macos-<arch>.dmg`. Pass an output path to write it elsewhere. With no signing identity configured, the packager uses ad hoc signing for local testing.

### Automatic updates

Signature verification requires OpenSSL 3. On macOS, install `openssl@3` with Homebrew; the validator selects its explicit path instead of the system LibreSSL.

The persistent outer app owns Sparkle 2.10.1; the disposable player panel forwards update requests to its parent. Settings displays the parent app's version, and both the application menu and status icon's context menu expose **检查更新…**. Sparkle checks daily, downloads updates automatically, and manages installation and relaunch. Quitting stops playback and terminates the child panel before the outer app exits.

The updater uses `https://github.com/popomore/MusicX/releases/download/update-feed/appcast.xml`. The dedicated `update-feed` pre-release holds signed metadata, is never Latest, and is updated after each versioned installer is published. It retains the newest three stable and preview items. Stable items have no channel; previews use `sparkle:channel=preview`. Settings defaults to **稳定版**; **全部版本** allows preview in addition to stable. The preference lives in `update-channel` in the app's data directory and is shared by settings, both menus, and automatic checks. Switching back to stable does not downgrade an installed preview. The choice applies to subsequent update checks; it does not cancel a Preview download or an update already prepared by Sparkle, which can still be installed.

The legacy stable URL `https://github.com/popomore/MusicX/releases/latest/download/appcast.xml` remains available to existing installations. Every versioned release includes its own signed single-item `appcast.xml`. Each enclosure points to that tag's installer. The publisher authenticates previous metadata before merging it, refuses decreasing or conflicting build numbers, and checks the previous feed digest before replacement so a stale retry cannot discard newer updates. The dedicated feed must remain mutable; versioned installers and tags are never overwritten. Pushing source or building a CI artifact does not make an update available.

Feed replacement uploads a uniquely named `appcast-next-*` asset and verifies its digest before touching `appcast.xml`. The publisher then renames the old asset to `appcast-previous-<asset-id>.xml` and promotes the staged asset. Failed promotion restores the old name; a lost API response never causes an already published newer feed to be rolled back. Signed remote copies are retained for recovery rather than deleted. If a runner stops between renames, the next generation/publication can recover fully uploaded signed metadata and the live entry before continuing. The two renames are not an atomic operation; a brief interruption can still occur between them.

Configure repository variable `SPARKLE_PUBLIC_KEY` and repository secret `SPARKLE_PRIVATE_KEY` before the next Developer ID release. Generate and export a dedicated key with Sparkle's `bin/generate_keys --account musicx` and `bin/generate_keys --account musicx -x <private-file>`. Preserve this key for future releases; keep the private value out of Git. The packager embeds only the public key. The workflow passes the private key to `generate_appcast` through stdin, verifies the enclosure signature against the configured public key, and uploads the feed with the installer. A signed stable release fails if this configuration or feed is missing.

For local packaging with updates enabled, set `MUSICX_SPARKLE_PUBLIC_KEY` to the public key before running `scripts/package-macos.sh`. The script downloads a pinned, checksum-verified Sparkle distribution and signs its nested framework components before signing the outer bundle. Omit the key for local builds with updates disabled. The CLI's private development cache is never configured as an update target.

Release versions come from tags, without changing Cargo package versions. `CFBundleShortVersionString` is the tag's numeric `X.Y.Z`; `MusicXReleaseVersion` retains the full preview suffix for the settings display. `CFBundleVersion` and `sparkle:version` use `1000000 + GITHUB_RUN_NUMBER` from the single Release macOS workflow. This migrates legacy `1.0.0` installations and orders stable and preview builds together; reruns of the same workflow keep their build number. Do not recreate the workflow or reset its counter without a migration. Older preview packages without Sparkle require a manual installation of a newer build. A real published A→B upgrade, Developer ID validation, and notarization remain separate acceptance checks.

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

The macOS CI workflow runs formatting, workspace tests, and release automation tests on pull requests and pushes to `main`. [macOS CI](../.github/workflows/ci-macos.yml)

Workspace tests use Cargo's test profile with debug information disabled (`CARGO_PROFILE_TEST_DEBUG=0 cargo test --locked --workspace`) to avoid release size optimization and LTO on every change. Rust dependencies and their compiled artifacts are cached by toolchain, target, manifests, lockfile, and compiler environment. Stable releases still run the workspace tests with `--release` before packaging.

After CI passes for a push to `main`, the Tag macOS Preview workflow creates a tag on the tested commit. It increments the patch of the latest published stable tag and appends the commit's UTC timestamp and first four characters: after `v1.0.0`, the next preview is `v1.0.1.preview-<YYYYMMDDHHMMSS>-<commit>`. A rerun reuses the tag when it points to the same commit. [Tag macOS Preview](../.github/workflows/tag-macos-preview.yml)

Tag macOS Preview dispatches Release macOS with the new tag as its ref because [tags created using `GITHUB_TOKEN` do not trigger another push workflow](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow#triggering-a-workflow-from-a-workflow). Manually pushing a version tag starts Release macOS through the tag push event. [Release workflow](../.github/workflows/release-macos.yml)

Release macOS builds an ad hoc signed DMG with Sparkle for preview tags and publishes it as a GitHub Pre-release. macOS may block it on first launch because it lacks notarization. Stable tags use Developer ID signing and notarization when all six repository Actions secrets are configured. With all six unset, a stable tag also produces an ad hoc signed regular release with a signed update feed. A partial configuration fails before the build of a stable tag.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application certificate and private key exported as a `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | Password used to export the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full Developer ID Application signing identity |
| `APPLE_API_PRIVATE_KEY` | Contents of the App Store Connect team API private key `.p8` |
| `APPLE_API_KEY` | App Store Connect API key ID |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID |

Formal releases use manual stable `vX.Y.Z` tags on tested commits on `main`; package versions are independent. Stable tags become regular releases and Latest regardless of Apple signing mode. Preview tags remain **Pre-release**. Both require the Sparkle signing configuration.

```sh
git tag -a v0.2.0 -m "MusicX 0.2.0"
git push origin v0.2.0
```

The macOS job validates the tagged commit, allocates its build number, packages the DMG, and signs both the release appcast and combined feed. Developer ID builds also notarize and staple the installer. The publish job verifies the checksum and update signature and uploads the installer, checksum, single-release appcast, and `release-manifest.json` to a draft. After verifying GitHub asset digests and publishing the installer, it updates the dedicated signed feed. The manifest records the display version, build number and signing mode and is retained for 90 days.

If publication fails after a successful build, rerun the failed jobs to reuse the same installer. Preview installers are retained for 14 days; tagged release installers are retained for seven days. The publisher reuses matching uploads and stops if the draft contains conflicting assets. After configuring Apple credentials, use the next version tag for the signed release.

The local release automation tests require Python 3.9 or later, Git, Bash, Cargo, `jq`, and `sha256sum` (GNU coreutils) on `PATH`; the GitHub CLI fixture uses `jq` to filter responses. Run them with `python3 -m unittest discover -s scripts/tests -v`. They use a local Git repository and a file-backed GitHub CLI fixture, without creating remote tags or releases.

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

The settings panel copies a configuration for clients that use the `mcpServers` format. For CLI testing, use the helper's build settings and data directory when running `./run mcp-token`. Send the token as an `Authorization: Bearer <token>` header over Streamable HTTP to the address shown in **Settings → MCP Connection**. The production default is `http://127.0.0.1:45967/mcp`; the QA example above uses `http://127.0.0.1:45969/mcp`. A valid, nonzero `MUSICX_PORT` overrides the build's port, so the client must use the helper's configured port.

| Tools | What they do |
| --- | --- |
| `list_music_plugins`, `plugin_search` | List installed source plugins and search a selected plugin; results contain standard `Track` objects |
| `play_song`, `enqueue_song` | Play or queue a complete `Track` through the configured `DataSource` |
| `manage_music_sources` | Browse sources, configure plugins, connect accounts, and import playlists |
| `manage_local_playlists` | Create, edit, pin, or play local playlists |
| `player_status`, `control_player` | Inspect and control the current session |
| `seek_player`, `set_volume` | Seek and set volume |
| `playback_history` | Read recent plays and progress, with a default of 20 entries and a maximum of 100 |

## Work with music sources

See [Plugin capabilities and MyGo differences](PLUGINS.md) for artist/podcast browsing, cloud-like/history imports, management actions, runtime constraints, and examples.

MusicX starts with the locally installed plugins, including an empty installation. Startup and listing installed sources do not install plugins or contact the catalog. Add a source in **Settings → Music Sources** on macOS or **Music Sources** on iPhone. Account sign-in, sign-out, configuration, and playlist import use that selected plugin's capabilities. MCP clients use `manage_music_sources` with `source=mygo:<plugin-id>`.

Search with `plugin_search` and an explicit `plugin` ID. Its `tracks` array contains the shared `Track` wire type: title, artist, duration in seconds, available cover/album/source-link metadata, and the original string ID under `source.id`. The adapter normalizes the plugin's native search response and preserves its top-level pagination fields.

Playback is independent of the source tools. Pass a complete item from `tracks` unchanged in the `track` parameter of `play_song` or `enqueue_song`. These tools resolve through the injected `DataSource` and submit a player action only after successful resolution. They do not invoke MyGo directly or reconstruct a partial track. `plugin_play` has been removed. The `play_song`/`enqueue_song` names now use this full-Track contract; the old bare `id` parameter is not supported.

The MyGo adapter routes playback, lyrics, and album requests by `source.plugin` and `source.id`. NetEase-only search/playlist tools, dedicated account routes, link/ID playback, and the `login`/`logout` CLI commands have been removed. No source is inferred from a bare numeric ID. Existing local records are retained; re-add tracks without source metadata from a selected plugin to play them.

Install a compatible single-file JavaScript or TypeScript plugin from a local directory.

```sh
cargo run --release --locked -- plugin-install /path/to/plugin
```

Available actions depend on the plugin's capabilities. Repository installs prefer the latest Release's platform or generic ZIP, then its source archive. Repositories without a Release resolve the default branch to one fixed commit and download its ZIP. Packages are checked against MusicX's single-file host, TypeScript is transpiled, and the entry is compiled without executing plugin code before installation; external modules and unsupported host permissions remain unavailable. Every source uses the same installation flow; there is no built-in repository, source version, or checksum pin. [MyGo Music plugin directory](https://music.egoist.dev/zh/plugins)

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

`catalog` retains its array response for existing MCP clients. `catalog_view` and `refresh_catalog` return `sources`, `refreshing`, and `warning`; clients can poll `catalog_view` while `refreshing` is true to display the completed background update. ZIP downloads are limited to 16 MiB and 4,096 entries; only the checked manifest, compiled JavaScript entry, and supported locale files are written into the installation.

Manifest `requestHeaders` and stream headers are supported. Header-dependent audio uses a helper-owned range-capable loopback transport; authenticated artwork is fetched by the helper. [Plugin compatibility and limits](PLUGINS.md#host-compatibility).

`manage_local_playlists` accepts `action=import_source`, `source=mygo:<plugin-id>`, and `source_playlist_id`. The equivalent source action is `manage_music_sources` with `action=import_playlist` and `playlist_id`. Both create local snapshots without changing the source's playlist.

## Checks before opening a pull request

Run these checks from the repository root.

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

For UI changes, include the macOS version and describe the window, playback, or source flow you tested. The vendored NSH patch notes describe platform changes. [macOS patches](../vendor/nsh-platform-macos/PATCH.md) · [iOS patches](../vendor/nsh-platform-ios/PATCH.md)

In an interactive macOS session, run the native window lifecycle regression with `MUSICX_DATA_DIR="$(mktemp -d)" MUSICX_NATIVE_UI_TEST=1 cargo test --locked -p musicx-panel --test window_lifecycle`. It checks first-launch activation, reopening minimized or hidden windows, standard menu registration, disabled empty-session actions, the settings shortcut, view navigation, closing the player, and live English/Chinese menu refresh. It changes the isolated instance's language preference; ordinary workspace tests skip this UI run.

Append `-- --menus-only` to verify menu behavior separately when the host session cannot grant focus to a test process. This mode retains the menu assertions and does not validate window activation or reopening.
