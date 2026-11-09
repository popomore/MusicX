# Progress

## Current

- Status: 两项修复、本地验收与 PR 交付完成。
- Blockers: 无。
- Next: 无；合并与安装由用户决定。
- Branch/PR: chore/dock → main；https://github.com/popomore/MusicX/pull/2 。
- Runtime/Follow-up: 已安装应用保持原样，测试使用独立目录。

## Plan

| Work | Done when | Status |
| --- | --- | --- |
| 启动窗口 | 首次打开、再次打开、最小化后恢复均显示并激活 | complete |
| 会话持久化 | 重启恢复歌曲、队列、位置、音量，默认暂停 | complete |
| 验证与交付 | 回归测试、工作区检查、运行证据和 PR 状态明确 | complete |

## Steps

### Step 1 — 2026-11-09 — 确认范围

- Action/decision: 用户报告启动后需再点 Dock，并补充恢复上次播放进度。默认恢复暂停，避免自动出声。
- Result: 现有历史记录保存进度，但 Player::new 从空会话开始；已有延迟获取音频的恢复路径可复用。
- Evidence: crates/core/src/player.rs、crates/core/src/history.rs；已安装窗口的 AX 与截图确认空队列状态。尚未声称完成运行验收。

### Step 2 — 2026-11-09 — 复现与修复

- Action/decision: 原生临时探针运行实际 run_panel：修复前 visible=true、active=false、key=false。候选原因按启动时序、直接创建子进程、前台权限传递排序。
- Result: 只将首次显示与激活移到 applicationDidFinishLaunching 后，探针变为 visible=true、active=true、key=true；保留现有启动子进程方式。重新打开统一调用同一显示函数。临时探针已删除，改为正式原生回归 harness。
- Evidence: `MUSICX_NATIVE_UI_TEST=1 cargo test --locked -p mini-cloudmusic-panel --test window_lifecycle` 在 macOS 27.0.1 通过 launch / minimized reopen / hidden reopen 三项。依据：[Apple activation 文档](https://developer.apple.com/documentation/appkit/nsapplication/activate%28%29)。
- Action/decision: 重启回归测试先红：`checkpoint must exist before shutdown`。增加 playback-session.json 原子私有保存，操作后与退出时保存，播放期间每 5 秒检查保存；只保存曲目元数据、队列、索引、来源上下文、位置、音量。
- Result: 新进程自动恢复暂停；点击播放才重新取流。未知时长不会错误地把恢复位置归零；损坏 JSON 或无效队列索引降级为空播放器。
- Evidence: 跨进程测试覆盖保存 73.25 秒、重启恢复、取流失败保留状态并重试、未知时长、损坏文件及无效索引；确认不保存临时音频 URL 或 User-Agent。

### Step 3 — 2026-11-09 — 完整检查与实际界面

- Action/decision: 运行 cargo fmt、cargo test --locked --workspace、cargo clippy --locked --workspace --all-targets -- -D warnings、git diff --check；按各进程独立依赖图构建 helper / panel / webauth。
- Result: 格式与 Clippy 无问题，37 项工作区测试通过；原生窗口 harness 的 3 项状态检查通过。
- Evidence: 独立数据实例 `~/Library/Application Support/MusicX Resume QA 4g8doppl` 使用单独端口；通过现有会话交接入口提供测试元数据，seek=90.5、volume=0.21，正常退出并重启真实 helper；返回 paused、两首队列和原位置。随后实际 panel 的 AX 显示 Resume checkpoint、播放按钮与 1:30 / 3:00，截图一致。
- Limit: 真实来源续播音频未听音验收；续播状态机使用测试 AudioOutput 验证。没有替换 /Applications/MusicX.app 或修改生产播放数据。
- Cleanup: 临时启动探针已移除，独立测试进程已停止。

### Step 4 — 2026-11-09 — PR 交付

- Action/decision: 按两个独立变更提交 3f2acb8（启动激活）与 6124717（会话恢复），推送现有 chore/dock 分支并创建 [PR #2](https://github.com/popomore/MusicX/pull/2)。目标 main 通过 sc 状态确认。
- Result: GitHub 返回 MERGEABLE / CLEAN、非 draft；main 未启用保护规则或 ruleset，无要求的 CI/审批。PR 无工作流运行、无 inline review threads 或已提交 reviews；super.engineering 无待处理评论。
- Limit: CodeRabbit 的可选自动审查因 OSS 配额耗尽未执行，其提示称约 50 分钟后恢复；这不是仓库要求的合并条件，未等待或触发额外审查。无持续监控任务，无待等待的必需检查。
- Delivery: 未合并、发布或安装。目标记录归档；文档提交推送后再核对当前 PR head 与干净同步状态。
