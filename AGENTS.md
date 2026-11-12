# Releases

1. Fetch `origin/main` and use a clean checkout with passing CI.
2. Use `vX.Y.Z` for the requested release. Derive release versions from tags; do not bump Cargo package versions.
3. Check local and remote tags, then create `git tag <tag> <commit>`. When requested, push only that tag with `git push origin refs/tags/<tag>`.
4. Verify the `Release macOS` workflow and published assets. Stable releases must become Latest and include a signed update feed; Apple credentials are optional.

Get approval before moving pushed tags. Create branches or worktrees only when explicitly requested.

# Rust

- Manage the minimum Rust version in root `Cargo.toml` and the compiler toolchain in `rust-toolchain.toml`.
- Run iOS Cargo commands from `apps/ios`.

# Contributions

- Write commit messages, pull request titles, and pull request descriptions in English.
- Use English message keys in i18n code; keep translated text in locale resources.
