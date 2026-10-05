# Releasing mlo

This is the checklist for cutting a release. A release is a `v*` tag; pushing
it runs `.github/workflows/release.yml`, which builds the archives for every
supported target, publishes a GitHub Release with a `SHA256SUMS` file, and
deploys the documentation site to GitHub Pages.

> **This repository has no git remote yet.** Everything below assumes a remote
> named `origin`; until the owner adds one, none of it can run. See
> [Before the first release](#before-the-first-release).

## Version locations

The version is written down in more than one place, and a release must agree
with itself. There is no automated version check in this repository, so verify
by hand (or grep) before tagging:

| file | what to change |
|---|---|
| `Cargo.toml` | `[package] version` — the version the binary reports |
| `Cargo.lock` | updated automatically by the next `cargo build`/`cargo metadata`; commit it |
| `CHANGELOG.md` | a new `## <version>` section at the top |
| `docs/release-notes/release-notes-<version>.md` | **new file**; the workflow uses it as the GitHub Release body when present |
| `docs/INSTALL.md` | the "current version" sentence and the archive-name table |
| `docs/index.md` | only if it names a version (it does not today) |

The workflow reads `Cargo.toml` itself and refuses to build when the pushed tag
does not match it, so a forgotten `Cargo.toml` bump fails the run instead of
shipping a mistagged binary. The other files are prose and are not checked.

## Cut the release

```sh
# 1. Bump the version everywhere in the table above, then:
cargo build --release            # regenerates Cargo.lock; catches a broken build
cargo test                       # unit + acceptance tests, headless, no network, no device

# 2. Commit and push the bump to the default branch.
git add -A
git commit -m "Release 0.1.0"
git push origin main

# 3. Tag — the tag name is Cargo.toml's version prefixed with `v`.
git tag -a v0.1.0 -m "mlo 0.1.0"
git push origin v0.1.0
```

Pushing the tag is what starts the release:

- **build** — five targets: `x86_64-pc-windows-msvc`, `x86_64-apple-darwin`,
  `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`.
  Linux legs also build the `-min` variant (`--no-default-features`). Each
  archive is `mlo-<version>-<target>.tar.gz` (`.zip` on Windows) and contains
  the binary, `README.md`, `LICENSE`, `CHANGELOG.md`, `install.sh` and
  `install.ps1`.
- **release** — refuses to publish unless every target produced its archive,
  writes `SHA256SUMS`, and creates the GitHub Release. The body is
  `docs/release-notes/release-notes-<version>.md` when that file exists, and
  GitHub's generated notes otherwise.
- **pages** — publishes `docs/` to GitHub Pages.

### Watch the run

```sh
gh run list   --workflow=release.yml --limit 5
gh run watch  $(gh run list --workflow=release.yml --limit 1 --json databaseId -q '.[0].databaseId')
```

Or open the repository's **Actions** tab. Re-running a release without
re-tagging is safe: use **Run workflow** (`workflow_dispatch`) on
`release.yml`. The version still comes from `Cargo.toml`, and the release job
recreates/updates the release for `v<version>`.

## Enable GitHub Pages (once)

The `pages` job uploads `docs/` with `actions/configure-pages`,
`actions/upload-pages-artifact` and `actions/deploy-pages`. Those actions
require the repository to have Pages configured for a workflow:

1. **Settings → Pages**
2. **Build and deployment → Source: GitHub Actions**

Until that is set, the `pages` job fails with a `Get Pages site failed` /
"Pages is not enabled" error while the rest of the release still succeeds. The
job also needs the `pages: write` and `id-token: write` permissions, which the
workflow already grants.

## Before the first release

This repository has **no remote configured** (`git remote -v` is empty), so
there is nothing to push a tag to yet. The owner must:

1. Create the repository (e.g. `https://github.com/dillydalli3r/mlo`).
2. Add it as the remote:

   ```sh
   git remote add origin https://github.com/dillydalli3r/mlo.git
   git branch -M main
   git push -u origin main
   ```

3. Update the placeholder coordinates so the installers and docs point at the
   real repository:
   - `Cargo.toml` → `repository = "https://github.com/dillydalli3r/mlo"`
   - `install.sh` / `install.ps1` → their default `MLO_REPO` (`dillydalli3r/mlo`)
   - `docs/index.md` and `docs/INSTALL.md` → the `dillydalli3r/mlo` URLs
4. Enable Pages as above, then push the first `v*` tag.

## Troubleshooting

- **`tag vX does not match Cargo.toml version Y`** — bump `Cargo.toml`, commit,
  then move the tag: `git tag -d vX && git tag -a vX -m "mlo X" && git push -f origin vX`.
- **A matrix leg failed** — fix, then re-run the failed jobs from the Actions
  tab, or run `release.yml` via `workflow_dispatch`. Nothing is published until
  all legs pass, so a partial run leaves no half-release behind.
- **The Linux arm64 leg** uses GitHub's arm64 hosted runner (`ubuntu-24.04-arm`),
  available to public repositories. If this repository is ever private, that
  leg must switch to a cross build (e.g. `cross-rs/cross` with a pre-build
  installing `libasound2-dev:arm64`).