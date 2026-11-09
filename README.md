# MusicX

MusicX 是用 Rust 编写的 macOS 菜单栏音乐播放器。AVPlayer 负责音频播放，后台维护队列、自动切歌、播放历史和本机 MCP。NSH 面板按需打开，系统「正在播放」和媒体键控制同一会话。

## 启动

需要 macOS 14 或更新版、Rust 1.97 或更新版以及 Xcode 命令行工具。在仓库根目录运行：

```sh
./run
```

`./run` 分别构建三个 macOS 进程，并将签名应用包准备到 `~/Library/Application Support/Mini Cloud Music/runtime/`。播放核心和界面分别构建，以保持它们各自的依赖特性。设置 `CARGO_TARGET_DIR` 可以指定构建输出目录。

- **MusicX Helper** 运行播放核心和 MCP。菜单栏标志内的小圆点表示播放中，悬停显示曲名和状态。
- **MusicX** 是按需打开的 NSH 窗口。左击菜单栏标志可打开，关闭窗口后后台继续播放。
- **MusicX 网页登录** 是网页登录时启动的临时 WebKit 进程。

右击菜单栏标志选择「退出 MusicX」，或在播放器激活时按 ⌘Q，可以退出后台和面板。

## 目录结构

| 目录 | 职责 |
| --- | --- |
| `crates/core` | 播放核心、MCP、MyGo 插件宿主、本地歌单和历史 |
| `crates/protocol` | 共享的数据类型、会话和本机接口协议 |
| `apps/macos/panel` | macOS 的 NSH 播放器、侧栏和音乐来源管理 |
| `apps/macos/webauth` | 临时网页登录组件 |
| `apps/ios` | 独立的 iPhone NSH 工程，复用播放核心和界面模块 |
| `assets` | 应用图标、配色和 MyGo 来源目录 |
| `vendor` | 带本地补丁的 NSH 平台源码 |

macOS 组件和共享 crate 使用根 Cargo workspace 与锁文件。iPhone 使用独立 workspace 和锁文件。界面约定和产品范围见 [DESIGN.md](docs/DESIGN.md) 与 [PRODUCT.md](docs/PRODUCT.md)。[iPhone 构建说明](apps/ios/README.md)

## 连接 Agent

启动后，在另一个终端读取本机 MCP 令牌：

```sh
cargo run --locked -- mcp-token
```

在支持 Streamable HTTP 的 Agent 中加入以下配置，具体字段以客户端为准。

```json
{
  "mcpServers": {
    "mini-cloudmusic": {
      "type": "http",
      "url": "http://127.0.0.1:45967/mcp",
      "headers": {
        "Authorization": "Bearer <本机 MCP 令牌>"
      }
    }
  }
}
```

| MCP 工具 | 功能 |
| --- | --- |
| `search_songs` | 搜索网易云曲目 |
| `my_playlists` / `playlist_tracks` | 分页读取网易云账号歌单和曲目 |
| `play_song` / `enqueue_song` | 播放或排队网易云曲目 |
| `list_music_plugins` / `plugin_search` / `plugin_play` | 查看已安装插件、搜索和点播 |
| `manage_music_sources` | 添加来源、配置、关联账号和导入歌单 |
| `manage_local_playlists` | 创建、编辑、置顶和播放本地歌单 |
| `player_status` / `control_player` | 读取和控制播放 |
| `seek_player` / `set_volume` | 跳转和设置音量 |
| `playback_history` | 查看最近的播放记录及进度 |

## 播放、歌单和历史

主窗小封面打开「正在播放」。侧栏会向当前屏幕上空余更多的一侧展开，可以切换播放历史、我的歌单和设置。点击大封面或专辑名查看专辑，点击曲名看歌曲信息和来源，点击单行歌词进入同步歌词。

从专辑、历史或本地歌单选曲会以该列表的快照作为当前播放队列。曲名右侧的爱心会加入或移除本地「我喜欢」。导入来源歌单会创建可独立编辑的本地副本。

音质在主窗选择，支持标准、高音质和无损。实际音质取决于来源返回的音频，切换时保留当前曲目和播放进度。

每次实际播放记录自己的进度。历史列表打开或刷新时显示已播时长和总时长，完整播放后显示「已播完」。旧记录只显示已有信息。Agent 可以用 `playback_history` 查询，默认 20 条，最多 100 条。

## 音乐来源

在设置中选择来源以关联账号或导入歌单。默认网易云数据源使用固定版本的 MyGo 插件。可在终端扫码登录或退出：

```sh
cargo run --locked -- login
cargo run --locked -- logout
```

也可以将支持的 MyGo 单文件 JavaScript 插件添加到播放器：

```sh
cargo run --locked -- plugin-install /path/to/plugin
```

插件能力依来源而定。播放器支持搜索、取流、专辑和歌词，以及插件声明的配置、账号关联和歌单导入。数据源目录和接口见 [MyGo Music 插件目录](https://music.egoist.dev/zh/plugins)。

本机账号、令牌、历史和歌单位于 `~/Library/Application Support/Mini Cloud Music/`。MCP 只监听本机回环地址。

## 验证

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

NSH 本地补丁见 [macOS 补丁](vendor/nsh-platform-macos/PATCH.md) 和 [iOS 补丁](vendor/nsh-platform-ios/PATCH.md)。
