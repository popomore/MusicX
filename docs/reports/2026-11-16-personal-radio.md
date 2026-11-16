# Personal Radio verification

## Implemented behavior

My Playlists offers **Get Radio recommendations**. Each connected, playback-capable music account supplies its personal FM batch or an explicitly advertised daily recommendation page. MusicX alternates sources, deduplicates by provider track identity and appends to the local Radio playlist. **Get more recommendations** appends another batch. Fetching leaves the listening session and current queue intact.

Radio uses normal playlist playback and management. Renaming and pinning survive a refresh; deleting it allows a later successful fetch to create it again. Empty results do not create or clear a playlist. Per-source failures are reported while successful sources still contribute tracks. Unavailable tracks and mixed home-page content are excluded.

## Automated verification

- Workspace tests: 167 libtest cases passed, together with the custom localization, notice and UI layout harnesses. The opt-in native window lifecycle harness was skipped.
- New core regression: connected-account selection, provider identity, interleaving, deduplication, source failure isolation, malformed tracks, repeated/empty results, rename/pin preservation, deletion/recreation and atomic failure at playlist capacity.
- New MyGo regression: personal station selection, opaque identity and duration conversion, unavailable-track filtering, daily-page selection, failed-FM fallback and exclusion of public home sections/stations.
- Production playlist UI exercised in English and Simplified Chinese against an isolated HTTP fixture: disabled busy controls, fetching feedback, partial failures, preservation of failures across library revision updates, empty results, failed refresh preservation and explicit playlist playback.
- Formatting, whitespace checks, Clippy with warnings denied, 47 release-automation tests and the iOS workspace's locked dependency metadata check passed.

## Live account verification

Built a separate **MusicX Radio QA** application with its own bundle identity, data directory and loopback port. Copied the three installed plugins and their existing account state into its private test directory. Started with an empty local library and queue; automatic plugin updates were disabled for the QA instance.

| Source | Plugin version | First batch | Second batch |
| --- | --- | ---: | ---: |
| Bodian Music | 0.1.0 | 7 | 7 |
| Kugou Concept | 1.3.0 | 5 | 5 |
| NetEase Cloud Music | 0.1.6 | 3 | 3 |

The isolated application's real native backend saved 15 tracks, then appended 15 more, yielding 30 unique provider track identities. Both requests succeeded for all three accounts (about 3.3 and 2.8 seconds). The queue and playback state remained unchanged. The installed application's local playlist file retained its original SHA-256 digest throughout this check.

## Evidence boundaries

- The native automation channel failed to start on two attempts (`Sky Computer Use native pipe startup failed`). Visual inspection and manual clicks in the QA window remain unverified. UI interaction evidence above comes from the production components running in the headless NSH harness.
- Live verification covers fetching and saving recommendations. It does not establish successful stream resolution, audible playback, or behavior on a physical iPhone.
- These results concern the plugin versions and connected accounts used during this run. Sources without an identifiable personal recommendation route contribute no tracks; public recommendations are not substituted.
