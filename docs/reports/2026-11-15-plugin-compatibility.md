# 插件可信标识与兼容能力补齐

日期：2026-11-15，macOS arm64。基于 `dfc9435107d70c90b76557dc17afab7d26d773de` 后的实现，MyGo 能力版本门槛为 `0.1.24`。这是能力适配边界，不代表完整复刻上游应用。

最终结果：75 个目录项目中，74 个通过最终安装/脚本检查；Windows 任务栏歌词后台扩展被明确拒绝。34 个项目返回歌曲搜索行（包含码率信息扩展），18 个有音频包探测成功，16 个通过本轮前两秒解码。保留此前已验证证据后，共 19 个具体版本标记可信；19 个版本均重新从 GitHub 安装并通过内容匹配。

## 可信的含义

- 可信表示列出的版本和能力有实际调用及短音频解码证据，不是安全审计、作者身份认证、账号/会员资格保证或所有曲目整曲保证。
- 绑定仓库、插件 ID、版本、规范化 manifest、安装后的脚本、语言文件及全部附带文件摘要。插件自己声明的 verification 字段无效。变更权限、代码、版本、仓库、语言文件或附带文件都会失去对应证据；用户配置和账号 storage 独立保存。
- 目录未检查最新包时显示“可信版本 X”；已安装内容完全一致时显示“可信”。未知版本不继承旧版本可信。
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
| 远程播放 | 适配 remotePlay/Pause/Resume/State/Seek/Volume 和设备接口，RPC 留在后台；旧会话不能暂停新会话；Spotify 等由选中设备发声，需要真实账号与设备另验收 |
| 推荐和电台 | 原生来源页可打开 home、声明的 browse 导航、推荐歌单和电台；共用现有歌曲/集合页面；FM 队列末尾自动取下一批，暂停、切队列或其他切换操作取消迟到结果 |
| 公共接口 | MCP `manage_music_sources` 的 invoke 白名单开放建议、评论、喜欢、收藏、订阅、历史及云端歌单增删改排序；page 返回规范化原生页；内部辅助导出不暴露为 RPC；本地心形和本地歌单仍是本地操作 |
| 音质 | 标准/高音质/无损/Hi-Res 分别传给插件；保留请求档位与实际编码的区别，未获得 FLAC/Hi-Res 的实测不冒称通过 |

可信标识不会授予进程或任意网络权限。安装不执行第三方辅助程序。运行下载器、FFmpeg、Python、Apple Music helper 等仍需要相应程序实际存在且支持当前系统；账号条件、平台接口和系统授权由各平台决定。脱离原进程组的后台服务不受支持，撤权无法保证停止它们。不完整的源码回退包不会覆盖现有安装，含辅助程序的来源必须提供完整包。

## 仍未支持或未实际验收

- `e5u/mygo-taskbar-lyrics` 需要 Windows 任务栏及持续后台扩展机制，最终安装器明确拒绝。Discord 插件也面向 Windows；能安装或读取页面不代表 macOS 出现 Discord 状态。LRCLib 的上游本地曲库格式与 MusicX 本地库没有自动迁移。
- 主机不加载 npm/Node 模块；每次调用仍使用独立 VM，依赖模块全局变量跨调用维持 daemon/watch/background hook 的插件不能视为完整支持。一般辅助进程 I/O 和播放 lease 已验证，第三方长期后台 daemon 尚未逐一验收。
- 真实 Spotify/Apple Music 登录和 Premium/订阅设备控制、真实 NAS/自建音乐服务器、第三方 helper 完整登录、评论/云端歌单写入、会员付费整曲和所有曲目的完整性未做真实账号验收。
- 本轮音频样本解码通过不等于完整听完。独立 AVPlayer 代理回归使用本机合成音频并静音，不证明第三方服务实际可听。没有替换或迁移本机已安装版本和账号。

## 验证方式

75 项重新安装使用独立空数据目录，不复用用户登录态。匿名只读运行调用 932 次，另外对 18 个已探测样本做两秒 FFmpeg 解码，16 个通过；Faye 和 Arteiimis 补测遇到网易云 -460 风控。9 个原可信版本和 10 个新增版本均经 GitHub 正常安装链路确认可信标识生效。原始测试及新证据一起保留，服务瞬时失败不会被写成安全结论。

真实 macOS QA 使用 `MusicX Plugin QA` / `local.musicx.pluginqa0aeb`，独立监听地址和数据目录。已实际观察 QQ 可信标识及 60 秒限制、Spotify 未验证状态与设备/进程说明、网易云推荐首页及歌单歌曲列表。安装版 MusicX 同时保留。

评审修复后的工作区测试有 164 项通过，另有自定义 headless UI 检查通过，2 项需显式开启的测试保留忽略。严格 Clippy、格式、iOS locked metadata 检查以及 47 项发布脚本测试通过。重建隔离原生测试版后，再次观察了 QQ 的可信解释与 60 秒试听说明，以及 Spotify 的未验证、设备发声和前台辅助程序限制。源码/运行时模拟、真实第三方匿名调用、原生窗口、原生合成音频和真实账号/设备是分别记录的证据。

[原始兼容性与同平台比较](2026-11-15-plugin-audit.md)保留修复前基线。[本轮逐项目与逐调用摘要](2026-11-15-plugin-compatibility.json)包含版本和结果；[可信证据登记](../../crates/plugins/src/verified_plugins.json)包含精确内容摘要。本机原始日志位于 `/tmp/musicx-plugin-audit-20261115-0aeb/`，不随 Git 提交。

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
- 远程播放先应用支持的音量控制，启动成功后才标记已开始；失败后重试选中曲目。空状态和非 seek 控制保留已流逝进度，缺少可选音量/seek 不变成播放错误。新增实际 JavaScript 导出的隔离适配器回归，未代替真实设备验收。
- 目录中的可信状态重新读取安装目录的 manifest，避免旧目录数据掩盖本地权限修改。权限绑定页面指纹；已安装详情和指纹来自同一安装快照，并与安装切换、启动、撤权串行；macOS 文件名大小写别名不能覆盖设置、安装记录或主文件。
- fetch 取消回收请求槽并跳过排队工作。下载回退明确区分完整/不完整资产，拒绝用不完整回退替换现有安装；前台辅助程序不支持自行脱离进程组的后台服务。
