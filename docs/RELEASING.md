# Releasing

Releases are built by `.github/workflows/wheels.yml`. Pushing a tag `vX.Y.Z`:

1. Fails at once unless `vX.Y.Z` equals the version in `Cargo.toml` and
   `pyproject.toml` (and `python/pyboy_rs/__init__.py` and `Cargo.lock` agree), and
   `CHANGELOG.md` has a `## X.Y.Z` section.
2. Builds the sdist and five wheels and smoke-tests each wheel on Python 3.11, 3.12
   and 3.13 (import, version, a short run, `Machine.read_bank_bytes`); the sdist is
   built from source and tested too.
3. Hashes the six files into `SHA256SUMS.txt`.
4. Creates the GitHub release as a draft with the `CHANGELOG.md` section as notes,
   attaches the seven files, downloads them back and checks `SHA256SUMS.txt`, and
   only then publishes it.

That is the distribution channel: PokeSim and PokeSim Core pin the release asset
URLs and hashes. Nothing is published to PyPI from a tag.

The release assets are exactly these files (seven in all; the workflow fails on any other set):

- `pyboy_rs-X.Y.Z-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-macosx_10_12_x86_64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-macosx_11_0_arm64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-win_amd64.whl`
- `pyboy_rs-X.Y.Z.tar.gz`
- `SHA256SUMS.txt` (`sha256sum` format for the six files above; check with `sha256sum -c SHA256SUMS.txt`)

The workflow never modifies a published release; it fails instead. If a draft for
the tag already exists, its notes are replaced, assets not in the set above are
removed, and the files are replaced before it is published.

## Dry run (no tag, no release)

Run `Wheels` by `workflow_dispatch` on `main` (leave `publish_pypi` off):

```sh
gh workflow run wheels.yml --ref main
```

It runs every build and smoke job, then uploads the six files, `SHA256SUMS.txt`
and `notes.md` as the `release-files` workflow artifact, and prints the hashes in
the job summary. It never creates, edits or uploads to a release. Wheels are not
bit-for-bit reproducible, so a tag build has its own hashes; take them from the
release's `SHA256SUMS.txt`.

PyPI is optional and manual only: `publish_pypi` set to true on a manual run
(default false). It needs the one-time setup below.

## PyPI one-time setup (optional, owner)

1. Sign in at <https://pypi.org> and open Account settings, Publishing.
2. Add a **pending publisher** (the project does not exist yet):
   - PyPI project name: `pyboy-rs`
   - Owner: `afk-sapien`
   - Repository name: `pyboy-rs`
   - Workflow name: `wheels.yml`
   - Environment name: `pypi`
3. In GitHub, Settings, Environments, create an environment named `pypi`. Add
   yourself as a required reviewer so each publish needs approval.

Nothing is registered or published by the repository itself.

## Cutting a release

1. Bump the version in `pyproject.toml`, `Cargo.toml` (and `Cargo.lock`) and
   `python/pyboy_rs/__init__.py`; add a `CHANGELOG.md` entry. A test fails if
   they disagree.
2. Merge to `main` through a pull request and check that the `Wheels` workflow
   is green (run it with `workflow_dispatch` if needed).
3. Do a dry run (above) on the merge commit and check that every job passes.
4. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`. The `release` job
   attaches the files and publishes the release (drafted first, published after
   the upload, so it is never public without its assets).
5. The tag URL (`https://github.com/afk-sapien/pyboy-rs/tree/vX.Y.Z`) is the
   corresponding source that the README points to for LGPL purposes.

## Regenerating third-party notices

`python tools/generate_third_party.py` rewrites `THIRD_PARTY_NOTICES.md` from
`cargo metadata` for the crates that are linked into the wheel.
