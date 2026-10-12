# Building and releasing NextCue

## Workflows

**CI** runs on pull requests, pushes to `main`/`master`, and manual dispatch. It builds the frontend, runs browser-companion and Playwright UI tests, and runs native Rust tests on macOS, Windows, and Ubuntu 22.04. Integration tests requiring a real AI account or downloaded speech model remain opt-in.

**Package desktop** runs manually or when a `v*` tag is pushed. It checks versions and runs CI before packaging macOS ARM64 and x64 DMGs, a Windows x64 NSIS installer, and Linux x64 AppImage/deb packages. All builds upload `nextcue-*` workflow artifacts with 14-day retention. Linux packages use Ubuntu 22.04 as the build baseline.

Manual runs only create workflow artifacts. Tag runs collect all installers in one draft GitHub Release after every build succeeds. Rerunning a tag replaces assets only while the release remains a draft. Published release assets are not overwritten by this workflow.

## Prepare a version

Keep these versions identical:

- `package.json` (refresh `package-lock.json` with `npm install --package-lock-only`)
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml` (refresh `Cargo.lock` with `cargo check --manifest-path src-tauri/Cargo.toml`)

Validate locally:

```sh
node scripts/check-release-version.mjs
npm run build
npm test
```

Commit and push the version and workflow files. Tag that commit with the exact version:

```sh
# Example for the current version; use a new version for subsequent releases.
git tag v0.1.0
git push origin v0.1.0
```

The workflow rejects tags that do not match `v` plus the package version. In GitHub Actions, open the packaging run to download artifacts. For a tag run, review the draft under Releases, test every installer on its target platform, and edit release notes before publishing.

## Signing and distribution

The initial workflow produces development preview packages. macOS uses ad-hoc signing (`APPLE_SIGNING_IDENTITY: '-'`) and is not notarized. Windows installers are unsigned. No signing secrets are required for preview packaging.

For public distribution, replace ad-hoc signing with Developer ID signing and configure Apple notarization credentials following [Tauri's macOS signing guide](https://v2.tauri.app/distribute/sign/macos/). Configure Windows certificate signing following [Tauri's Windows signing guide](https://v2.tauri.app/distribute/sign/windows/). Keep credentials in GitHub Actions secrets and pass them only to the packaging step. Platform code signing is separate from Tauri updater signatures; NextCue currently has no automatic updater.

Confirm third-party license notices and connected-provider distribution permissions before publishing. In particular, commercial ChatGPT subscription sign-in requires provider approval as described in the user guide. The proprietary NextCue license does not grant third-party permissions.

## Repository setup

Enable GitHub Actions. Release creation uses the built-in `GITHUB_TOKEN` with `contents: write` only in the release job; no personal access token is required. Repository/organization policy must allow that permission. CI and build jobs use read-only repository permissions. Fork pull requests do not run the release job.

Make the frontend and native CI checks required in branch protection if you want GitHub to enforce them before merging. Workflow files do not configure branch protection automatically.

The pipeline follows [Tauri's GitHub packaging guidance](https://v2.tauri.app/distribute/pipelines/github/). GitHub-hosted runs are the cross-platform verification step; a local macOS check cannot validate Windows or Linux installers.
