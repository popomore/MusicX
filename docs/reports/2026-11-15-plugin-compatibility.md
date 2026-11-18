# 插件可信标识与兼容能力补齐

日期：2026-11-15，macOS arm64。基于 `dfc9435107d70c90b76557dc17afab7d26d773de` 后的实现，MyGo 能力版本门槛为 `0.1.24`。这是能力适配边界，不代表完整复刻上游应用。

最终结果：75 个目录项目中，74 个通过最终安装/脚本检查；Windows 任务栏歌词后台扩展被明确拒绝。34 个项目返回歌曲搜索行（包含码率信息扩展），18 个有音频包探测成功，16 个通过本轮前两秒解码。保留此前已验证证据后，共 19 个具体版本标记可信；19 个版本均重新从 GitHub 安装并通过内容匹配。

## 产品边界修订

按用户确认，MusicX 的功能由 Rust 插件契约定义，MyGo 适配器实现；注入 `host.*` 和公共 Web API 只表示运行时兼容。下方 74/75 的数字是安装与脚本检查结果，不表示 74 个来源均可由 MusicX 播放，也不自动授予 UI/MCP 能力。

播放器统一使用 `DataSource -> Media -> Rust player -> AVPlayer`。已移除 `musicx-remote` 标记及来源托管播放器的旁路。按用户要求，Spotify 专用的 librespot 取流/解码、临时 WAV、账号授权截取和额外 streaming OAuth 请求均已删除。只接受插件 `stream` 实际提供的音源；Spotify 等只有设备控制的来源不宣称支持 MusicX 本机播放，账号、搜索与歌单接口仍按原有契约处理。第三方脚本与 manifest 保持原样。

通用 `invoke` 产品接口已移除。原生推荐/浏览/电台使用规范化的 Rust 页面；MCP 保留已有来源管理、账号和导入接口，不新增原始脚本方法、设备控制、云端写入或辅助程序授权入口。个人音乐信息按独立查看、按需导入的方向设计；当前已有歌单、喜欢歌曲和历史导入，收藏专辑、关注和订阅的独立本地模型仍需后续实现。

移除专用适配时的回归确认：Spotify remote-only 来源不再声明 stream，播放解析不会调用账号或设备接口；带有真实 stream 的来源继续使用通用媒体契约，遥控标记仍被拒绝。该阶段工作区 160 项测试和自定义原生组件检查通过，另 2 项 opt-in 测试未运行；严格 Clippy、格式及 iOS locked metadata 检查通过。

之前的专用取流实现与合成 PCM/WAV 验证已被用户要求撤销，不代表当前支持 Spotify 音频，也未产生 Spotify 真实账号播放证据。Spotify 保持未验证，不增加可信标识。历史设备控制测试同样不属于当前产品播放能力。

## Rust 契约落实（2026-11-16）

同日后续拆分：通用契约迁移至 `crates/plugin`（`musicx-plugin`），MyGo 实现迁移至 `crates/mygo-plugin`（`musicx-mygo-plugin`）；下列阶段记录中的旧目录/包名保留其当时含义。当前结构见 [开发指南](../DEVELOPMENT.md#repository-layout)。运行时数据、第三方脚本与可信登记内容不变。

之前仍有搜索、账号/歌单和能力字符串耦合 MyGo 的缺口，本次已迁移：

- `crates/protocol/src/sources.rs` 定义 `SourceCapability`、搜索类型、页面请求、集合类型、歌单快照和来源管理结果；`crates/plugin-api` 定义 `PluginSource: DataSource`。
- `crates/plugins/src/adapter.rs` 实现 MyGo 适配，账号/歌单原始调用及搜索数据转换封装在这里。非歌曲搜索返回 Rust 实体的已知字段，不再透传任意 provider 数据；保留 opaque ID、歌曲秒制时长和不可播标记。
- `crates/core/src/sources.rs` 负责通用分发与本地快照导入。MCP 和 UI 路由经 `Services` 注入同一接口，MCP 普通依赖中已移除 `musicx-plugins`。
- `host_capabilities` 改为 Rust 枚举，UI 使用 `supports(SourceCapability)`；原始导出只留作诊断，旧 JSON `capabilities` 字段兼容保留。页面与集合导航也改为枚举，旧 `method`/`params` 格式兼容，但不能调用任意脚本函数。

独立 Rust 测试来源实际覆盖 MCP 搜索/账号/歌单、UI 页面路由，以及歌单/喜欢/最近播放的本地导入，不经过 MyGo；另以真实 JavaScript fixture 验证 MyGo 的类型转换、嵌套歌单和原脚本保留。旧页面请求省略或传 null params 的兼容性另有回归。此轮没有增加产品能力、修改第三方插件或改变 UI 布局，真实账号与音频验收范围不变。

契约初次提交工作区 164 项测试及自定义原生组件检查通过，2 项 opt-in 测试未运行；严格 workspace Clippy、格式和 iOS locked metadata 通过。MCP 普通依赖树无 MyGo 实现，Spotify QA 包四个原文件的 SHA256 与改动前一致。

后续评审回归补充：

- 封面、歌手和集合转换共享响应级 Artwork/Catalog 快照，避免按每张封面重算整个插件包摘要。100 首搜索在旧实现产生 529 次摘要校验；新回归限制整次搜索/歌单/页面各不超过 12 次，且下一次响应及实际图片请求重新验证权限，不使用跨请求授权缓存。
- 卸载先停辅助程序，再删除仅该来源的 `mygo-data/<id>`。损坏/缺失 manifest、嵌套 symlink、其他来源数据保留均有实际文件/进程回归。撤权不依赖有效 manifest 的描述回执；不可读时成功返回 null，正常仍返回旧对象格式。
- 歌单完整性检查使用实际 tracks 所在 envelope 的 trackCount；推荐页保留数字集合 ID。来源页新增 Rust `playable` 字段，由 Playback 能力决定；metadata-only 来源保留歌曲行但禁用播放，生产组件 headless 检查覆盖两种状态，布局未重做。
- helper 只能为自己活跃进程组拥有的 IPv4 监听端口提供隐式音源授权，或使用用户另外保存的服务器/网络授权。代理每次请求前复查 listener；真实端口回归覆盖无关服务拒绝、受管服务可用，以及撤权后其他进程重绑定相同端口也收不到代理请求。

评审补修后的完整工作区回归 165 项通过、2 项 opt-in ignored，自定义组件与严格 Clippy/格式检查通过。仍没有把模拟、真实本机进程/网络回归提升为第三方真实账号或实际音频验收。

## 可信的含义

- 可信表示列出的版本和能力有实际调用及短音频解码证据，不是安全审计、作者身份认证、账号/会员资格保证或所有曲目整曲保证。
- 绑定仓库、插件 ID、版本、规范化 manifest、安装后的脚本、语言文件及全部附带文件摘要。插件自己声明的 verification 字段无效。变更权限、代码、版本、仓库、语言文件或附带文件都会失去对应证据；用户配置和账号 storage 独立保存。
- 来源名称旁用单色圆圈勾号表示可信。目录未检查最新包时，图标的悬停提示和无障碍名称为“可信版本 X”；已安装内容完全一致时为“可信”。详情保留版本、验证范围和限制。未知版本不继承旧版本可信。
- QQ 受测曲目只有 60 秒试听，SixWa 只有 45 秒；Bilibili 样本是分段音频，短于合集总时长。详情保留这些限制。

## 已补的宿主能力

| 能力 | 当前行为与边界 |
| --- | --- |
| Web API | URL/URLSearchParams 的相对地址、读写和迭代；UTF-8 TextEncoder/TextDecoder、流式解码、Headers、base64、表单和二进制 fetch/arrayBuffer；不声称支持所有浏览器 API 或所有传统编码 |
| 并发与资源 | fetch 使用 6 个后台 worker，队列 64、每次 VM 最多 16 个请求；响应 8 MiB、storage 4 MiB、VM 64 MiB；普通调用 18 秒期限保留；失败 storage 写入保留原值 |
| 安装 | 支持带命名空间的点分 ID；ZIP 上限 32 MiB；过大源码仓库固定 commit 后读取 manifest/入口/语言；保留附带文件和可执行位，拒绝越界及符号链接；单文件 JS/TS 和不执行代码的编译检查保留 |
| 自建服务器 | 用户保存的服务器 URL 授权精确协议、主机、端口；私网、HTTP、自定义端口、鉴权音频和封面可用；跨 origin 跳转拒绝；WebDAV 方法与二进制请求受相同授权约束 |
| 任意网络 | `network:["*"]` 默认不放开任意地址。用户可对当前内容显式允许任意互联网/局域网服务器，文件变化后失效；授权携带页面的文件指纹，过期授权拒绝 |
| 外部程序 | 显式授权后按声明的命令运行，支持 stdin、逐行 stdout、stderr、等待和终止；单 VM 最多 4 个活动进程、64 个句柄，全局最多 16 个活动进程；辅助程序必须保持在原进程组内且不能自行驻留；普通调用结束和撤权清理受管进程组，流式辅助程序随播放资源保留 |
| 文件媒体 | 仅允许插件安装目录及独立数据目录内的文件；封面有大小上限；辅助程序生成的音频为当前播放项制作独立副本，防止缓存文件复用干扰当前播放，卸载播放项时清理副本 |
| 音源交接 | 只接受插件提供的真实媒体，交给 Rust 播放器及 AVPlayer；设备控制不作为音源，Spotify 专用取流/解码适配已删除 |
| 推荐和电台 | 原生来源页可打开 home、声明的 browse 导航、推荐歌单和电台；共用现有歌曲/集合页面；FM 队列末尾自动取下一批，暂停、切队列或其他切换操作取消迟到结果 |
| 产品接口 | Rust 定义规范化页面和媒体；移除通用 invoke。UI/MCP 独立选择产品能力，运行时导出不自动开放；本地心形和歌单编辑保持本地操作 |
| 音质 | 标准/高音质/无损/Hi-Res 分别传给插件；保留请求档位与实际编码的区别，未获得 FLAC/Hi-Res 的实测不冒称通过 |

可信标识不会授予进程或任意网络权限。安装不执行第三方辅助程序。运行下载器、FFmpeg、Python、Apple Music helper 等仍需要相应程序实际存在且支持当前系统；账号条件、平台接口和系统授权由各平台决定。脱离原进程组的后台服务不受支持，撤权无法保证停止它们。不完整的源码回退包不会覆盖现有安装，含辅助程序的来源必须提供完整包。

## 仍未支持或未实际验收

- `e5u/mygo-taskbar-lyrics` 需要 Windows 任务栏及持续后台扩展机制，最终安装器明确拒绝。Discord 插件也面向 Windows；能安装或读取页面不代表 macOS 出现 Discord 状态。LRCLib 的上游本地曲库格式与 MusicX 本地库没有自动迁移。
- 主机不加载 npm/Node 模块；每次调用仍使用独立 VM，依赖模块全局变量跨调用维持 daemon/watch/background hook 的插件不能视为完整支持。一般辅助进程 I/O 和播放 lease 已验证，第三方长期后台 daemon 尚未逐一验收。
- 真实 Spotify/Apple Music 登录和 Premium/订阅设备控制、真实 NAS/自建音乐服务器、第三方 helper 完整登录、评论/云端歌单写入、会员付费整曲和所有曲目的完整性未做真实账号验收。
- 本轮音频样本解码通过不等于完整听完。独立 AVPlayer 代理回归使用本机合成音频并静音，不证明第三方服务实际可听。没有替换或迁移本机已安装版本和账号。

## 验证方式

75 项重新安装使用独立空数据目录，不复用用户登录态。匿名只读运行调用 932 次，另外对 18 个已探测样本做两秒 FFmpeg 解码，16 个通过；Faye 和 Arteiimis 补测遇到网易云 -460 风控。9 个原可信版本和 10 个新增版本均经 GitHub 正常安装链路确认可信标识生效。原始测试及新证据一起保留，服务瞬时失败不会被写成安全结论。

真实 macOS QA 使用 `MusicX Plugin QA` / `local.musicx.pluginqa0aeb`，独立监听地址和数据目录。已实际观察 QQ 可信标识及 60 秒限制、Spotify 未验证状态与设备/进程说明、网易云推荐首页及歌单歌曲列表。安装版 MusicX 同时保留。随后重建图标版本，原生来源列表和 QQ 详情均显示圆圈勾号，保留无障碍名称和验证说明。

评审修复后的工作区测试有 168 项通过，另有自定义 headless UI 检查通过，2 项需显式开启的测试保留忽略。严格 Clippy、格式、iOS locked metadata 检查以及 47 项发布脚本测试通过。重建隔离原生测试版后，再次观察了 QQ 的可信解释与 60 秒试听说明，以及 Spotify 的未验证、设备发声和前台辅助程序限制。源码/运行时模拟、真实第三方匿名调用、原生窗口、原生合成音频和真实账号/设备是分别记录的证据。

[原始兼容性与同平台比较](2026-11-15-plugin-audit.md)保留修复前基线。[本轮逐项目与逐调用摘要](2026-11-15-plugin-compatibility.json)包含版本和结果；[可信证据登记](../../crates/mygo-plugin/src/verified_plugins.json)包含精确内容摘要。本机原始日志位于 `/tmp/musicx-plugin-audit-20261115-0aeb/`，不随 Git 提交。

## 可信版本

| 仓库 | 版本 | 样本限制 |
| --- | --- | --- |
| [GentleM4dman/mygo-music-bilibili](https://github.com/GentleM4dman/mygo-music-bilibili) | 0.4.2 | 音频分段短于合集总时长 |
| [hubandad/mygo-music-netease](https://github.com/hubandad/mygo-music-netease) | 1.0.0 | 整曲/账号仍取决于平台 |
| [djyde/mygo-lizhi](https://github.com/djyde/mygo-lizhi) | 1.1.0 | 整曲/账号仍取决于平台 |
| [amqsy/mygo-music-bodian](https://github.com/amqsy/mygo-music-bodian) | 0.3.2 | 整曲/账号仍取决于平台 |
| [VenusLuna/mygo-music-netease-plus](https://github.com/VenusLuna/mygo-music-netease-plus) | 0.2.6 | 整曲/账号仍取决于平台 |
| [nimrc/mygo-chensheng](https://github.com/nimrc/mygo-chensheng) | 1.0.0 | 整曲/账号仍取决于平台 |
| [dddaniel1/mygo-plugin-qqmusic](https://github.com/dddaniel1/mygo-plugin-qqmusic) | 0.2.4 | 60 秒试听 |
| [AsperaUlt/mygo-netease](https://github.com/AsperaUlt/mygo-netease) | 1.0.14 | 整曲/账号仍取决于平台 |
| [jockcc/mygo-music-radio](https://github.com/jockcc/mygo-music-radio) | 0.1.0 | 整曲/账号仍取决于平台 |
| [sogud/mygo-music-netease](https://github.com/sogud/mygo-music-netease) | 0.1.12 | 整曲/账号仍取决于平台 |
| [AkinoKaede/mygo-music-netease-cloud-music](https://github.com/AkinoKaede/mygo-music-netease-cloud-music) | 0.1.13 | 整曲/账号仍取决于平台 |
| [SixWa12138/mygo-music-netease-music](https://github.com/SixWa12138/mygo-music-netease-music) | 0.4.0 | 试听秒数：45 |
| [specialhua/mygo-ximalaya](https://github.com/specialhua/mygo-ximalaya) | 0.1.4 | 整曲/账号仍取决于平台 |
| [GYRYEZ/mygo-music-roamdial](https://github.com/GYRYEZ/mygo-music-roamdial) | 0.1.0 | 整曲/账号仍取决于平台 |
| [craigary/mygo-music-kugou-lite](https://github.com/craigary/mygo-music-kugou-lite) | 0.8.1 | 整曲/账号仍取决于平台 |
| [Jimmyzzt/mygo-music-incompetech](https://github.com/Jimmyzzt/mygo-music-incompetech) | 0.2.2 | 整曲/账号仍取决于平台 |
| [qile222/mygo-music-qishui](https://github.com/qile222/mygo-music-qishui) | 0.9.8 | 整曲/账号仍取决于平台 |
| [fansion314/mygo-music-yoi](https://github.com/fansion314/mygo-music-yoi) | 0.1.1 | 整曲/账号仍取决于平台 |
| [andy8401/mygo-ximalaya-web-plugin](https://github.com/andy8401/mygo-ximalaya-web-plugin) | 0.1.1 | 整曲/账号仍取决于平台 |

## 全量复测摘要

| 仓库 | 版本 | 最终安装 | 匿名功能观察 |
| --- | --- | --- | --- |
| [sogud/mygo-music-netease](https://github.com/sogud/mygo-music-netease) | 0.1.12 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [GentleM4dman/mygo-music-bilibili](https://github.com/GentleM4dman/mygo-music-bilibili) | 0.4.2 | 通过 | 有版本绑定的可信证据 |
| [egoist/mygo-music-youtube-music](https://github.com/egoist/mygo-music-youtube-music) | 0.1.1 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [AkinoKaede/mygo-music-netease-cloud-music](https://github.com/AkinoKaede/mygo-music-netease-cloud-music) | 0.1.13 | 通过 | 歌曲搜索 50 条；2 秒解码通过；有版本绑定的可信证据 |
| [SixWa12138/mygo-music-netease-music](https://github.com/SixWa12138/mygo-music-netease-music) | 0.4.0 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [lane128/mygo-music-qishui](https://github.com/lane128/mygo-music-qishui) | 1.0.15 | 通过 | 请先登录汽水音乐。 |
| [hubandad/mygo-music-netease](https://github.com/hubandad/mygo-music-netease) | 1.0.0 | 通过 | 歌曲搜索 50 条；2 秒解码通过；有版本绑定的可信证据 |
| [DaviRain-Su/mygo-music-qishui](https://github.com/DaviRain-Su/mygo-music-qishui) | 0.5.2 | 通过 | 请先登录汽水音乐。 |
| [bitbyte404/mygo-music-kugou-lite](https://github.com/bitbyte404/mygo-music-kugou-lite) | 1.3.2 | 通过 | 歌曲搜索 30 条 |
| [djyde/mygo-lizhi](https://github.com/djyde/mygo-lizhi) | 1.1.0 | 通过 | 歌曲搜索 7 条；2 秒解码通过；有版本绑定的可信证据 |
| [yunforge/mygo-music-download](https://github.com/yunforge/mygo-music-download) | 0.2.0 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [r0k1s-i/mygotify](https://github.com/r0k1s-i/mygotify) | 0.1.0 | 通过 | 歌曲搜索 20 条 |
| [LYJW131/mygo-music-apple-music](https://github.com/LYJW131/mygo-music-apple-music) | 0.3.5 | 通过 | 歌曲搜索 25 条 |
| [Amamiya23/mygo-music-qq-plugin](https://github.com/Amamiya23/mygo-music-qq-plugin) | 0.3.0 | 通过 | QQ Music DoSearchForQQMusicDesktop: 0/2001 |
| [shensl4499/mygo-music-xiaoyuzhou](https://github.com/shensl4499/mygo-music-xiaoyuzhou) | 0.1.0 | 通过 | 请先连接小宇宙账号 |
| [KirishimaRin/mygo-music-netease-cloud-music](https://github.com/KirishimaRin/mygo-music-netease-cloud-music) | 0.1.7 | 通过 | 连接网易云音乐失败，请重试。 |
| [amqsy/mygo-music-bodian](https://github.com/amqsy/mygo-music-bodian) | 0.3.2 | 通过 | 歌曲搜索 30 条；有版本绑定的可信证据 |
| [Lorpaves/mygo-music-qqmusic](https://github.com/Lorpaves/mygo-music-qqmusic) | 0.3.5 | 通过 | 歌曲搜索 20 条 |
| [yusgsysh/mygo-music-fnos-plugin](https://github.com/yusgsysh/mygo-music-fnos-plugin) | 1.1.2 | 通过 | Set your fnOS username and password in the plugin's settings. |
| [InkCitrus/mygo-music-apple-music](https://github.com/InkCitrus/mygo-music-apple-music) | 0.2.7 | 通过 | 歌曲搜索 20 条 |
| [xingkaixin/mygo-music-spotify](https://github.com/xingkaixin/mygo-music-spotify) | 0.1.1 | 通过 | Error |
| [VenusLuna/mygo-music-netease-plus](https://github.com/VenusLuna/mygo-music-netease-plus) | 0.2.6 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [starriv/mygo-music-spotify](https://github.com/starriv/mygo-music-spotify) | 1.0.0 | 通过 | 请填写你的 Spotify 应用的 Client ID。 |
| [benhaotang/mygo-music-Ampache](https://github.com/benhaotang/mygo-music-Ampache) | 0.2.0 | 通过 | Set the Ampache URL, username and API password first |
| [yiyiSmallti/mygo-qishui](https://github.com/yiyiSmallti/mygo-qishui) | 0.6.13 | 通过 | 请先登录汽水音乐账号。 |
| [LYJW131/mygo-music-emby-jellyfin](https://github.com/LYJW131/mygo-music-emby-jellyfin) | 0.1.0 | 通过 | 请在插件设置里填写服务器地址。 |
| [popeyelau/mygo-queyue](https://github.com/popeyelau/mygo-queyue) | 0.1.0 | 通过 | 暂未登录或token已经过期 |
| [ZONGZI-000/zongzi-mygo-plugin-stable](https://github.com/ZONGZI-000/zongzi-mygo-plugin-stable) | 1.1.0 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [nimrc/mygo-chensheng](https://github.com/nimrc/mygo-chensheng) | 1.0.0 | 通过 | 有版本绑定的可信证据 |
| [louisgreen0726/mygo-music-pocket-casts](https://github.com/louisgreen0726/mygo-music-pocket-casts) | 0.1.2 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [Guyu23/mygo-music-qqmusic](https://github.com/Guyu23/mygo-music-qqmusic) | 0.1.4 | 通过 | 歌曲搜索 20 条 |
| [hiSlowMo/neteaseXqqmusic](https://github.com/hiSlowMo/neteaseXqqmusic) | 0.1.5 | 通过 | 歌曲搜索 50 条 |
| [TexasOct/mygo-subsonic](https://github.com/TexasOct/mygo-subsonic) | 0.1.10 | 通过 | 服务器地址、用户名或密码尚未填全。 |
| [Faye-bit/MyGo-Music-Netease-All](https://github.com/Faye-bit/MyGo-Music-Netease-All) | 0.3.1 | 通过 | 歌曲搜索 30 条；取流通过，本次解码补测受风控 |
| [specialhua/mygo-ximalaya](https://github.com/specialhua/mygo-ximalaya) | 0.1.4 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [LANMIN-X/mygo-plugin-ximalaya](https://github.com/LANMIN-X/mygo-plugin-ximalaya) | 0.1.7 | 通过 | 歌曲搜索 20 条 |
| [biaobiaobiao108/mygo-yun-music](https://github.com/biaobiaobiao108/mygo-yun-music) | 1.1.1 | 通过 | 请在插件设置中填写服务器地址。 |
| [GYRYEZ/mygo-music-roamdial](https://github.com/GYRYEZ/mygo-music-roamdial) | 0.1.0 | 通过 | 歌曲搜索 60 条；2 秒解码通过；有版本绑定的可信证据 |
| [AceYKN/mygo-music-apple-podcasts](https://github.com/AceYKN/mygo-music-apple-podcasts) | 0.2.0-preview.2 | 通过 | 请在插件设置中允许苹果播客启动辅助程序。 |
| [Arteiimis/mygo-music-netease](https://github.com/Arteiimis/mygo-music-netease) | 0.2.1 | 通过 | 歌曲搜索 50 条；取流通过，本次解码补测受风控 |
| [iko233/webdav4mygo](https://github.com/iko233/webdav4mygo) | 0.3.5 | 通过 | 请在插件设置中填写 WebDAV 音乐目录地址。 |
| [Kagura114514/mygo-music-discord-presence-addon](https://github.com/Kagura114514/mygo-music-discord-presence-addon) | 0.1.7 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [dddaniel1/mygo-plugin-qqmusic](https://github.com/dddaniel1/mygo-plugin-qqmusic) | 0.2.4 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [craigary/mygo-music-kugou-lite](https://github.com/craigary/mygo-music-kugou-lite) | 0.8.1 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [ZONGZI-000/zongzi-mygo-plugin-beta](https://github.com/ZONGZI-000/zongzi-mygo-plugin-beta) | 1.1.0-beta.20 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [AdkinsHan/mygo-local-lyrics](https://github.com/AdkinsHan/mygo-local-lyrics) | 1.1.1 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [dddaniel1/mygo-plugin-kugou](https://github.com/dddaniel1/mygo-plugin-kugou) | 0.1.0 | 通过 | 歌曲搜索 30 条 |
| [hhym/mygo-music-hulumusic-plugin](https://github.com/hhym/mygo-music-hulumusic-plugin) | 2.0.1 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [Guyu23/mygo-music-qishui](https://github.com/Guyu23/mygo-music-qishui) | 0.1.0 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [liuxiaofengone/synology-DSAudio-mygo-music-plugin](https://github.com/liuxiaofengone/synology-DSAudio-mygo-music-plugin) | 0.1.1 | 通过 | 请先登录群晖 Audio Station |
| [hhym/mygo-music-qingtingfm-main](https://github.com/hhym/mygo-music-qingtingfm-main) | 1.7.0 | 通过 | 歌曲搜索 20 条 |
| [GYRYEZ/mygo-music-douyin](https://github.com/GYRYEZ/mygo-music-douyin) | 0.8.7 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [jfeng018/mygo-music-lx-source](https://github.com/jfeng018/mygo-music-lx-source) | 0.2.2 | 通过 | 曲库 A: Plugin network permissions do not cover this address；曲库 B: Plugin network  |
| [hhym/mygo-music-qqmusic-plugin](https://github.com/hhym/mygo-music-qqmusic-plugin) | 0.5.1 | 通过 | 歌曲搜索 20 条 |
| [ivanxguo/mygo-music-bitrate](https://github.com/ivanxguo/mygo-music-bitrate) | 1.0.0 | 通过 | 歌曲搜索 48 条 |
| [satyawikananda/soundgo](https://github.com/satyawikananda/soundgo) | 0.1.0 | 通过 | 歌曲搜索 20 条 |
| [Jimmyzzt/mygo-music-incompetech](https://github.com/Jimmyzzt/mygo-music-incompetech) | 0.2.2 | 通过 | 歌曲搜索 4 条；2 秒解码通过；有版本绑定的可信证据 |
| [AsperaUlt/mygo-netease](https://github.com/AsperaUlt/mygo-netease) | 1.0.14 | 通过 | 歌曲搜索 50 条；2 秒解码通过；有版本绑定的可信证据 |
| [TengYangTuoHai/mygo-music-navidrome](https://github.com/TengYangTuoHai/mygo-music-navidrome) | 0.2.0 | 通过 | Enter the server URL, username and password in plugin settings |
| [YeeYeung/mygo-music-overcast](https://github.com/YeeYeung/mygo-music-overcast) | 0.1.0 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [fcmfcm1999/mygo-music-kugou](https://github.com/fcmfcm1999/mygo-music-kugou) | 1.0.0 | 通过 | 歌曲搜索 30 条 |
| [sounfury/mygo-music-spotify-soloist](https://github.com/sounfury/mygo-music-spotify-soloist) | 0.1.0 | 通过 | 请登录 Spotify 以加载音乐库。 |
| [andy8401/mygo-music-cnradio](https://github.com/andy8401/mygo-music-cnradio) | 1.1.0 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [qile222/mygo-music-qishui](https://github.com/qile222/mygo-music-qishui) | 0.9.8 | 通过 | 歌曲搜索 20 条；2 秒解码通过；有版本绑定的可信证据 |
| [MaYunFei/mygo-music-feiniu](https://github.com/MaYunFei/mygo-music-feiniu) | 0.1.0 | 通过 | 请先登录飞牛音乐。 |
| [jockcc/mygo-music-radio](https://github.com/jockcc/mygo-music-radio) | 0.1.0 | 通过 | 歌曲搜索 50 条；2 秒解码通过；有版本绑定的可信证据 |
| [moewah/mygo-music-emby](https://github.com/moewah/mygo-music-emby) | 0.3.1 | 通过 | 请登录 Emby 服务器 |
| [rocona/mygo-navidrome](https://github.com/rocona/mygo-navidrome) | 0.1.0 | 通过 | 请在插件设置中填写 Navidrome 用户名和密码。 |
| [eutleek/mygo-radio](https://github.com/eutleek/mygo-radio) | 1.2.0 | 通过 | 已安装；需配置、登录或通过推荐/浏览使用，详见逐调用结果 |
| [Maker-World/MygoMusicMigu](https://github.com/Maker-World/MygoMusicMigu) | 0.2.3 | 通过 | 歌曲搜索 20 条 |
| [pingfury108/mygo-music-sonust](https://github.com/pingfury108/mygo-music-sonust) | 0.1.1 | 通过 | 请在插件设置里填写服务器和 API 密钥。 |
| [fansion314/mygo-music-yoi](https://github.com/fansion314/mygo-music-yoi) | 0.1.1 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [hx23840/mygo-apple-music](https://github.com/hx23840/mygo-apple-music) | 0.3.3 | 通过 | 无法启动 Apple Music。请安装适用于本机的完整插件包，并在设置 > 插件中允许启动辅助程序。系统需要 macOS 14 或更新版本。 |
| [andy8401/mygo-ximalaya-web-plugin](https://github.com/andy8401/mygo-ximalaya-web-plugin) | 0.1.1 | 通过 | 歌曲搜索 30 条；2 秒解码通过；有版本绑定的可信证据 |
| [e5u/mygo-taskbar-lyrics](https://github.com/e5u/mygo-taskbar-lyrics) | 0.8.2 | 未支持 | Background extensions are not supported by this host |

## 评审回归补充

- 推荐页复用完整的歌单解析路径，支持嵌套 `playlist`；FM 顶层数组和 `tracks` 包装均规范化。自动补曲延用有界跳过失败曲目的逻辑，补曲期间入队会重新推进现有后继。
- 电台批次只剩一首时，桌面/iOS 下一首按钮与桌面菜单、快捷键允许手动补曲；普通有限队列末尾和切歌期间保持禁用。真实组件 headless 回归在修复前失败、修复后通过。
- 远程播放先应用支持的音量控制，启动成功后才标记已开始；失败后重试选中曲目。空状态和非 seek 控制保留已流逝进度，缺少可选音量/seek 不变成播放错误。新增实际 JavaScript 导出的隔离适配器回归，未代替真实设备验收。
- 目录中的可信状态重新读取安装目录的 manifest，避免旧目录数据掩盖本地权限修改。权限绑定页面指纹；已安装详情和指纹来自同一安装快照，并与安装切换、启动、撤权串行；macOS 文件名大小写别名不能覆盖设置、安装记录或主文件。
- fetch 取消回收请求槽并跳过排队工作。下载回退明确区分完整/不完整资产，拒绝用不完整回退替换现有安装；前台辅助程序不支持自行脱离进程组的后台服务。
- 后续回归修复了远程音量失败误暂停、选曲启动失败后的旧设备停止、空闲超时误清除错误，以及设备自行恢复后的暂停/退出、FM 自动切歌期间的等待定位，以及插件更新并重新授权后复用旧程序声明的问题。辅助程序在 VM 绑定及实际启动前都检查当前 manifest。
