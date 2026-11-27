import type { ProductCopy } from '../site-kit/types';

export const messages = {
  copy: {
    "title": "MusicX：Mac 菜单栏音乐播放器",
    "description": "macOS 菜单栏音乐播放器，支持同步歌词、本地歌单、智能体控制，以及通过 MyGo 插件接入音乐源。",
    "eyebrow": "MusicX for macOS",
    "headline": [
      "Mac 菜单栏",
      "音乐播放器"
    ],
    "intro": "在菜单栏控制播放，用智能体搜索和选歌，通过插件接入不同音乐平台。",
    "imageAlt": "MusicX 播放器，显示专辑封面、歌曲信息、同步歌词、进度条和播放控制。",
    "imageCaption": "macOS 上的 MusicX 播放器。",
    "highlights": [
      {
        "title": "菜单栏播放",
        "description": "通过播放器或 macOS 媒体键控制播放。"
      },
      {
        "title": "智能体控制",
        "description": "通过 MCP 搜索、播放歌曲和管理队列。"
      },
      {
        "title": "音乐源插件",
        "description": "使用兼容的 MyGo 插件连接不同音乐平台。"
      }
    ],
    "story": {
      "label": "智能体控制",
      "title": "通过智能体\n控制播放",
      "paragraphs": [
        "支持通过 MCP 客户端搜索歌曲、开始播放、添加队列和调整音量。",
        "在「设置 → 智能体」中配置 Codex 或 Claude Code。其他 MCP 客户端也可以使用 MusicX 提供的连接信息。"
      ],
      "quoteLabel": "指令示例",
      "quote": "播放一首周杰伦的歌，再加两首到队列。"
    },
    "featuresTitle": "播放功能",
    "features": [
      {
        "title": "关闭窗口后继续播放",
        "description": "关闭播放器窗口不会停止播放。点击菜单栏图标可重新打开，也能用媒体键和 macOS「正在播放」控制音乐。"
      },
      {
        "title": "封面、歌词和音质",
        "description": "查看专辑封面和同步歌词，浏览专辑与艺人，选择可用音质。具体内容由音乐源提供。"
      },
      {
        "title": "歌单与播放历史",
        "description": "收藏歌曲、创建本地歌单、查看播放历史。部分音乐源支持导入歌单。"
      }
    ],
    "detail": {
      "title": "音乐源插件",
      "description": "MusicX 支持兼容的 MyGo 音乐插件。从目录或 GitHub 仓库添加音乐源，需要时再登录对应账号。",
      "items": [
        "歌曲搜索",
        "专辑与艺人信息",
        "账号登录",
        "歌单导入",
        "同步歌词"
      ],
      "note": "具体功能、可播放内容、音质和账号要求取决于音乐源，部分来源仅提供试听。MusicX 初次启动不附带默认音乐源。"
    },
    "stepsTitle": "安装与设置",
    "steps": [
      {
        "title": "安装 MusicX",
        "description": "下载 macOS 预览版，打开磁盘映像，将 MusicX 拖到「应用程序」。"
      },
      {
        "title": "添加一个音乐源",
        "description": "打开「设置 → 音乐源」，添加兼容插件，并按需要连接账号。"
      },
      {
        "title": "连接智能体",
        "description": "打开「设置 → 智能体」配置客户端，然后重启客户端。"
      }
    ],
    "faqTitle": "常见问题",
    "faq": [
      {
        "question": "MusicX 自带音乐服务吗？",
        "answer": "初次启动不附带默认音乐源，需要自行添加兼容的 MyGo 插件。账号、订阅、地区和播放要求由各音乐服务决定。"
      },
      {
        "question": "哪些智能体可以控制 MusicX？",
        "answer": "macOS 应用提供 Codex 和 Claude Code 自动配置。其他支持 Streamable HTTP 的 MCP 客户端，可使用「设置 → 智能体」中的连接配置。"
      },
      {
        "question": "关闭播放器窗口后会怎样？",
        "answer": "音乐继续播放。点击菜单栏图标可重新打开播放器；选择「退出 MusicX」会结束播放。"
      },
      {
        "question": "有 iPhone 版本下载吗？",
        "answer": "源码项目包含 iPhone 应用。本页提供的是 macOS 预览版，iPhone 构建说明可在源码仓库查看。"
      }
    ],
    "downloadTitle": "下载 MusicX",
    "downloadDescription": "下载 macOS 预览版，或在 GitHub 查看源码。"
  } satisfies ProductCopy,
  page: {
    "player": "播放器",
    "agent": "智能体",
    "plugins": "音乐插件",
    "native": "菜单栏播放器",
    "control": "智能体控制",
    "sources": "多平台音乐源",
    "listening": "播放功能",
    "agentTitle": [
      "通过智能体",
      "控制播放"
    ],
    "pluginsLabel": "多平台音乐源，通过插件接入",
    "pluginsTitle": [
      "多平台",
      "音乐源插件"
    ],
    "examples": "社区插件示例",
    "pluginCaption": "插件能力",
    "playerCaption": "播放与队列",
    "start": "安装与设置",
    "questions": "常见问题",
    "ending": "下载 MusicX",
    "footer": "macOS 菜单栏音乐播放器",
    "neteaseCloudMusic": "网易云音乐"
  },
  actions: {
    "get": "下载预览版",
    "source": "查看源码",
    "releases": "所有版本",
    "guide": "使用指南",
    "requirements": "macOS 14 及以上 · Apple Silicon",
    "image": "查看完整截图",
    "unavailable": "当前提供源码，尚无公开安装包。",
    "available": "当前提供预览版。下载由 GitHub Releases 提供。",
    "notes": "此版本说明",
    "license": "MIT 开源",
    "back": "返回顶部"
  },
  accessibility: {
    "home": "首页",
    "mainNavigation": "主导航",
    "footerLinks": "页脚链接",
    "languageLabel": "Read this page in English",
    "languageName": "English",
    "skipToContent": "跳到正文"
  },
  notFound: {
    "title": "未找到页面",
    "description": "未找到页面，请返回首页。",
    "home": "中文首页 →"
  },
};
