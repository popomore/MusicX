# MusicX website facts

English lives at `/`; Simplified Chinese lives at `/zh/`. The site introduces the native application.

## MusicX facts and sources
[README](../../README.md), [product overview](../../docs/PRODUCT.md), [design guide](../../docs/DESIGN.md).

- Native compact macOS menu bar player. Media keys and system Now Playing operate the same playback session.
- MCP clients can search, play, queue, pause, seek and adjust volume. Settings → Agent offers setup for Codex and Claude Code and manual MCP configuration.
- Closing the player window releases its UI while the helper continues playback. Do not publish a numeric memory claim without a reproducible benchmark.
- MyGo-compatible plugins supply music sources. The app starts without a default source. Playback, account sign-in, lyrics, quality and imports depend on the source, account and available media. Do not promise all services, free playback or lossless for every track.
- Local playlists, liked songs and playback history; imported playlists are local snapshots. Timed lyrics and album/artist navigation are source-dependent.
- Community examples link to `hubandad/mygo-music-netease`, `dddaniel1/mygo-plugin-qqmusic` and `GentleM4dman/mygo-music-bilibili`, as documented in the source snapshot's [plugin verification report](https://github.com/popomore/MusicX/blob/f5d27a2f1393c66a70b489886f01610547459745/docs/reports/2026-11-15-plugin-compatibility.md). These are plugin examples, not official service partnerships or promises of full-track availability. The QQ sample is preview-limited; source/album/account requirements remain explicit. Spotify local playback is not advertised.
- An iPhone source project exists, but the website's download is the verified macOS build. Do not invent an App Store link.

The site links to preview release [v1.0.1.preview-20261116031314-f5d2](https://github.com/popomore/MusicX/releases/tag/v1.0.1.preview-20261116031314-f5d2), `MusicX-macos-arm64.dmg`. It is a prerelease; the website does not claim notarization or a stable release. The releases list provides other versions.

Use real application captures. See [asset provenance](assets.md). The build and release procedure is in [the site README](../README.md).
