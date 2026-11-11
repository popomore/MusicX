# 发布

- 用户要求“基于 main 创建 X.Y.Z”时，按发布版本处理，创建 `vX.Y.Z` tag；只有明确要求分支或工作树时才创建它们。
- 正式发布 tag 使用 `vX.Y.Z` 格式，例如 `v1.0.0`。用户要求推送时，只推送该 tag。
- 发布前同步 `origin/main`，确认待发布提交已在 `main` 上，且对应提交的 CI 已通过。
- 发布版本以 tag 为唯一来源，不在 Cargo 清单、锁文件或源码中维护当前发布版本；发布时无需修改 Cargo 包版本。Cargo 的 `version` 只用于内部包元数据。依赖变更时仍须同步根目录的 `Cargo.lock` 和独立 iOS workspace 的 `apps/ios/Cargo.lock`。
- 合法格式为正式版 `vX.Y.Z` 或预览版 `vX.Y.Z.preview-YYYYMMDDHHMMSS-SHA4`，其中 X/Y/Z 为无前导零的非负整数。预览 tag 的 UTC 时间与四位小写 SHA 必须对应源提交。
- 根 workspace 成员通过 `rust-version.workspace = true` 继承根 `[workspace.package].rust-version` 声明的最低 Rust 版本。实际编译器版本及所需组件由根 `rust-toolchain.toml` 固定；CI 使用 `rustup toolchain install --no-self-update` 读取它，避免在 workflow 中重复声明版本。
- `scripts/release-tag.sh` 只校验 tag 与提交身份，不读取 Cargo 版本。发布前应在待发布提交的干净检出中运行校验。

## 创建和推送 tag

在待发布的 `main` 提交检出中执行以下步骤；将示例版本替换为用户指定的版本。任一步骤失败都应停止，修复后重新校验。

```sh
set -eu
release_tag=v1.0.0
git fetch origin main --tags
release_sha=$(git rev-parse refs/remotes/origin/main)
test "$(git rev-parse HEAD)" = "$release_sha"
test -z "$(git status --porcelain)"
bash scripts/release-tag.sh "$release_sha" "$release_tag"
git tag "$release_tag" "$release_sha"
git push origin "refs/tags/$release_tag"
git ls-remote --tags origin "refs/tags/$release_tag"
```

- 校验脚本必须输出 `stable`，才可创建正式发布 tag。非法格式会输出 `Invalid release tag` 并以非零状态退出。
- 创建前检查本地和远端是否已有同名 tag；已存在时核对提交，不覆盖或强推。更改已推送 tag 的指向须先说明影响并取得用户明确授权。

## 推送后的验收

- `.github/workflows/release-macos.yml` 监听所有 tag，也支持对现有 tag 手动触发。独立的 `validate` job 先校验 tag 格式、预览提交身份与源提交是否位于 `main`；非法 tag 或对分支手动触发必须报错，后续构建和发布不运行。
- 校验结果确定发布渠道及版本。构建通过 `MUSICX_RELEASE_VERSION` 注入 tag 的 `X.Y.Z`，用于应用的 `CFBundleShortVersionString`、`CFBundleVersion` 和版本界面。Release manifest 与稳定版 appcast 的版本也须与 tag 对应。
- 核对远端 tag 的提交，再检查对应的 `Release macOS` 运行及失败步骤。tag 推送成功、workflow 成功、Release 资产齐全和实际安装可用是不同的验收状态，分别报告。
- 发布资产应包含 `MusicX-macos-arm64.dmg`、对应的 `.sha256` 文件和 `release-manifest.json`；正式稳定版还须包含签名的 `appcast.xml`。核对 manifest 的 `source_sha`、版本、签名模式和文件摘要。
- 正式版 tag 对应正式 Release，必须完成 Developer ID 签名与公证，缺少配置时明确失败；预览 tag 对应 prerelease，可使用 ad hoc 签名。
- `main` 的 CI 成功后，自动预览沿用最近已发布、非草稿的合法正式 Release 的版本，追加 UTC 时间和短 SHA；首次正式发布前跳过自动预览。版本从 GitHub Releases 读取，不从 Cargo 清单读取。
