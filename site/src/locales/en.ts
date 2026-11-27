import type { ProductCopy } from '../site-kit/types';

export const messages = {
  copy: {
    "title": "MusicX: a menu bar music player for your Mac",
    "description": "A macOS menu bar music player with synced lyrics, local playlists, agent controls, and MyGo music-source plugins.",
    "eyebrow": "MusicX for macOS",
    "headline": [
      "Music player",
      "for macOS"
    ],
    "intro": "Play music from the menu bar, control playback with an agent, and connect music platforms through plugins.",
    "imageAlt": "MusicX player with album artwork, track information, synced lyrics, a progress bar, and playback controls.",
    "imageCaption": "The MusicX player on macOS.",
    "highlights": [
      {
        "title": "Menu bar playback",
        "description": "Control playback from the player or macOS media keys."
      },
      {
        "title": "Agent controls",
        "description": "Search, play, and queue music through MCP."
      },
      {
        "title": "Music-source plugins",
        "description": "Connect music platforms with compatible MyGo plugins."
      }
    ],
    "story": {
      "label": "Agent controls",
      "title": "Control playback\nwith an agent",
      "paragraphs": [
        "An MCP client can search for songs, start playback, add tracks to the queue, and change the volume.",
        "Set up Codex or Claude Code from Settings → Agent. Other MCP clients can use the connection details shown in MusicX."
      ],
      "quoteLabel": "Example request",
      "quote": "Play a track by Jay Chou and queue two more."
    },
    "featuresTitle": "Playback features",
    "features": [
      {
        "title": "Background playback",
        "description": "Playback continues when the player window closes. Reopen it from the menu bar, or use media keys and macOS Now Playing."
      },
      {
        "title": "Artwork, lyrics, and audio quality",
        "description": "View album artwork and synced lyrics, browse albums and artists, and choose audio quality. Available details depend on the music source."
      },
      {
        "title": "Playlists and history",
        "description": "Save liked songs, create local playlists, and browse playback history. Some sources also support playlist import."
      }
    ],
    "detail": {
      "title": "Music-source plugins",
      "description": "MusicX supports compatible MyGo music plugins. Add a source from the catalog or a GitHub repository, then sign in if the source needs an account.",
      "items": [
        "Source search",
        "Album & artist details",
        "Account sign-in",
        "Playlist import",
        "Timed lyrics"
      ],
      "note": "Features, playable tracks, audio quality, and account requirements depend on each source; some offer previews only. MusicX starts without a default music source."
    },
    "stepsTitle": "Installation and setup",
    "steps": [
      {
        "title": "Install MusicX",
        "description": "Download the macOS preview, open the disk image, and move MusicX to Applications."
      },
      {
        "title": "Add a music source",
        "description": "Open Settings → Music Sources. Add a compatible plugin and connect your account if needed."
      },
      {
        "title": "Connect your agent",
        "description": "Open Settings → Agent, set up your client, then restart the client and ask for a song."
      }
    ],
    "faqTitle": "Questions",
    "faq": [
      {
        "question": "Does MusicX include a music service?",
        "answer": "MusicX starts without a default source. You choose compatible MyGo plugins. Each service decides its account, subscription, regional, and playback requirements."
      },
      {
        "question": "Which agents can control MusicX?",
        "answer": "The macOS app offers automatic setup for Codex and Claude Code. Other clients can connect with MCP over Streamable HTTP using the configuration in Settings → Agent."
      },
      {
        "question": "What happens when I close the player?",
        "answer": "Playback continues. Use the menu bar icon to reopen the player. Quit MusicX to stop playback."
      },
      {
        "question": "Is there an iPhone download?",
        "answer": "An iPhone app is available in the source project. The download on this page is the macOS preview. See the repository for the iPhone build guide."
      }
    ],
    "downloadTitle": "Download MusicX",
    "downloadDescription": "Download the macOS preview or view the source on GitHub."
  } satisfies ProductCopy,
  page: {
    "player": "Player",
    "agent": "Agent",
    "plugins": "Music plugins",
    "native": "Menu bar player",
    "control": "Agent controls",
    "sources": "Music platform plugins",
    "listening": "Playback features",
    "agentTitle": [
      "Agent",
      "playback controls"
    ],
    "pluginsLabel": "Music platforms, connected through plugins",
    "pluginsTitle": [
      "Music-source",
      "plugins"
    ],
    "examples": "Community plugin examples",
    "pluginCaption": "Plugin capabilities",
    "playerCaption": "Playback & queue",
    "start": "Installation and setup",
    "questions": "Questions",
    "ending": "Download MusicX",
    "footer": "A macOS menu bar music player",
    "neteaseCloudMusic": "NetEase Cloud Music"
  },
  actions: {
    "get": "Download preview",
    "source": "View source",
    "releases": "All releases",
    "guide": "Getting started",
    "requirements": "macOS 14 or later · Apple Silicon",
    "image": "View full screenshot",
    "unavailable": "Source is available. A public download is not available yet.",
    "available": "A preview release, available from GitHub Releases.",
    "notes": "Release notes",
    "license": "MIT open source",
    "back": "Back to top"
  },
  accessibility: {
    "home": "home",
    "mainNavigation": "Main navigation",
    "footerLinks": "Footer links",
    "languageLabel": "用中文阅读此页",
    "languageName": "中文",
    "skipToContent": "Skip to content"
  },
  notFound: {
    "title": "Page not found",
    "description": "Page not found. Please return to the homepage.",
    "home": "English homepage"
  },
};
