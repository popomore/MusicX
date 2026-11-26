# iCloud sync

Open **Settings → iCloud** on the Mac or the **iCloud** section in iPhone settings and enable **Sync with iCloud** on each device. Use the same Apple Account and turn on iCloud Drive in system settings. Sync is off initially and runs while MusicX is running. The Mac helper continues syncing after the player window closes. iOS schedules background execution according to system policy.

MusicX syncs liked songs, local playlists (including names, pinning and source references), listening history with the progress of each individual play, and these application preferences: language, preferred audio quality, volume normalization, lyric preview, update channel and automatic plugin updates. Preferences that apply only to macOS are retained on iPhone for use on the Mac.

Source accounts, credentials, installed plugins, source configuration, plugin permissions, repository update check records, MCP tokens, current queues, volume and playback sessions remain on each device. Install the needed sources and sign in separately to play a synced track. Sync stores music metadata rather than audio files or resolved stream URLs.

You can keep making changes offline. **Sync now** retries immediately; automatic exchanges run about every 30 seconds and after iCloud reports file changes. **iCloud connected** means MusicX exchanged the locally available iCloud files and handed changes to the system. It does not confirm server upload or delivery to another device. Downloads and upload timing depend on iCloud. Disabling sync preserves both local data and existing iCloud data. An Apple Account change pauses exchanges until you deliberately turn sync off and on for that account.

## Signed builds

iCloud Documents requires an Apple signing identity, an explicit app identifier with the iCloud Documents capability, and a matching provisioning profile. Ad hoc builds retain local playback and show that iCloud is unavailable. Adding entitlement text to an ad hoc signature does not grant container access.

Register one shared iCloud container for both apps, for example `iCloud.com.example.MusicX`, and enable iCloud Documents for each app identifier. The default Mac playback helper identifier is `local.musicx.playback`; the iPhone identifier is `local.musicx.iphone`. The Mac panel and web sign-in helper do not need container access.

For macOS packaging, supply the container, the playback helper's profile, and your Apple signing identity:

```sh
export MUSICX_ICLOUD_CONTAINER=iCloud.com.example.MusicX
export MUSICX_ICLOUD_PROVISIONING_PROFILE=/path/to/MusicX.provisionprofile
export MUSICX_CODESIGN_IDENTITY='Developer ID Application: Example (TEAMID)'
sh scripts/package-macos.sh
```

For an iPhone device build, use that app's own profile and signing identity with the same container:

```sh
cd apps/ios
export MUSICX_ICLOUD_CONTAINER=iCloud.com.example.MusicX
export MUSICX_ICLOUD_PROVISIONING_PROFILE=/path/to/MusicX.mobileprovision
export MUSICX_CODESIGN_IDENTITY='Apple Development: Example (TEAMID)'
python3 package.py --target ios
```

The packagers verify the profile's expiry, exact application identifier and container authorization, embed it, add the required entitlements, and sign the final bundle. Apple profiles can authorize services with a wildcard and environments with an allowlist; the signature requests only `CloudDocuments` and one concrete environment. Simulator packaging retains its existing ad hoc behavior. Do not commit profiles or signing credentials.

For release automation, set the `MUSICX_ICLOUD_CONTAINER` variable in the restricted `macos-stable` and `macos-testflight` Environments. In `macos-stable`, also set `ICLOUD_PROVISION_PROFILE` to the base64 Developer ID profile for `local.musicx.playback`. The signing import stages it privately and passes its path to the DMG packager. TestFlight uses the existing `STORE_PROVISION_PROFILE` for `com.popomore.musicx`: regenerate it after enabling iCloud, then update the Environment secret. The TestFlight packager adds the container and iCloud entitlements after assigning the final Store bundle identity, retaining Sandbox and TestFlight entitlements.

Both signing paths reject incomplete or unauthorized configuration. Omit the iCloud settings to retain builds without iCloud support. Preview DMGs and local `--prepare-only` TestFlight QA remain ad hoc; they cannot access the container. A `--prepare-only` build rejects a configured iCloud container instead of claiming support. Changing these settings does not enable iCloud in an already installed app; install a newly signed build.

## Storage and conflicts

The Apple adapter resolves the authorized container using `URLForUbiquityContainerIdentifier`, discovers private app data with `NSMetadataQueryUbiquitousDataScope`, requests missing downloads and coordinates reads/writes through `NSFileCoordinator`. [Apple iCloud file management](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/iCloud/iCloud.html)

Each device publishes its own versioned replica under `MusicXSync/v1`. Registers track playlist metadata fields, track membership, order, individual settings and individual plays. Independent changes merge. Concurrent edits to the same field resolve deterministically by Lamport revision and device identifier; the order in which files arrive does not decide the result. Deletion tombstones are retained so stale replicas cannot restore removed content. Concurrent track additions omitted by a winning order are appended deterministically. A playlist's identity register remains deleted if another offline device adds a track to that deleted playlist.

Local `icloud-state.json` stores merge state and a recoverable apply journal. Library edits and preference writes are serialized during capture and application. Received history is materialized separately in `icloud-history.json`; the local append-only history and its byte-offset play identifiers are preserved. Corrupt or unsupported replicas stop that exchange without replacing the local library. Corrupt received-history caches leave local history readable. File sizes and collection limits are checked before application. Tombstones and history are not automatically pruned; exceeding a sync limit reports an error rather than truncating data.

An apply marker prevents local preference changes until an interrupted update is recovered. Startup performs this recovery before exposing UI/MCP mutations, and playlist edits finish pending recovery first, including when iCloud is disabled or unavailable. This prevents a post-apply deletion or a preference reverted to its previous value from being mistaken for an unapplied update. Device-local artwork paths are omitted from shared tracks and history so the receiving source can replenish artwork locally.

## Verification

Automated checks cover convergent merges, concurrent additions and field edits, deletion/re-like behavior, repeated plays and individual progress, interrupted-apply recovery, malformed data, two isolated device processes, no-op retries, account changes, disabled sync, credential exclusion, native file coordination, and English/Chinese settings controls. macOS workspace tests and an iOS target compile check cover the shared integration.

Real iCloud acceptance additionally requires two entitled, signed devices. Verify initial import in both directions, offline edits and reconnection, history progress, disabled/re-enabled sync, account changes, and unavailable storage. Local replica tests and successful builds do not establish server delivery. No signed two-device iCloud run is recorded for this implementation yet.
