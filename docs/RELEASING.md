# Releasing

Releases are built by `.github/workflows/wheels.yml`. Pushing a tag `v*` builds the
sdist and the five wheels, runs the smoke tests, and attaches the six files and a `SHA256SUMS` file to the
GitHub release for that tag (created from the `CHANGELOG.md` section if it does not
exist). That is the distribution channel: PokeSim and PokeSim Core pin the release
asset URLs. Nothing is published to PyPI from a tag.

PyPI is optional and manual only: run the workflow with `workflow_dispatch` and
`publish_pypi` set to true (default false). It needs the one-time setup below.

The release assets are exactly these files (seven in all; the workflow fails on any other set):

- `pyboy_rs-X.Y.Z-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-macosx_10_12_x86_64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-macosx_11_0_arm64.whl`
- `pyboy_rs-X.Y.Z-cp311-abi3-win_amd64.whl`
- `pyboy_rs-X.Y.Z.tar.gz`
- `SHA256SUMS` (`sha256sum` format for the six files above; check with `sha256sum -c SHA256SUMS`)

## Dry run (no tag, nothing public)

Run `Wheels` by `workflow_dispatch` with `draft_release` set to true. It builds and
smoke-tests everything, then uploads the six files and `SHA256SUMS` to a DRAFT release `vX.Y.Z`
(untagged; a draft has no public URL and no tag is created). The SHA-256 of each
file is in the job summary. The workflow never modifies a release that is already published, on a dry
run or a tag push: it fails instead. Only a draft has its assets replaced. Delete
the draft afterwards, or let the tag push reuse it (its assets are replaced).

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
3. Optionally do a dry run (above) and check the hashes.
4. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`. The `release` job
   attaches the files and publishes the release (drafted first, published after
   the upload, so it is never public without its assets).
5. The tag URL (`https://github.com/afk-sapien/pyboy-rs/tree/vX.Y.Z`) is the
   corresponding source that the README points to for LGPL purposes.

## Regenerating third-party notices

`python tools/generate_third_party.py` rewrites `THIRD_PARTY_NOTICES.md` from
`cargo metadata` for the crates that are linked into the wheel.
