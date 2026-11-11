# 发布

- 用户要求“基于 main 创建 X.Y.Z”时，按发布版本处理，创建 `vX.Y.Z` tag；只有明确要求分支或工作树时才创建它们。
- 正式发布 tag 使用 `vX.Y.Z` 格式，例如 `v1.0.0`。用户要求推送时，只推送该 tag。
- 发布前同步 `origin/main`，确认待发布提交已在 `main` 上，且对应提交的 CI 已通过。
- `apps/macos/helper/Cargo.toml`（`musicx`）与 `crates/platform-apple/Cargo.toml`（`musicx-platform-apple`）的版本必须相同，并与 tag 去掉 `v` 后的版本一致。更新版本时同步根目录的 `Cargo.lock` 和独立 iOS workspace 的 `apps/ios/Cargo.lock`，版本变更须先合入 `main`。
- `scripts/release-tag.sh` 读取当前工作树的 Cargo 元数据；必须在待发布提交的干净检出中校验，不能用其他提交的版本文件代替。

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

- 校验脚本必须输出 `stable`，才可创建正式发布 tag。出现 `Expected release tag ...` 表示版本不匹配，不能靠重跑 Actions 解决。
- 创建前检查本地和远端是否已有同名 tag；已存在时核对提交，不覆盖或强推。更改已推送 tag 的指向须先说明影响并取得用户明确授权。

## 推送后的验收

- `.github/workflows/release-macos.yml` 在推送 `v*` tag 时运行，也支持对现有 tag 手动触发。
- 核对远端 tag 的提交，再检查对应的 `Release macOS` 运行及失败步骤。tag 推送成功、workflow 成功、Release 资产齐全和实际安装可用是不同的验收状态，分别报告。
- 发布资产应包含 `MusicX-macos-arm64.dmg`、对应的 `.sha256` 文件和 `release-manifest.json`；正式稳定版还须包含签名的 `appcast.xml`。核对 manifest 的 `source_sha`、版本、签名模式和文件摘要。
- 正式稳定版需要 Developer ID 签名和公证；ad hoc 签名产物会作为 Preview 发布。`main` 的 CI 成功后另有工作流自动创建 `vX.Y.Z.preview-<UTC时间>-<短SHA>` 预览 tag，无须手动重复创建。
