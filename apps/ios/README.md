# iPhone 原生客户端

iPhone 版使用 Rust、NSH 0.4、UIKit、AVPlayer 和 AVAudioSession。设备独立保存队列、历史、账号和插件，使用自己的播放会话。核心和协议来自 `crates/`，部分界面模块复用 `apps/macos/panel/src/`。

## 构建

需要包含 iOS SDK 的 Xcode、Rust iOS targets 和 NSH CLI 0.4。在本目录执行：

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
python3 package.py --target ios-simulator
python3 package.py --target ios
```

构建脚本默认从 PATH 使用 `nsh`，也可设置 `NSH_BIN` 指定路径。生成的应用包位于 `.nsh/out/<target>/迷你云音乐.app`。模拟器包使用临时签名，设备包需使用自己的开发者签名和 provisioning profile。脚本在签名前声明后台音频模式。

iOS 最低部署版本由 `.cargo/config.toml` 设为 15.0。同一个配置引用仓库内的 NSH iOS 补丁。[补丁说明](../../vendor/nsh-platform-ios/PATCH.md)

## 数据和 MCP

应用沙箱的 `Library/Application Support/MusicX` 保存账号、历史和 MCP 令牌。首次升级时会迁入旧目录的数据。Streamable HTTP 在手机本机 `127.0.0.1:45968/mcp` 监听。登录和私有歌单需要在手机上独立授权。
