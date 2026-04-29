# Release pipeline

How a tag turns into a Homebrew-installable build of hush.

## Repos involved

Just this one. We use a "custom-URL Homebrew tap": users run
`brew tap djmunro/hush https://github.com/djmunro/hush.git` once, which
points Brew at this repo. Brew finds the cask at `Casks/hush.rb` like
it would in a normal `homebrew-hush` repo. Everything in one place — no
separate tap repo to maintain, no PAT to manage.

## Versioning chain

```
Cargo.toml version   ←  canonical source (manual edit)
        │
        ▼
git tag vX.Y.Z       ←  must match Cargo.toml exactly (CI enforces)
        │
        ▼
build.rs             ←  reads `git rev-parse --short HEAD` → HUSH_GIT_HASH env
        │
        ▼
src/ui.rs            ←  env!("CARGO_PKG_VERSION") + env!("HUSH_GIT_HASH")
        │
        ▼
Tray menu shows      ←  "Hush 0.2.0 (abc1234)"
```

The git hash is informational — it lets you tell two builds of "0.2.0"
apart (a CI-shipped release vs. a local dev build). For shipped builds,
the hash is the commit the tag points at.

## What happens on `git push --tags`

`.github/workflows/release.yml` triggers on `push: tags: ['v*']`:

1. **Checkout** with `fetch-depth: 0` so `build.rs` can resolve the git hash.
2. **Verify tag matches Cargo.toml.** Bash extracts `version = "X.Y.Z"`
   from `Cargo.toml`, compares to `${GITHUB_REF_NAME#v}`. Mismatch → fail.
3. **Install Rust toolchain** (stable, via `dtolnay/rust-toolchain`).
4. **Cache** Cargo + target dir (`Swatinem/rust-cache`) — first release
   on a runner is slow (~10 min for whisper.cpp build), subsequent ~3 min.
5. **Clippy gate**: `cargo clippy --release --all-targets -- -D warnings`.
   Matches the CLAUDE.md zero-warnings rule. Fails the release on any
   warning, so we don't ship sloppy builds.
6. **Build + package**: `bash scripts/package.sh`. Produces
   `dist/Hush-X.Y.Z.dmg` and `dist/Hush-X.Y.Z.zip`.
7. **Upload to GitHub Release**: `gh release create $TAG --generate-notes`
   with both artifacts. If the release already exists (e.g., release-please
   created an empty one), falls back to `gh release upload ... --clobber`.
8. **Bump Homebrew cask**: checks out `main`, computes SHA256 of the DMG,
   `sed`s `version` and `sha256` in `Casks/hush.rb`, commits, pushes. Uses
   the ambient `GITHUB_TOKEN` (no PAT needed since we're committing to the
   same repo the workflow runs in).

Total wall time: ~5–10 min on `macos-14` runner (Apple Silicon). Free for
public repos.

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

`Casks/hush.rb` declares two cleanup paths:

| Action | What it removes |
|---|---|
| `brew uninstall --cask hush` | `/Applications/Hush.app`, the autostart LaunchAgent (via `launchctl bootout` + `delete:`), kills any running process (`quit:`). |
| `brew uninstall --cask --zap hush` | Above, plus the model cache (`~/.cache/hush`), preferences plist, saved app state. |

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

# 3. Tag and push
git commit -am "release v0.x.y"
git tag v0.x.y
git push && git push --tags

# 4. Manually create the GitHub Release
gh release create v0.x.y \
  --generate-notes \
  dist/Hush-0.x.y.dmg \
  dist/Hush-0.x.y.zip

# 5. Manually bump the cask in this same repo
SHA=$(shasum -a 256 dist/Hush-0.x.y.dmg | awk '{print $1}')
sed -i '' "s|version \".*\"|version \"0.x.y\"|" Casks/hush.rb
sed -i '' "s|sha256 \".*\"|sha256 \"${SHA}\"|" Casks/hush.rb
git commit -am "Bump cask to 0.x.y" && git push
```

CI just automates these steps. Reading them once helps you debug when
the workflow fails.

## One-time setup checklist

- [ ] Both brothers have collaborator access (admin) on `djmunro/hush`.
- [ ] Repo Settings → Actions → Workflow permissions = "Read and write
      permissions" so the cask-bump step can `git push` (default for
      personal-account public repos is read-only).

That's it. The recurring release flow is just `git tag vX.Y.Z && git push --tags`.
