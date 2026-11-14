# MusicX 插件兼容性与同平台比较

> 这是修复前的历史基线。后续可信标识、宿主功能补齐及最终复测结果见[兼容能力补齐报告](2026-11-15-plugin-compatibility.md)。

测试日期：2026-11-15。MusicX 源码：`dfc9435107d70c90b76557dc17afab7d26d773de`，macOS arm64，MyGo 兼容基线 `0.1.22`。

结论：目录中发现的仓库不等于 MusicX 已支持的插件。75 个仓库中，34 个通过安装/脚本检查，39 个被实际安装校验拒绝，2 个下载包超过 16 MiB。通过安装的插件仍有基础 Web API、缓存大小、服务端风控及账号权限问题。当前应按“已验证的具体版本和能力”选择插件。

## 测试范围与证据

- 范围为本次 MusicX 目录快照的 75 个仓库，包含同平台替代实现及 4 个界面/歌词/码率扩展，不是 75 个独立音乐平台。
- 选择方式与当前安装器一致：`releases/latest` 返回的 Release 的 macOS arm64 ZIP、通用 ZIP，或对应源码 ZIP；没有 Release 时固定到 HEAD commit。49 个仓库从这个接口选包，26 个使用固定 commit。是否预发布按 GitHub Release 标记判断，不按版本名称中的 preview 字样判断。
- 73 个下载包提取了宿主实际保留的 manifest、入口脚本和支持的语言文件，并逐个使用真实 `mygo::install` 校验。每个仓库使用独立数据目录，避免相同插件 ID 冲突。其余资源和外部程序不会被 MusicX 安装。
- 另走实际 GitHub 仓库安装链路验证 7 例：sogud、VenusLuna、Bilibili 安装成功；截图中的 Spotify、YOI 被拒绝；NetEaseXQQMusic、网易云 All 下载超限。首次 GitHub 请求曾返回 403，后续重试恢复，单独计为瞬时下载故障。
- 对可安装插件测试五类搜索及分页，再按返回实体测试专辑、歌手、歌单、播客页；按导出接口读取账号状态、歌单、喜欢、最近播放、节目列表和歌词。未执行云端喜欢、删改歌单、上传或订阅写操作。
- 匿名测试共 415 次运行调用。17 个仓库至少一次返回歌曲/单集搜索行，其中包含码率扩展的信息行；11 个仓库有音频包探测成功，9 个进一步完成前 2 秒 FFmpeg 解码。QQ 一例明确只有 60 秒试听。**探测、短片段解码、原生 AVPlayer 播放、完整听完和实际听见声音是不同证据**；本次没有原生界面、AVPlayer、拖动和可听输出验收。
- 主测试不带账号。另通过现有 Music MCP 对本机 5 个已安装版本做只读账号/搜索检查，没有替换它们：波点 0.1.0、酷狗 1.3.0、网易云 0.1.6、小宇宙 0.1.0 已连接账号并搜索成功；喜马拉雅 0.1.6 未连接账号，搜索报 `Invalid URL`。
- 宿主回归：`cargo test --locked -p musicx-plugins`，56 项通过、2 项 opt-in 测试忽略；`cargo fmt --all -- --check` 和测试驱动编译通过。宿主回归通过不代表第三方插件实测通过。

逐调用计数、错误、样本标识、权限、版本及 SHA256 见 [结构化结果](2026-11-15-plugin-audit.json)。原始快照、包和日志保留于本机 `/tmp/musicx-plugin-audit-20261115-0aeb/`；该临时目录不会随 Git 提交。

## 1. 我们的宿主遇到了什么问题

### 安装阶段

下表按安装器首个阻断原因互斥统计。一个插件可能同时需要外部程序、动态网络和更高版本。

| 原因 | 数量 | 典型例子及影响 |
| --- | ---: | --- |
| 外部程序声明不在受限 FFmpeg 白名单 | 17 | YouTube Music、MyGotify、多个汽水插件、Apple Music helper；部分只额外声明 Windows/其他 FFmpeg 路径也被整组拒绝 |
| 动态网络权限 `network:["*"]` | 12 | 飞牛、Ampache、Subsonic、Navidrome 等自建服务；宿主不接受任意服务地址/内网访问 |
| `playback:"remote"` | 5 | 截图 Spotify、其他 Spotify Connect/Soloist、两个 Apple Music；宿主只实现本地 stream |
| 最低 MyGo 版本高于 0.1.22 | 4 | 两个 Zongzi 需要 0.1.23；craigary 酷狗和 moewah Emby 需要 0.1.24 |
| 包超过 16 MiB | 2 | hiSlowMo 的发布 ZIP 为 17,528,886 字节；Faye-bit 的默认分支源码 ZIP 也超限 |
| 插件 ID 格式 | 1 | YOI 使用含点的 `world.fansionia.mygo-music-yoi`，宿主不接受 |

截图插件 [xingkaixin/mygo-music-spotify](https://github.com/xingkaixin/mygo-music-spotify) 0.1.1 的 manifest 是 `playback:"remote"`，还声明了 librespot。其 Spotify Connect 方案要求 Premium 和在线播放设备。本次本地安装重复在 3–4 毫秒内拒绝，实际仓库安装也得到相同拒绝。**这是宿主能力边界，尚未执行登录或验证 Spotify 账号，不是网络或账户登录错误。**

目录默认只做 GitHub Topic 发现，未读取 manifest 时所有发现条目都能显示“添加”。安装时又把具体限制压成 `This source is not supported by the current host`。因此页面出现“添加时检查兼容性”，随后只有笼统报错。应在详情展示具体阻断原因，并区分未经检查、部分支持和已验证。

### 运行阶段：已确认的公共兼容缺口

| 缺口 | 最小复现 | 受影响实测 |
| --- | --- | --- |
| `new URL(relative, base)` 不支持 | `new URL('/search','https://api.example.com')` → `Invalid URL` | Guyu23 汽水、三款喜马拉雅 |
| `URL` 对象缺字段 | `pathname`、`origin`、`search`、`searchParams` 都是 `undefined` | QQ 搜索读 `searchParams.has` 失败；喜马拉雅也使用路径和参数属性 |
| `URLSearchParams` 不完整 | 缺 `get`、`has`；字符串构造把字符序号当参数名 | 依赖标准 Web URL 操作的插件无法按预期工作 |
| 缺 UTF-8 Web API | `TextEncoder` / `TextDecoder` → 未定义 | SixWa 网易云模块无法初始化；bitbyte404 酷狗、Amamiya23 / Lorpaves / hhym QQ 歌词失败 |
| fetch 响应上限 2 MiB | RoamDial 搜索 → `Plugin response exceeds 2 MiB` | 电台目录体积超过上限 |
| 插件持久存储上限 64 KiB | 存 63,000 字符成功；65,536 字符失败 | Incompetech 把完整曲库缓存写入 storage 时失败，另有瞬时网络错误 |

这些最小复现只运行在隔离数据目录中，未修改产品兼容代码。响应/缓存上限是有意的资源边界，处理方式可以是分页、缓存设计或有界扩容；不能仅靠放开任意进程和网络解决。

还有三个容易误判的问题：

- **服务端限制**：网易云 `-460`、Bilibili 拒绝请求、SoundCloud client id 提取失败、酷狗参数/签名错误，不应全部归因于宿主。AkinoKaede 的五类搜索返回 400，KirishimaRin 返回连接失败，尚未完整定位到第三方接口或兼容层。
- **返回地址不等于整曲**：dddaniel1 QQ 的《发如雪》目录为 299 秒，实际音频为 60 秒试听。Bilibili 样本目录为 25,142 秒的合集，但音频为 226.835 秒，需核对多 P/分段时长，不能按“100 首无损合集”标题推断质量或完整性。
- **插件 ID 冲突**：12 组替代实现共用 ID，例如 5 个 QQ 插件均为 `qqmusic`，3 个网易云为 `netease`。隔离安装实测第二个 `netease` 被拒绝。相同 ID 不代表同一作者，也不保证账号存储格式兼容。

### 插件能力丰富，但 MusicX 尚未接出的部分

MusicX 当前可承接搜索、流解析、歌词、专辑/歌手、登录、歌单读取/导入、喜欢/最近播放读取及播客。`home`、`browse`、插件导航、推荐页面、FM 连播、评论、云端红心/歌单写入、订阅管理、上传、`remote*` 不会因为插件导出这些函数就自动在 MusicX 可用。详见 [宿主合同](../PLUGINS.md)。

音质也有差异：当前插件通道把 `Hires` 映射为 `lossless`，没有独立传入 `hires`。插件可以在自身设置里覆盖音质，但“插件声称 Hi-Res”不能等同于 MusicX 已独立协商并验证 Hi-Res。匿名同曲测试中，无损请求实际降级为 320 kbps MP3，本次没有验证 FLAC/Hi-Res。

## 2. 同一平台的功能比较

下面的“丰富度”按声明接口与宿主可接出的能力讨论；未登录的私人库、会员音质及云端写入没有记为实测成功。所有结论只对应本次具体版本和网络条件。

| 平台 | 候选比较 | 当前判断 |
| --- | --- | --- |
| 网易云（10） | VenusLuna 0.2.6：五类搜索、歌词、专辑/歌手、歌单、播客通过，支持喜欢导入；没有 `recent` 导出。hubandad 1.0.0 / AsperaUlt 1.0.14：同类公开能力通过，另有 `recent` 导出；hubandad 缺少 `liked`，AsperaUlt 同时提供 liked/recent。Arteiimis 0.2.1：接口更丰富且带 recent/settings，但后续取流遭风控。sogud 0.1.12：较少扩展，标准取流 -460，高音质曾成功。AkinoKaede 0.1.13 / KirishimaRin 0.1.7：搜索失败。SixWa 0.4.0：云盘上传等更丰富，但 TextEncoder 阻断。Faye-bit 0.3.1：包超限。YOI 0.1.1：ID 被拒。 | 优先继续账号验收 VenusLuna；需要 recent 时评估 hubandad / AsperaUlt。不能承诺绕过平台风控。尚不替换本机 sogud 0.1.6。 |
| QQ 音乐（5） | Amamiya23 0.3.0、Lorpaves 0.3.5、hhym 0.5.1：歌曲/专辑/歌手/歌单搜索和页面通过，歌单编辑/评论等扩展丰富；样本播放受账号权益限制，逐字歌词路径缺 TextDecoder。Guyu23 0.1.4：有 recent，但歌曲搜索被 URL.searchParams 阻断。dddaniel1 0.2.4：接口较少，匿名 LRC 和短片段解码成功，实际只给 60 秒。 | 没有完成会员整曲验证，不能选出完整播放赢家。dddaniel1 适合基础浏览验证，不应把试听当整曲；丰富版本需先补 TextDecoder 并做账号验收。 |
| 酷狗（4） | bitbyte404 1.3.2：四类音乐搜索、专辑/歌手页丰富，有 recent、KRC；匿名样本需登录，KRC 缺 TextDecoder。本机已装 1.3.0 已登录搜索正常。dddaniel1 0.1.0：歌曲搜索成功，其余查询为空，样本需登录。craigary 0.8.1：最低 0.1.24 被拒。fcmfcm1999 1.0.0：搜索参数/签名错误。 | 当前先保留已装概念版；bitbyte404 是继续验收的主要候选。自动领 VIP、评论等声明不代表 MusicX 已接出。 |
| Spotify（4） | xingkaixin 0.1.1：Spotify Web API + Connect/librespot。starriv 1.0.0：Connect/Soloist，并有库和歌词。sounfury 0.1.0：Windows Soloist 方案。MyGotify 0.1.0：Spotify 曲库匹配 YouTube Music 音频，歌词额外走 LRCLIB/网易云，依赖 yt-dlp/FFmpeg。 | 当前四个均不可安装。前三者依赖 remote；MyGotify 是另一平台匹配取音，不保证同一母带/版本，且进程依赖被拒。 |
| Apple Music（3） | InkCitrus 0.2.7：官方网页账户、TTML、macOS 原生远程控制。hx23840 0.3.3：原生辅助 App 远程播放。LYJW131 0.3.5：声称含 Hi-Res 的流方案，但需 amhelper/FFmpeg。 | 三个均被当前宿主拒绝。没有会员登录、DRM、整曲或 Hi-Res 实测。 |
| 汽水（5） | lane128 1.0.15 / DaviRain-Su 0.5.2 / yiyiSmallti 0.6.13 / qile222 0.9.8：扫码、推荐、歌词/曲库等丰富，需 helper 或额外进程。Guyu23 0.1.0：不声明额外进程，理论适配成本较低，但四类搜索均被 URL 兼容层阻断。 | 当前没有可直接推荐的可用实现。若只补宿主基础 API，先复验 Guyu23；其余涉及二进制分发和进程协议。 |
| 喜马拉雅（3） | specialhua 0.1.4、andy8401 0.1.1：搜索、节目页、历史。LANMIN-X 0.1.7：另有 lyrics、订阅导出及已购相关浏览。三个均在 URL 处理阶段失败，本机 LANMIN-X 0.1.6 同样失败。 | 先补公共 URL API，再比较已购/订阅/文稿；目前无法据接口丰富度推荐其中一个。 |
| 电台（4） | jockcc 0.1.0：公开搜索 50 条并完成 MP3 短片段解码；home/FM 仍未接出。RoamDial 0.1.0：目录超 2 MiB。andy8401 cnradio 1.1.0：需 Python/FFmpeg。eutleek 1.2.0：只有 home/browse/stations，缺 search。 | 当前只有 jockcc 有已验证的 MusicX 搜索→音频路径；不等于所有电台地址可用。 |
| 自建/自定义服务（12） | 飞牛、Emby/Jellyfin、Ampache、Subsonic/Navidrome、WebDAV、群晖等提供不同的私有曲库、收藏、歌单和歌词接口，通常需要动态域名、私网/自定义端口；部分还需 helper 或 MyGo 0.1.24。 | 当前均未进入账号/服务器功能验收。自有服务器的可控性不应与公共聚合 API 混为一谈，但 MusicX 需要显式的服务器授权模型才能承接。 |

网易云同曲补测统一使用 `5257138`《屋顶》，保留搜索返回的完整 Track 对象，依次请求 standard/high/lossless/hires。VenusLuna、hubandad、AsperaUlt 四档都得到可探测音频，其中后三档仍为 MP3，未获无损；sogud 高音质/映射后的 hires 曾成功，标准和 lossless -460；Arteiimis 在这轮四档均 -460。此前 Arteiimis 标准探测曾成功，因此将其归为不稳定而非永久不可用。

其他平台：Bilibili、波点、李志曲库、陈升曲库已完成短片段解码，具体样本和限制见全量表。小宇宙在匿名环境要求登录，但本机已登录版本可搜单集；Pocket Casts 两个查询均空，不能因此断言平台不支持搜索；Overcast 因动态音频域名权限被拒；雀乐要求登录；SoundCloud client id 解析失败；Incompetech 缓存写入失败；YouTube Music、蜻蜓、抖音、咪咕仍被进程声明阻断。

## 3. 是否值得信任

这次能给出权限和供应链初筛，不能为任何插件出具“已完成安全审计”的结论。没有观察到恶意行为也不等于证明不存在。

| 信任问题 | 本次证据 | 判断 |
| --- | --- | --- |
| 目录是否审核/背书 | 默认目录来自公开 GitHub `topic:mygo-music-plugin archived:false` 搜索 | 出现在目录中只代表可发现，不能视为 MusicX 或音乐平台认证 |
| 代码与依赖是否可检查 | 获取了全部候选的 manifest/入口信息；被阻断的包只做静态扫描，未执行 | 单文件、明确域名、无额外进程的实现较容易复核；并不自动保证安全或服务长期可用 |
| 下载完整性 | 39 个已下载 Release 资产的 GitHub SHA256 digest 与实际 ZIP 相符，其余也记录了本地 SHA256 | 能确认本次包字节一致；摘要和本机安装基线不是独立作者签名，也不证明账号未被接管 |
| 会话隐私 | 宿主按插件保存 JSON storage，登录态不放系统 Keychain；多个上游 README 明确为明文会话 | 连接账号就是把该会话交给该作者的插件。域名清单、登录方式、更新来源和权限变化都需要复核 |
| 维护成熟度 | 这 75 个仓库创建日期均为 2026-11-06 至 11-14 | 只有数日历史，星数不足以证明长期稳定。sogud 星数最多但本轮仍遇到取流风控 |
| 数据来源是否相同 | MyGotify 由 YouTube 匹配 Spotify；download-music 使用 `flac.music.hi.cn`；LX/聚合插件接入更多解析源 | 需要区分平台直连、跨平台匹配和第三方解析，尤其不能把“无损”文案直接当真实编码证明 |
| 广泛权限是否必要 | 部分插件要求 `process:true`、`*`、shell、下载/解包工具或多个外部二进制；自建服务器也会声明 `network:["*"]` | 按功能解释所需权限，不把被拒插件一概判为不可信，也不应整体解除宿主限制 |

目前更适合继续验收的是：明确平台域名、无额外二进制、已能完成公开调用的插件。网易云可优先 VenusLuna，带 recent 需求时看 hubandad/AsperaUlt；酷狗保留现有 bitbyte404；小宇宙保留现有已登录实现。跨平台匹配、第三方解析和额外 helper 应单独评审。此判断不授权替换现有插件或迁移账号。

## 建议的修复顺序

1. 补完整 URL、URLSearchParams、TextEncoder/TextDecoder，并把这次最小失败样本做成宿主回归。影响汽水、喜马拉雅、QQ、酷狗、网易云，收益最直接。
2. 目录预检并展示具体限制；把码率/歌词/桌面扩展与音乐源分开；明确插件 ID 冲突。不要把“导出函数数”显示为“宿主已支持功能数”。
3. 给实际音频探测补试听/分段识别，修正合集总时长与首 P 音频的差异；质量界面保留请求档位与实际返回档位。
4. 为大目录采用分页/有界缓存；再评估资源上限，不直接无界扩容。
5. 独立设计 remote、helper 和自建服务器权限后再扩展对应平台；先做已验证音乐源的账号、会员整曲、原生播放与拖动验收。

本轮交付是测试报告、结构化证据与隔离复测驱动，未修复产品代码。尚未验收：新插件真实扫码/网页登录、会员或付费内容、FLAC/Hi-Res、所有曲目的完整性、云端写操作、原生界面、AVPlayer、seek 和实际声音。

## 全量 75 个仓库

能力列为声明接口的缩写：搜=search，播=stream 或 remotePlay，词=lyrics，专=album，艺=artist，单=playlist/playlists，喜=liked，近=recent，客=show/shows，登=signIn。**声明不等于实测成功**；被拒包的接口来自静态识别。详细的宿主支持/未接出接口分别列在 JSON 中。

<!-- Generated repository table follows. -->

| # | 平台 | 仓库 | 版本 | 声明能力 | 安装及运行结果 | 权限/来源关注 |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | Apple Music | [hx23840/mygo-apple-music](https://github.com/hx23840/mygo-apple-music) | 0.3.3 | 搜 遥播 词 专 艺 单 喜 | 远程播放不支持 | 远程设备控制、外部程序 |
| 2 | Apple Music | [InkCitrus/mygo-music-apple-music](https://github.com/InkCitrus/mygo-music-apple-music) | 0.2.7 | 搜 播 遥播 词 专 艺 单 喜 近 登 | 远程播放不支持 | 远程设备控制、外部程序 |
| 3 | Apple Music | [LYJW131/mygo-music-apple-music](https://github.com/LYJW131/mygo-music-apple-music) | 0.3.5 | 搜 播 词 专 艺 单 喜 近 登 | 外部程序不兼容 | 外部程序 |
| 4 | Apple Podcasts | [AceYKN/mygo-music-apple-podcasts](https://github.com/AceYKN/mygo-music-apple-podcasts) | 0.2.0-preview.2 | 搜 播 喜 近 客 登 | 外部程序不兼容 | 外部程序 |
| 5 | Bilibili | [GentleM4dman/mygo-music-bilibili](https://github.com/GentleM4dman/mygo-music-bilibili) | 0.4.2 | 搜 播 词 专 艺 单 喜 近 登 | 2 秒解码通过；音频时长与目录不一致，需核对多 P/分段 | 外部程序 |
| 6 | Incompetech | [Jimmyzzt/mygo-music-incompetech](https://github.com/Jimmyzzt/mygo-music-incompetech) | 0.2.2 | 搜 播 专 艺 | 曲库缓存超过 64 KiB；另有网络错误 | 额外内容服务 |
| 7 | Overcast | [YeeYeung/mygo-music-overcast](https://github.com/YeeYeung/mygo-music-overcast) | 0.1.0 | 搜 播 喜 近 客 登 | 动态网络权限不支持 | 动态服务地址 |
| 8 | Pocket Casts | [louisgreen0726/mygo-music-pocket-casts](https://github.com/louisgreen0726/mygo-music-pocket-casts) | 0.1.2 | 搜 播 喜 近 客 登 | 匿名结果为空或要求登录；完整能力待账号验证 | 外部程序、额外内容服务 |
| 9 | QQ 音乐 | [Amamiya23/mygo-music-qq-plugin](https://github.com/Amamiya23/mygo-music-qq-plugin) | 0.3.0 | 搜 播 词 专 艺 单 喜 登 | 歌曲搜索通过；样本播放受登录/权限限制；歌词缺 TextDecoder | 明确平台/内容域名 |
| 10 | QQ 音乐 | [dddaniel1/mygo-plugin-qqmusic](https://github.com/dddaniel1/mygo-plugin-qqmusic) | 0.2.4 | 搜 播 词 专 艺 单 喜 登 | 2 秒解码通过；返回 60 秒试听，非整曲 | 明确平台/内容域名 |
| 11 | QQ 音乐 | [Guyu23/mygo-music-qqmusic](https://github.com/Guyu23/mygo-music-qqmusic) | 0.1.4 | 搜 播 词 专 艺 单 喜 近 登 | URL.searchParams 缺失，歌曲搜索失败 | 明确平台/内容域名 |
| 12 | QQ 音乐 | [hhym/mygo-music-qqmusic-plugin](https://github.com/hhym/mygo-music-qqmusic-plugin) | 0.5.1 | 搜 播 词 专 艺 单 喜 登 | 歌曲搜索通过；样本播放受登录/权限限制；歌词缺 TextDecoder | 明确平台/内容域名 |
| 13 | QQ 音乐 | [Lorpaves/mygo-music-qqmusic](https://github.com/Lorpaves/mygo-music-qqmusic) | 0.3.5 | 搜 播 词 专 艺 单 喜 登 | 歌曲搜索通过；样本播放受登录/权限限制；歌词缺 TextDecoder | 明确平台/内容域名 |
| 14 | SoundCloud | [satyawikananda/soundgo](https://github.com/satyawikananda/soundgo) | 0.1.0 | 搜 播 词 专 艺 单 喜 近 登 | 未能解析 SoundCloud client id | 外部程序 |
| 15 | Spotify | [r0k1s-i/mygotify](https://github.com/r0k1s-i/mygotify) | 0.1.0 | 搜 播 词 专 艺 单 喜 登 | 外部程序不兼容 | 外部程序、聚合/第三方解析 |
| 16 | Spotify | [sounfury/mygo-music-spotify-soloist](https://github.com/sounfury/mygo-music-spotify-soloist) | 0.1.0 | 搜 遥播 词 专 艺 单 喜 近 登 | 远程播放不支持 | 远程设备控制、外部程序 |
| 17 | Spotify | [starriv/mygo-music-spotify](https://github.com/starriv/mygo-music-spotify) | 1.0.0 | 搜 遥播 词 专 艺 单 喜 近 客 登 | 远程播放不支持 | 远程设备控制、外部程序 |
| 18 | Spotify | [xingkaixin/mygo-music-spotify](https://github.com/xingkaixin/mygo-music-spotify) | 0.1.1 | 搜 遥播 词 专 艺 单 喜 近 客 登 | 远程播放不支持 | 远程设备控制、外部程序 |
| 19 | YouTube Music | [egoist/mygo-music-youtube-music](https://github.com/egoist/mygo-music-youtube-music) | 0.1.1 | 搜 播 词 专 艺 单 喜 近 登 | 外部程序不兼容 | 外部程序 |
| 20 | 咪咕 | [Maker-World/MygoMusicMigu](https://github.com/Maker-World/MygoMusicMigu) | 0.2.3 | 搜 播 词 专 艺 单 喜 登 | 外部程序不兼容 | 外部程序 |
| 21 | 喜马拉雅 | [andy8401/mygo-ximalaya-web-plugin](https://github.com/andy8401/mygo-ximalaya-web-plugin) | 0.1.1 | 搜 播 近 客 登 | URL 兼容层阻断 | 明确平台/内容域名 |
| 22 | 喜马拉雅 | [LANMIN-X/mygo-plugin-ximalaya](https://github.com/LANMIN-X/mygo-plugin-ximalaya) | 0.1.7 | 搜 播 词 近 客 登 | URL 兼容层阻断 | 明确平台/内容域名 |
| 23 | 喜马拉雅 | [specialhua/mygo-ximalaya](https://github.com/specialhua/mygo-ximalaya) | 0.1.4 | 搜 播 近 客 登 | URL 兼容层阻断 | 明确平台/内容域名 |
| 24 | 小宇宙 | [shensl4499/mygo-music-xiaoyuzhou](https://github.com/shensl4499/mygo-music-xiaoyuzhou) | 0.1.0 | 搜 播 喜 近 客 登 | 匿名结果为空或要求登录；完整能力待账号验证 | 明确平台/内容域名 |
| 25 | 扩展 | [AdkinsHan/mygo-local-lyrics](https://github.com/AdkinsHan/mygo-local-lyrics) | 1.1.1 | 搜 播 词 专 艺 | 外部程序不兼容 | 外部程序 |
| 26 | 扩展 | [e5u/mygo-taskbar-lyrics](https://github.com/e5u/mygo-taskbar-lyrics) | 0.8.2 | 无音乐源接口 | 外部程序不兼容 | 外部程序 |
| 27 | 扩展 | [ivanxguo/mygo-music-bitrate](https://github.com/ivanxguo/mygo-music-bitrate) | 1.0.0 | 搜 | 码率查询扩展；48 条信息行，不是播放源，缺少 stream | 明确平台/内容域名 |
| 28 | 扩展 | [Kagura114514/mygo-music-discord-presence-addon](https://github.com/Kagura114514/mygo-music-discord-presence-addon) | 0.1.7 | 无音乐源接口 | 外部程序不兼容 | 外部程序 |
| 29 | 抖音 | [GYRYEZ/mygo-music-douyin](https://github.com/GYRYEZ/mygo-music-douyin) | 0.8.7 | 播 专 艺 | 外部程序不兼容 | 外部程序 |
| 30 | 李志 | [djyde/mygo-lizhi](https://github.com/djyde/mygo-lizhi) | 1.1.0 | 搜 播 词 专 艺 喜 近 | 2 秒解码通过 | 额外内容服务 |
| 31 | 汽水 | [DaviRain-Su/mygo-music-qishui](https://github.com/DaviRain-Su/mygo-music-qishui) | 0.5.2 | 搜 播 词 专 艺 单 喜 近 登 | 外部程序不兼容 | 外部程序 |
| 32 | 汽水 | [Guyu23/mygo-music-qishui](https://github.com/Guyu23/mygo-music-qishui) | 0.1.0 | 搜 播 词 专 艺 单 喜 近 登 | URL 兼容层阻断 | 明确平台/内容域名 |
| 33 | 汽水 | [lane128/mygo-music-qishui](https://github.com/lane128/mygo-music-qishui) | 1.0.15 | 搜 播 词 专 艺 单 喜 近 登 | 外部程序不兼容 | 外部程序 |
| 34 | 汽水 | [qile222/mygo-music-qishui](https://github.com/qile222/mygo-music-qishui) | 0.9.8 | 搜 播 词 专 艺 单 喜 近 登 | 外部程序不兼容 | 外部程序 |
| 35 | 汽水 | [yiyiSmallti/mygo-qishui](https://github.com/yiyiSmallti/mygo-qishui) | 0.6.13 | 搜 播 词 专 艺 单 喜 登 | 外部程序不兼容 | 外部程序 |
| 36 | 波点 | [amqsy/mygo-music-bodian](https://github.com/amqsy/mygo-music-bodian) | 0.3.2 | 搜 播 词 专 艺 单 喜 登 | 2 秒解码通过 | 明确平台/内容域名 |
| 37 | 电台 | [andy8401/mygo-music-cnradio](https://github.com/andy8401/mygo-music-cnradio) | 1.1.0 | 搜 播 | 外部程序不兼容 | 外部程序、额外内容服务 |
| 38 | 电台 | [eutleek/mygo-radio](https://github.com/eutleek/mygo-radio) | 1.2.0 | 播 | 无 search；依赖 home/browse，MusicX 无对应浏览入口 | 额外内容服务 |
| 39 | 电台 | [GYRYEZ/mygo-music-roamdial](https://github.com/GYRYEZ/mygo-music-roamdial) | 0.1.0 | 搜 播 | 目录响应超过宿主 2 MiB 限制 | 额外内容服务 |
| 40 | 电台 | [jockcc/mygo-music-radio](https://github.com/jockcc/mygo-music-radio) | 0.1.0 | 搜 播 喜 近 | 2 秒解码通过 | 额外内容服务 |
| 41 | 网易云 | [AkinoKaede/mygo-music-netease-cloud-music](https://github.com/AkinoKaede/mygo-music-netease-cloud-music) | 0.1.13 | 搜 播 词 专 艺 单 喜 近 客 登 | 网易云接口参数错误 400，五类搜索失败 | 明确平台/内容域名 |
| 42 | 网易云 | [Arteiimis/mygo-music-netease](https://github.com/Arteiimis/mygo-music-netease) | 0.2.1 | 搜 播 词 专 艺 单 喜 近 客 登 | 音频探测通过；短片段解码时取流遭 -460 风控 | 明确平台/内容域名 |
| 43 | 网易云 | [AsperaUlt/mygo-netease](https://github.com/AsperaUlt/mygo-netease) | 1.0.14 | 搜 播 词 专 艺 单 喜 近 客 登 | 2 秒解码通过 | 明确平台/内容域名 |
| 44 | 网易云 | [fansion314/mygo-music-yoi](https://github.com/fansion314/mygo-music-yoi) | 0.1.1 | 搜 播 词 专 艺 单 喜 近 客 登 | 插件 ID 格式不支持 | 明确平台/内容域名 |
| 45 | 网易云 | [Faye-bit/MyGo-Music-Netease-All](https://github.com/Faye-bit/MyGo-Music-Netease-All) | 0.3.1 | 搜 播 词 专 艺 单 喜 客 登 | 下载包超过 16 MiB | 明确平台/内容域名 |
| 46 | 网易云 | [hubandad/mygo-music-netease](https://github.com/hubandad/mygo-music-netease) | 1.0.0 | 搜 播 词 专 艺 单 喜 近 客 登 | 2 秒解码通过 | 明确平台/内容域名 |
| 47 | 网易云 | [KirishimaRin/mygo-music-netease-cloud-music](https://github.com/KirishimaRin/mygo-music-netease-cloud-music) | 0.1.7 | 搜 播 词 专 艺 单 喜 近 客 登 | 连接网易云失败，尚未区分接口与兼容层 | 明确平台/内容域名 |
| 48 | 网易云 | [SixWa12138/mygo-music-netease-music](https://github.com/SixWa12138/mygo-music-netease-music) | 0.4.0 | 搜 播 词 专 艺 单 喜 近 客 登 | TextEncoder 缺失，模块执行失败 | 明确平台/内容域名 |
| 49 | 网易云 | [sogud/mygo-music-netease](https://github.com/sogud/mygo-music-netease) | 0.1.12 | 搜 播 词 专 艺 单 喜 客 登 | 音频探测通过；短片段解码时取流遭 -460 风控 | 明确平台/内容域名 |
| 50 | 网易云 | [VenusLuna/mygo-music-netease-plus](https://github.com/VenusLuna/mygo-music-netease-plus) | 0.2.6 | 搜 播 词 专 艺 单 喜 客 登 | 2 秒解码通过 | 明确平台/内容域名 |
| 51 | 聚合 | [hhym/mygo-music-hulumusic-plugin](https://github.com/hhym/mygo-music-hulumusic-plugin) | 2.0.1 | 搜 播 词 专 艺 单 喜 近 客 | 外部程序不兼容 | 外部程序、聚合/第三方解析 |
| 52 | 聚合 | [hiSlowMo/neteaseXqqmusic](https://github.com/hiSlowMo/neteaseXqqmusic) | 0.1.5 | 搜 播 词 专 艺 单 喜 近 客 登 | 下载包超过 16 MiB | 外部程序 |
| 53 | 聚合 | [jfeng018/mygo-music-lx-source](https://github.com/jfeng018/mygo-music-lx-source) | 0.2.2 | 搜 播 词 单 登 | 动态网络权限不支持 | 动态服务地址、聚合/第三方解析 |
| 54 | 聚合 | [yunforge/mygo-music-download](https://github.com/yunforge/mygo-music-download) | 0.2.0 | 搜 播 词 专 单 | 本轮查询空结果；不据此判定没有内容 | 聚合/第三方解析 |
| 55 | 聚合 | [ZONGZI-000/zongzi-mygo-plugin-beta](https://github.com/ZONGZI-000/zongzi-mygo-plugin-beta) | 1.1.0-beta.20 | 搜 播 词 专 艺 单 喜 近 客 | 高于 MyGo 0.1.22 基线 | 外部程序、聚合/第三方解析 |
| 56 | 聚合 | [ZONGZI-000/zongzi-mygo-plugin-stable](https://github.com/ZONGZI-000/zongzi-mygo-plugin-stable) | 1.1.0 | 搜 播 词 专 艺 单 喜 近 客 | 高于 MyGo 0.1.22 基线 | 外部程序、聚合/第三方解析 |
| 57 | 自建音乐服务 | [benhaotang/mygo-music-Ampache](https://github.com/benhaotang/mygo-music-Ampache) | 0.2.0 | 搜 播 词 专 艺 单 客 登 | 动态网络权限不支持 | 动态服务地址 |
| 58 | 自建音乐服务 | [biaobiaobiao108/mygo-yun-music](https://github.com/biaobiaobiao108/mygo-yun-music) | 1.1.1 | 搜 播 词 专 艺 单 喜 近 登 | 动态网络权限不支持 | 动态服务地址 |
| 59 | 自建音乐服务 | [iko233/webdav4mygo](https://github.com/iko233/webdav4mygo) | 0.3.5 | 搜 播 词 艺 单 | 外部程序不兼容 | 外部程序、动态服务地址 |
| 60 | 自建音乐服务 | [liuxiaofengone/synology-DSAudio-mygo-music-plugin](https://github.com/liuxiaofengone/synology-DSAudio-mygo-music-plugin) | 0.1.1 | 搜 播 词 专 艺 单 喜 登 | 动态网络权限不支持 | 动态服务地址 |
| 61 | 自建音乐服务 | [LYJW131/mygo-music-emby-jellyfin](https://github.com/LYJW131/mygo-music-emby-jellyfin) | 0.1.0 | 搜 播 词 专 艺 单 喜 近 登 | 动态网络权限不支持 | 动态服务地址 |
| 62 | 自建音乐服务 | [MaYunFei/mygo-music-feiniu](https://github.com/MaYunFei/mygo-music-feiniu) | 0.1.0 | 搜 播 词 专 艺 单 喜 近 登 | 动态网络权限不支持 | 动态服务地址 |
| 63 | 自建音乐服务 | [moewah/mygo-music-emby](https://github.com/moewah/mygo-music-emby) | 0.3.1 | 搜 播 词 专 艺 单 喜 近 登 | 高于 MyGo 0.1.22 基线 | 动态服务地址 |
| 64 | 自建音乐服务 | [pingfury108/mygo-music-sonust](https://github.com/pingfury108/mygo-music-sonust) | 0.1.1 | 搜 播 词 专 艺 单 喜 近 | 动态网络权限不支持 | 动态服务地址 |
| 65 | 自建音乐服务 | [rocona/mygo-navidrome](https://github.com/rocona/mygo-navidrome) | 0.1.0 | 搜 播 专 艺 单 喜 登 | 动态网络权限不支持 | 动态服务地址 |
| 66 | 自建音乐服务 | [TengYangTuoHai/mygo-music-navidrome](https://github.com/TengYangTuoHai/mygo-music-navidrome) | 0.2.0 | 搜 播 词 专 艺 单 喜 近 登 | 动态网络权限不支持 | 动态服务地址 |
| 67 | 自建音乐服务 | [TexasOct/mygo-subsonic](https://github.com/TexasOct/mygo-subsonic) | 0.1.10 | 搜 播 词 专 艺 单 喜 登 | 动态网络权限不支持 | 动态服务地址 |
| 68 | 自建音乐服务 | [yusgsysh/mygo-music-fnos-plugin](https://github.com/yusgsysh/mygo-music-fnos-plugin) | 1.1.2 | 搜 播 词 专 艺 单 喜 近 登 | 动态网络权限不支持 | 动态服务地址 |
| 69 | 蜻蜓 FM | [hhym/mygo-music-qingtingfm-main](https://github.com/hhym/mygo-music-qingtingfm-main) | 1.7.0 | 搜 播 喜 近 客 登 | 外部程序不兼容 | 外部程序 |
| 70 | 酷狗 | [bitbyte404/mygo-music-kugou-lite](https://github.com/bitbyte404/mygo-music-kugou-lite) | 1.3.2 | 搜 播 词 专 艺 单 喜 近 登 | 歌曲搜索通过；样本播放受登录/权限限制；歌词缺 TextDecoder | 明确平台/内容域名 |
| 71 | 酷狗 | [craigary/mygo-music-kugou-lite](https://github.com/craigary/mygo-music-kugou-lite) | 0.8.1 | 搜 播 词 专 艺 单 喜 近 登 | 高于 MyGo 0.1.22 基线 | 明确平台/内容域名 |
| 72 | 酷狗 | [dddaniel1/mygo-plugin-kugou](https://github.com/dddaniel1/mygo-plugin-kugou) | 0.1.0 | 搜 播 词 专 单 喜 登 | 歌曲搜索通过；样本播放受登录/权限限制 | 明确平台/内容域名 |
| 73 | 酷狗 | [fcmfcm1999/mygo-music-kugou](https://github.com/fcmfcm1999/mygo-music-kugou) | 1.0.0 | 搜 播 词 专 艺 单 登 | 搜索参数/签名错误 | 明确平台/内容域名 |
| 74 | 陈升 | [nimrc/mygo-chensheng](https://github.com/nimrc/mygo-chensheng) | 1.0.0 | 搜 播 词 专 艺 喜 近 | 2 秒解码通过 | 额外内容服务 |
| 75 | 雀乐 | [popeyelau/mygo-queyue](https://github.com/popeyelau/mygo-queyue) | 0.1.0 | 搜 播 词 单 喜 登 | 匿名结果为空或要求登录；完整能力待账号验证 | 额外内容服务 |

## 复核与复测

隔离驱动：[plugin_audit.rs](../../crates/plugins/examples/plugin_audit.rs)。构建：`CARGO_PROFILE_DEV_DEBUG=0 cargo build --locked -p musicx-plugins --example plugin_audit`。执行前必须把 `MUSICX_DATA_DIR` 指向一次性的独立目录。支持 `install-local`、`install-repo`、`describe`、`search`、`invoke`、`action`、`resolve`；`resolve` 接收完整 Track JSON，可附 `probe` 或 `decode` 及请求音质，decode 只消费前 2 秒。运行错误使用非零退出码。批量编排脚本与完整原始数据见上述本机临时目录。

所有仓库的固定 revision、ZIP URL、入口/manifest/归档 SHA256、GitHub 发布时间/更新时间、声明权限、宿主接口交集和各调用摘要均保存在同名 JSON 文件中；没有保存账号会话、完整歌词或临时音频 URL。GitHub 许可证识别只作为原始元数据保留，不据此推断完整授权或安全性。
