# MusicX for iPhone

The iPhone client uses Rust, NSH 0.4, UIKit, AVPlayer, and AVAudioSession. It plays independently and stores its account data, playback history, and plugins on the device. It shares the playback core and protocol with the macOS app and reuses selected panel modules.

## Requirements

- Xcode with the iOS SDK and an iOS Simulator runtime for simulator builds
- Rust with the ARM64 iOS targets
- NSH CLI 0.4 and Python 3

The deployment target is iOS 15.0, as set in `.cargo/config.toml`. The iPhone app is a separate Cargo workspace and uses the repository's patched NSH iOS platform crate. [iOS platform patches](../../vendor/nsh-platform-ios/PATCH.md)

## Build and package

Start from the repository root.

```sh
cd apps/ios
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
python3 package.py --target ios-simulator
```

Package for a physical device with the `ios` target.

```sh
python3 package.py --target ios
```

The script uses `nsh` from `PATH`. Set `NSH_BIN` to select a different executable. Bundles are written to `.nsh/out/<target>/`, and the script prints the full bundle path.

The packager enables background audio before signing. Simulator builds are ad hoc signed. To install the device bundle, sign it with your own Apple development identity and matching provisioning profile.

Run a compile check from `apps/ios`.

```sh
cargo check --locked --target aarch64-apple-ios
```

## Local data and MCP

The app stores account data, playlists, playback history, and its MCP token in the sandbox's `Library/Application Support/MusicX/`. On upgrade, the app moves the legacy directory into place if the new path is absent. Open **Music Sources** to add a plugin and use its supported sign-in and playlist import actions. **My Playlists** contains local snapshots. There is no default source or provider-specific link/ID playback.

The Streamable HTTP endpoint listens on the phone's loopback interface at `http://127.0.0.1:45968/mcp`. It uses the device's own bearer token and playback session.
