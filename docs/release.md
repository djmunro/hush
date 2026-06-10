# Release pipeline

How a merged PR turns into a Homebrew-installable build of hush —
automatically, with no human in the loop.

## Repos involved

Just this one. We use a "custom-URL Homebrew tap": users run
`brew tap djmunro/hush https://github.com/djmunro/hush.git` once, which
points Brew at this repo. Brew finds the cask at `Casks/hush-dictation.rb`
like it would in a normal `homebrew-hush` repo. Everything in one place —
no separate tap repo to maintain, no PAT to manage.

The cask is named `hush-dictation`, not `hush`: homebrew-cask ships an
unrelated "Hush" (a Safari nag-blocker extension), and the bare name
`brew install --cask hush` resolves to that one instead of ours.

## Versioning chain

```
Cargo.toml version   ←  canonical source (auto-bumped by CI on merge)
        │
        ├─▶ git tag vX.Y.Z          (created by CI, same commit as the bump)
        ├─▶ Info.plist              (build-app.sh reads Cargo.toml)
        ├─▶ Casks/hush-dictation.rb (CI seds version + sha256)
        │
        ▼
build.rs             ←  reads `git rev-parse --short HEAD` → HUSH_GIT_HASH env
        │
        ▼
src/ui.rs            ←  env!("CARGO_PKG_VERSION") + env!("HUSH_GIT_HASH")
        │
        ▼
Tray menu shows      ←  "Hush 0.4.2 (abc1234)"
```

The git hash is informational — it lets you tell two builds of "0.4.2"
apart (a CI-shipped release vs. a local dev build).

## What happens when a PR merges

`.github/workflows/release.yml` triggers on `push: branches: [main]`
(docs-only and markdown-only pushes are ignored):

1. **Checkout `main` tip** with `fetch-depth: 0` (full history + tags).
2. **Compute version.** If the Cargo.toml version is already tagged,
   auto-bump the patch (0.4.2 → 0.4.3) and sed Cargo.toml. If Cargo.toml
   holds an untagged version (i.e. the PR deliberately bumped minor or
   major), use it as-is. This is the only release lever a human has:
   edit `Cargo.toml` in the PR to control the next version; otherwise
   patch-bumps happen automatically.
3. **Clippy gate**: `cargo clippy --release --all-targets -- -D warnings`.
4. **Build + package**: `bash scripts/package.sh` →
   `dist/Hush-X.Y.Z.{dmg,zip}`. `build-app.sh` stamps the bundle's
   Info.plist from Cargo.toml, so plist, binary, tag, and cask all agree.
5. **Update cask**: sed `version` and the DMG's real `sha256` into
   `Casks/hush-dictation.rb`.
6. **Commit + tag + push**: one commit (`release vX.Y.Z`) containing
   Cargo.toml, Cargo.lock, and the cask bump, tagged `vX.Y.Z`, pushed to
   main. Pushed with the ambient `GITHUB_TOKEN`, which never triggers
   workflows — so no self-trigger loop.
7. **Publish GitHub Release** with auto-generated notes and both artifacts.

If two PRs merge in quick succession, the `concurrency: release` group
queues the second run, and it checks out the branch tip (not the
triggering SHA) so it builds on top of the first run's release commit.

Total wall time: ~5–10 min on `macos-14` runner (Apple Silicon). Free for
public repos.

`build.yml` is the PR gate — same clippy + package steps, no publish.

## What ad-hoc signing means for releases

We sign with `codesign --force --sign -` (ad-hoc), no Developer ID, no
notarization. Tradeoffs:

- **Direct .dmg download**: macOS shows a Gatekeeper warning on first
  launch. Users right-click → Open, or `xattr -d com.apple.quarantine`.
- **Homebrew install**: clean. Brew strips the `com.apple.quarantine`
  extended attribute as part of cask install, so Gatekeeper doesn't fire.
  This is *the* reason we recommend brew as the primary install path.

To upgrade to a properly-notarized release later: enroll in the Apple
Developer Program ($99/yr), add `DEVELOPER_ID_CERT` (base64-encoded p12)
and `DEVELOPER_ID_PASSWORD` + `APPLE_ID` + `APPLE_TEAM_ID` +
`APP_PASSWORD` secrets, replace the ad-hoc `codesign` line in
`scripts/build-app.sh` with a Developer ID sign, and add an
`xcrun notarytool submit ... --wait` step before `gh release create`.
Out of scope until users complain.

## Cask uninstall surface

`Casks/hush-dictation.rb` declares two cleanup paths:

| Action | What it removes |
|---|---|
| `brew uninstall --cask hush-dictation` | `/Applications/Hush.app`, unloads the autostart LaunchAgent (`launchctl:`), kills any running process (`quit:`). |
| `brew uninstall --cask --zap hush-dictation` | Above, plus the LaunchAgent plist, model cache (`~/.cache/hush`), preferences plist, saved app state. |

The LaunchAgent plist lives under `zap trash:`, not `uninstall delete:` —
a `delete:` stanza makes brew shell out to sudo even for user-owned
files, which breaks unattended uninstalls.

**Brew cannot remove TCC permissions** — Apple owns them, keyed to bundle
ID. Documented in the cask's `caveats` block; users run `tccutil reset`
manually if they want a totally clean slate.

## Manual fallback

If CI is broken and you need to ship right now:

```bash
# 1. Bump version
vim Cargo.toml         # version = "0.x.y"

# 2. Build + package locally
bash scripts/package.sh

# 3. Update the cask
SHA=$(shasum -a 256 dist/Hush-0.x.y.dmg | awk '{print $1}')
sed -i '' -E 's|^  version ".*"|  version "0.x.y"|' Casks/hush-dictation.rb
sed -i '' -E "s|^  sha256 .*|  sha256 \"${SHA}\"|" Casks/hush-dictation.rb

# 4. Commit, tag, push
git commit -am "release v0.x.y"
git tag v0.x.y
git push && git push --tags

# 5. Manually create the GitHub Release
gh release create v0.x.y \
  --generate-notes \
  dist/Hush-0.x.y.dmg \
  dist/Hush-0.x.y.zip
```

CI just automates these steps. Reading them once helps you debug when
the workflow fails.

## One-time setup checklist

- [ ] Both brothers have collaborator access (admin) on `djmunro/hush`.
- [ ] Repo Settings → Actions → Workflow permissions = "Read and write
      permissions" so the release commit can `git push` (default for
      personal-account public repos is read-only).

That's it. The recurring release flow is: merge a PR.
