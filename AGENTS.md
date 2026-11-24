# Contributions

- Write all commit messages, including the subject and body, in English.
- Write pull request titles and descriptions in English; titles may become squash commit subjects.
- Use Conventional Commits for PR titles and every new handwritten commit: `type(scope): subject`, at most 100 characters in the subject. Scope is optional, lowercase, and describes the changed component.
- Allowed types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `style`, `revert`. Use an imperative English subject, a blank line before the body, and explain the reason when it helps review.
- `feat` releases a minor version; `fix` and `perf` release a patch. Other types do not release by themselves. Use `!` before the colon or a nonempty `BREAKING CHANGE:` footer for a major release. Never disguise a product change as `chore` to skip publication.
- Business PRs use **Squash** with a Conventional Commit title and an English final commit body. Synchronization PRs between `main`, `release`, and maintenance branches use **Merge commit** to preserve ancestry; never squash or rebase these PRs. Their Conventional Commit title should state the direction, such as `chore(release): promote main to stable`.
- `commit-policy` validates PR titles and new handwritten commits using the trusted base revision's rules. GitHub-generated merge/revert commits and historical commits carried by same-repository release synchronization are allowed. English quality and the final merge method also require human review.
- Use English message keys in i18n code; keep translated text in locale resources.

# Releases

1. `main` publishes automatic GitHub Preview releases (`vX.Y.Z-preview.N`) and TestFlight Internal after successful push CI. `release` publishes automatic stable releases (`vX.Y.Z`) after its own push CI. Merging to these branches authorizes their automatic publication; review changes before merging.
2. semantic-release owns versions, tags, generated English release notes and publication. Derive versions from tags and Conventional Commits; never bump Cargo package versions, manually pre-tag an automated release, or run a second tag generator. Legacy timestamp previews remain readable but are no longer generated.
3. Promote tested work with a Merge commit PR from `main` to `release`. Wait for CI on the resulting merge SHA; a green Preview SHA does not validate the stable merge SHA. After every stable promotion, merge `release` back into `main` with a Merge commit PR so the stable tag is in main's ancestry before the next Preview cycle. Backport fixes to an explicitly requested `N.x` or `N.N.x` maintenance branch, then merge fixes forward through PRs. Maintenance releases use their own signed feed and never replace Latest or the global feed.
4. Stable DMGs require Developer ID Application signing, Apple notarization, a stapled ticket and signed Sparkle metadata. Current `release` becomes Latest; ad hoc signing is only for previews/local development. Missing credentials fail before publication.
5. TestFlight uses App Store application/installer distribution identities and a macOS App Store provisioning profile. Enable App Sandbox, omit Sparkle, and verify `VALID`, Internal group association and `IN_BETA_TESTING` after upload. Do not add testers or submit an external beta/public App Store release without separate authorization.
6. Preserve the `Release macOS` workflow file/identity and its build counter (`1000000 + GITHUB_RUN_NUMBER`). Retry the original failed run/jobs using its plan and byte-identical signed artifact; do not dispatch a new run to rebuild an already tagged version. Stop if provenance, tags or asset digests disagree. See [release operations](docs/DEVELOPMENT.md#build-and-publish-with-github-actions).

## Public repository credentials

- PR workflows have read-only tokens, no signing secrets, and no privileged execution of PR code. Do not use `pull_request_target` to execute PR code or restore PR artifacts/caches in credential-bearing jobs.
- The release dispatcher admits only the official successful push CI, same repository, allowed branch and exact tested tip. Dispatch the release workflow on that branch so Environment deployment restrictions apply to its actual ref.
- Use the short-lived `GITHUB_TOKEN`; grant `contents: write` only to semantic-release planning/publication and `actions: write` only to the dispatcher. Do not introduce a broad PAT. Pass PR metadata as JSON, never interpolate titles/bodies in shell scripts.
- Signing secrets live in branch-restricted `macos-preview`, `macos-stable`, and `macos-testflight` Environments. Scope credentials to the steps that need them, import keys into disposable keychains, and never publish private keys/passwords/tokens or raw JWT output.
- Pin Actions to full commit SHAs and release tooling to the committed npm lockfile. Release workflows, scripts/configuration and dependency locks require review by the CODEOWNERS. Repository branch protection must require `commit-policy`, `release-tests` and `test`; keep rebase merging disabled.

Get approval before moving pushed tags. Create branches or worktrees only when explicitly requested.

# Rust

- Manage the minimum Rust version in root `Cargo.toml` and the compiler toolchain in `rust-toolchain.toml`.
- Run iOS Cargo commands from `apps/ios`.
