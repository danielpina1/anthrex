# Releasing anthrex

A release is a `vX.Y.Z` tag on `main`. Pushing the tag runs
`.github/workflows/release.yml`, which builds the `anthrex` binary for four targets,
packages each as `anthrex-vX.Y.Z-<target>.tar.gz` with a `.sha256` file, and
publishes a GitHub release with those assets. While the major version is 0, every
release is published as a pre-release.

Targets: `x86_64-unknown-linux-gnu` (ubuntu-latest), `aarch64-unknown-linux-gnu`
(ubuntu-24.04-arm), `aarch64-apple-darwin` (macos-latest) and `x86_64-apple-darwin`
(cross-compiled on macos-latest).

## Steps

1. **Bump the version.** On a branch, set `[workspace.package] version` in the root
   `Cargo.toml` to `X.Y.Z`, then refresh the lockfile so only the workspace crates'
   versions change:

   ```bash
   cargo update -w --offline
   git diff Cargo.lock   # only anthrex* version lines
   ```

   The daemon/client handshake compares `proto::PROTO_VERSION`, not the crate
   version, so a version bump alone needs no protocol change.

2. **Update the CHANGELOG.** Move the entries under `## [Unreleased]` into a new
   `## [X.Y.Z] — YYYY-MM-DD` section and add the link reference at the bottom. The
   release workflow uses that section, verbatim, as the release notes, and fails if
   it is missing.

3. **Merge to `main`.** Open a pull request with the two changes above; merge it once
   CI is green.

4. **Dry-run the release workflow.** In GitHub, open Actions → Release → Run workflow
   and pick `main` (or run `gh workflow run release.yml --ref main`). A manual run
   builds and packages every target and attaches the archives to the run as
   workflow artifacts; it never creates a release. Download one or two artifacts and
   check `anthrex --version` and the checksum.

5. **Tag and push.** From an up-to-date `main`:

   ```bash
   git switch main && git pull --ff-only
   git tag -a vX.Y.Z -m "anthrex X.Y.Z"
   git push origin vX.Y.Z
   ```

   The workflow's first job fails if the tag without its `v` differs from the Cargo
   version, before anything is built or published.

6. **Check the release.** When the run finishes, open
   <https://github.com/danielpina1/anthrex/releases/tag/vX.Y.Z> and confirm:
   - eight assets: one `.tar.gz` and one `.tar.gz.sha256` per target;
   - the notes match the CHANGELOG section, and the release is marked pre-release
     while the major version is 0;
   - a downloaded archive installs and runs as described in `docs/install.md`.

## If something goes wrong

- **Version guard failed.** The tag and `Cargo.toml` disagree. Delete the tag
  (`git push origin :refs/tags/vX.Y.Z` and `git tag -d vX.Y.Z`), fix the version on
  `main` through a pull request, and tag again.
- **A build or the publish job failed.** Nothing is published unless every build
  succeeds. Fix the cause on `main`, delete the tag as above, and tag the fixed
  commit. If the release was created but is wrong, delete it with
  `gh release delete vX.Y.Z` before re-tagging.
