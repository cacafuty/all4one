# Release Process

This document describes how to prepare and publish a new release of All4One.

## Prerequisites

- Push access to the GitHub repository
- Local git repository with all changes committed
- All tests passing locally

## Step 1: Create Release Branch

Create a dedicated release branch (do not work directly on `main`):

```bash
# Create and check out the release branch
git checkout -b release/v0.1.11

# Verify you're on the new branch
git branch -v
# Should show: * release/v0.1.11
```

Naming convention: `release/v{VERSION}`

## Step 2: Update Version Numbers

Update the version in both `Cargo.toml` files to match the next release tag:

```bash
# Edit agent/Cargo.toml
[package]
name = "all4one-agent"
version = "0.1.11"  # Update this

# Edit common/Cargo.toml
[package]
name = "all4one-common"
version = "0.1.10"  # Update this (typically agent_version - 1)
```

Typically: `agent` version increments first; `common` may lag by 1 minor version.

Example progression:
- Release 0.1.10 → agent `0.1.10`, common `0.1.9`
- Release 0.1.11 → agent `0.1.11`, common `0.1.10`

## Step 3: Update Release Notes

Edit `RELEASE_NOTES.md`:

```markdown
## v0.1.11 (Unreleased)

### New Features
- [List all new features, APIs, behavior changes]

### Bug Fixes
- [Any important fixes]

### Documentation
- [Links to updated docs]

### Platform Support
- ✅ Linux x86-64
- ✅ Linux ARM64
- ✅ macOS ARM64
- ✅ Windows x86-64

### Known Limitations
- [Any blockers or TODOs for future releases]
```

Mark as released by changing the header from `(Unreleased)` to the actual date:

```markdown
## v0.1.11 (2026-05-09)
```

## Step 4: Verify Tests Pass

```bash
cd /path/to/all4one
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

All should pass before proceeding.

## Step 5: Commit Changes to Release Branch

```bash
git add agent/Cargo.toml common/Cargo.toml RELEASE_NOTES.md
git commit -m "chore(release): bump to v0.1.11"
git push origin release/v0.1.11
```

## Step 6: Create Pull Request for Review

Go to GitHub repository and create a PR:
- **Base branch**: `main`
- **Compare branch**: `release/v0.1.11`
- **Title**: `chore(release): prepare v0.1.11`
- **Description**:
  ```
  ## Release v0.1.11
  
  ### Changes
  - Bumped agent to 0.1.11, common to 0.1.10
  - Updated RELEASE_NOTES.md with new features
  
  ### Checklist
  - [x] Tests pass locally
  - [x] Version numbers updated
  - [x] Release notes complete
  - [ ] Approved by maintainer
  ```

Wait for review and CI checks to pass.

## Step 7: Merge Release PR to Main

Once approved and all CI checks pass:
1. Click **Merge pull request** on GitHub
2. Choose **Squash and merge** or **Create a merge commit** (per your workflow)
3. Delete the release branch after merging

```bash
# If merging locally instead:
git checkout main
git pull origin main
git branch -d release/v0.1.11
```

## Step 8: Create and Push Git Tag

Tag **only from `main` after the merge**:

```bash
# Ensure you're on main with latest changes
git checkout main
git pull origin main

# Create an annotated tag
git tag -a v0.1.11 -m "Release v0.1.11: Distributed storage with replication, shared-volume listener, and cluster-wide explorer"

# Push the tag to GitHub
git push origin v0.1.11
```

Pushing the tag **automatically triggers** the GitHub Actions release workflow (`.github/workflows/release.yml`).

**Workflow runs:**
1. Builds binaries for all platforms (Linux x86-64, Linux ARM64, macOS ARM64, Windows x86-64)
2. Compresses each binary (tar.gz for Unix, zip for Windows)
3. Uploads artifacts to GitHub Release page

## Step 9: Verify Release Build

Go to GitHub repository → **Releases** → watch for the new tag build to complete.

Once complete, you will see:
- Release title: `v0.1.11`
- Automated release notes (from commit history)
- Downloadable binaries:
  - `all4one-agent-linux-x86_64.tar.gz`
  - `all4one-agent-linux-arm64.tar.gz`
  - `all4one-agent-macos-arm64.tar.gz`
  - `all4one-agent-windows-x86_64.zip`

## Step 9: Verify Release Build

Go to GitHub repository → **Releases** → watch for the new tag build to complete.

Once complete, you will see:
- Release title: `v0.1.11`
- Automated release notes (from commit history)
- Downloadable binaries:
  - `all4one-agent-linux-x86_64.tar.gz`
  - `all4one-agent-linux-arm64.tar.gz`
  - `all4one-agent-macos-arm64.tar.gz`
  - `all4one-agent-windows-x86_64.zip`

## Step 10: Publish Release (Optional)

If the release is marked as a **Draft** on GitHub:
1. Visit Releases page
2. Find the v0.1.11 release
3. Click "Edit"
4. Uncheck "Set as a draft"
5. Click "Publish release"

If the automated flow published it directly, you're done.

## Step 11: Update Download Links

If you have documentation or setup guides pointing to older releases, update them:

```markdown
# OLD
curl -L "https://github.com/cacafuty/all4one/releases/download/v0.1.10/all4one-agent-linux-x86_64.tar.gz" \
     -o all4one-agent.tar.gz

# NEW
curl -L "https://github.com/cacafuty/all4one/releases/download/v0.1.11/all4one-agent-linux-x86_64.tar.gz" \
     -o all4one-agent.tar.gz
```

Update in:
- `docs/guides/node-setup.md`
- `docs/api/rest-api.md` (if it references download URLs)
- `README.md` (if it has a "quick start" section)

## Important Notes

### Never Push Directly to Main

**Always** use a release branch and PR:
```bash
# ❌ WRONG - Do not do this
git commit ... && git push origin main

# ✅ RIGHT - Always do this
git checkout -b release/v0.1.11
git commit ...
git push origin release/v0.1.11
# [Create PR on GitHub]
# [Get review]
# [Merge to main]
```

This ensures:
- Changes are reviewed before release
- CI runs on PR before merge
- Git history is clean and tagged commits are on main
- Rollback is possible (close the PR)

### CI/CD Workflow

The release automation depends on:
1. **PR merge to main** triggers `.github/workflows/ci.yml` (fmt, clippy, test)
2. **Tag push** (e.g., `v0.1.11`) triggers `.github/workflows/release.yml` (build + upload)

Both workflows must pass for a successful release.

## Workflow Overview (Quick)

```
1. Create release branch (release/v0.1.11)
   ↓
2. Update Cargo.toml versions
   ↓
3. Update RELEASE_NOTES.md
   ↓
4. Run: cargo fmt, clippy, test (all pass)
   ↓
5. Commit & push to release branch
   ↓
6. Create PR on GitHub (base: main, compare: release/v0.1.11)
   ↓
7. Get code review & approval
   ↓
8. CI checks pass on PR
   ↓
9. Merge PR to main (squash or merge commit)
   ↓
10. Delete release branch
    ↓
11. Tag on main: git tag -a v0.1.11 && git push origin v0.1.11
    ↓
12. GitHub Actions builds all platforms (5-10 minutes)
    ↓
13. Verify artifacts on Releases page
    ↓
14. Update docs/guides with new download links
```

## Troubleshooting

### "PR merge conflicts"
If `main` has diverged from `release/v0.1.11`, resolve conflicts:
```bash
git checkout release/v0.1.11
git pull origin main  # or: git merge main
# Resolve conflicts manually
git add .
git commit -m "merge: resolve conflicts"
git push origin release/v0.1.11
```
The PR will auto-update.

### "Tag already exists"
```bash
# If you pushed a tag by mistake, delete it locally and on GitHub
git tag -d v0.1.11
git push origin --delete v0.1.11
# Then re-tag and push (ensure you're on main, at the merge commit)
git tag -a v0.1.11 -m "..."
git push origin v0.1.11
```

### "Release workflow failed on Linux ARM64"
This is expected if the machine doesn't have the aarch64 cross-compiler. The workflow will mark that as a warning (`allow_failure: true`) and continue. You can still use the x86-64 or macOS builds.

### "No binaries appeared on release page"
Check the **Actions** tab:
1. Find the most recent workflow run for the tag
2. Click into it
3. Expand the failed job to see build logs
4. Common issues:
   - Tests failed (check build output)
   - Artifact upload failed (network issue, re-run workflow)

### "Need to revert release"
If something went wrong after release:
```bash
# Create a revert commit
git revert v0.1.11  # Creates a new commit that undoes v0.1.11
git push origin main

# DO NOT delete the tag; instead, prepare v0.1.12 with fixes
# Tag a new patch release once issues are fixed
```

## Next Release

After publishing v0.1.11, start planning v0.1.12:

1. Update `RELEASE_NOTES.md` with a new `## v0.1.12 (Unreleased)` section at the top
2. Bump versions in Cargo.toml to 0.1.12 and 0.1.11 (if following the lag pattern)
3. Create a new branch or commit with these placeholder changes
4. Continue development

Example:

```markdown
# RELEASE_NOTES.md

## v0.1.12 (Unreleased)

### New Features
- [TODO: Add features as they are implemented]

### Bug Fixes
- [TODO]

...

## v0.1.11 (2026-05-09)

### New Features
- Distributed storage with replication
- [etc.]
```
