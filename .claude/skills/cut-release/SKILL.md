---
name: cut-release
description: Explain or troubleshoot hush's automatic release pipeline. Releases happen automatically on every merge to main — there is normally nothing to do by hand.
---

# hush releases are automatic

## The model

Every push to `main` (i.e. every merged PR) triggers
`.github/workflows/release.yml`, which with no human involvement:

1. Auto-bumps the patch version in `Cargo.toml` (0.4.2 → 0.4.3)
2. Gates on `cargo clippy --release --all-targets -- -D warnings`
3. Builds + packages `Hush-X.Y.Z.{dmg,zip}` via `scripts/package.sh`
4. Seds the new version + DMG sha256 into `Casks/hush-dictation.rb`
5. Commits `release vX.Y.Z` (Cargo.toml + Cargo.lock + cask), tags it,
   pushes to main
6. Publishes a GitHub Release with auto-generated notes + artifacts

Docs-only pushes (`**.md`, `docs/**`) don't release.

**So if the user asks to "cut a release": merge the work to main.
That IS the release.** Don't tag, don't bump, don't push tags.

## Minor/major bumps

The patch auto-bump only fires when Cargo.toml's version is already
tagged. To ship 0.5.0 instead of 0.4.3: set `version = "0.5.0"` in
`Cargo.toml` inside the PR. The workflow sees the untagged version and
uses it verbatim.

## Verifying a release went out

```bash
gh run list --repo djmunro/hush --workflow release --limit 1   # success?
gh release view --repo djmunro/hush                            # latest release
git pull                                                       # pull CI's release commit
brew update && brew info --cask hush-dictation                 # cask updated?
```

## When something breaks

### Workflow failed on clippy

Fix the lint in a follow-up PR. Merging the fix releases both changes.

### Workflow failed pushing the release commit

Most likely cause: repo Settings → Actions → General → Workflow
permissions is not "Read and write permissions".

### Release published but brew installs the old version

`brew update` first — brew caches tap state.

### Local checkout fights CI's release commits

CI commits to main after every merge. Always `git pull --rebase` before
pushing local work to main.

## What you must NOT do

- Don't tag by hand. The workflow creates tags; a hand-pushed tag does
  nothing (no workflow listens on tags anymore) and will desync the
  auto-bump arithmetic if it's ahead of main.
- Don't hand-edit version/sha256 in `Casks/hush-dictation.rb` unless CI
  is broken — CI overwrites both on every release.
- Don't rename the cask back to `hush` — homebrew-cask ships an
  unrelated "Hush" Safari extension that shadows the bare name.
- Don't enable hardened runtime in `scripts/build-app.sh` — we sign
  ad-hoc on purpose. See `docs/macos-permissions.md`.

## Manual fallback (CI completely broken)

See `docs/release.md` § "Manual fallback" — recipe to ship by hand.
