# Releasing

Releases are built by `.github/workflows/wheels.yml`. Pushing a tag `vX.Y.Z`:

1. Fails at once unless `vX.Y.Z` equals the version in `Cargo.toml` and
   `pyproject.toml` (and `python/pyboy_rs/__init__.py` and `Cargo.lock` agree),
   `CHANGELOG.md` has a `## X.Y.Z` section, and X.Y.Z is at least the newest
   release that `SECTION_SINCE` in `crates/pyboy-core/src/state.rs` names. A
   build below that would save states its own release notes say need a newer
   pyboy-rs.
2. Builds the sdist and five wheels and smoke-tests each wheel on Python 3.11, 3.12
   and 3.13 (import, version, a short run, `Machine.read_bank_bytes`, and a saved
   state that carries the pyboy-rs section after its format 15 body); the sdist
   is built from source and tested too.
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
2. Commit the bump, then record a fresh PGO profile (below) with
   `tools/pgo.sh record` and commit `pgo/linux-x86_64.profdata` and
   `pgo/profile.txt` in the same pull request. The version is part of every
   symbol name, so a bump leaves the old profile matching nothing, and a tag
   build fails until the profile is recorded again.
3. Merge to `main` through a pull request and check that the `Wheels` workflow
   is green (run it with `workflow_dispatch` if needed).
4. Do a dry run (above) on the merge commit and check that every job passes
   and that the Linux x86_64 job shows no stale profile warning. A tag build
   would fail on that warning. Count the merge commit against the last
   release tag with `tools/icount.py`
   ([Benchmarks](benchmarks.md#instruction-counts)) and give the table in the
   release notes.
5. Tag and push: `git tag vX.Y.Z && git push origin vX.Y.Z`. The `release` job
   attaches the files and publishes the release (drafted first, published after
   the upload, so it is never public without its assets).
6. The tag URL (`https://github.com/afk-sapien/pyboy-rs/tree/vX.Y.Z`) is the
   corresponding source that the README points to for LGPL purposes.

## Profile-guided Linux x86_64 wheel

The Linux x86_64 wheel is built with profile-guided optimization (PGO). The
profile is `pgo/linux-x86_64.profdata`, an LLVM indexed profile recorded from an
instrumented wheel playing Pokemon Red and Crystal through the Python API.
`pgo/profile.txt` holds the Rust toolchain, target and maturin version it was
recorded with and a fingerprint of the Rust source. The `Wheels` workflow builds
that one wheel with `-Cprofile-use` and the recorded toolchain. The other four
wheels and the sdist build as before. A profile matches one toolchain and one
target only, and there are no ROMs to record the others with.

CI has no ROMs, so the profile is recorded locally and committed. It holds
function names, control-flow hashes and counters, about 380 KB. Before
`tools/pgo.sh record` writes it, it checks that no 64-byte stretch of data from
any ROM or state it played appears in it. A 32-byte block counts as data when
it has at least 3 distinct byte values and at least 8 bytes that differ from
its most common byte. A block of one or two small values in fill does not
count, since a small counter followed by zero counters matches it by chance.

PGO changes how the compiler lays out, inlines and orders code, not what the
code does. Save states and audio from a PGO wheel are identical to those from a
wheel without it.

### When the profile goes stale

rustc matches a profile record to a function by its symbol name and a hash of
its control flow. The symbol name includes the crate version and the toolchain.

- A change to a function's control flow drops the profile for that function
  only. rustc warns `function control flow change detected (hash mismatch)` and
  builds that function as without PGO.
- A version bump, a new toolchain or a change to the dependencies renames every
  function, so none of the profile matches. The wheel builds and works at the
  speed of a build without PGO. rustc warns `no profile data available` for
  each function.

A stale profile never changes behavior, it only loses speed. The workflow runs
`tools/pgo.sh check`, which compares the recorded source fingerprint and
version with the commit being built.

- On a `vX.Y.Z` tag push a stale profile fails the Linux x86_64 job, so the
  release is not published. Record the profile again, merge it and tag a new
  commit. Never move a tag.
- On a manual run it only puts a warning on the run, so a dry run of work in
  progress still builds every wheel.

So the release order is: bump the version, record the profile, commit both,
merge, dry run, then tag.

### Refreshing the profile

Record on a Linux x86_64 machine that has the ROMs, after the version bump and
any Rust change are committed, then commit the two files:

```sh
rustup toolchain install 1.97.1 --component llvm-tools
pip install maturin==1.15.0
tools/pgo.sh record --rom red.gb red-a.state red-b.state --rom crystal.gbc crystal-a.state crystal-b.state
tools/pgo.sh check
```

Use five or so states per game from ordinary play: the overworld, battles,
menus and text. Leave the benchmark scenes out, so measured gains are not
tuned to them. Gzip states are fine. `record` plays 9000 frames per state
(`--frames N`) and takes a few minutes, most of it the instrumented build. It
refuses to run while the Rust source has uncommitted changes, because the
profile must match what HEAD builds. The maturin version must equal the
`wheels.yml` pin.

Recording is repeatable: two records from the same source and states write
the same profile byte for byte, so `cmp` against the committed file shows
whether a record changed anything. A game's cartridge clock is locked to a
fixed date during play. One pyo3 function that sets up the Python classes runs
in a different order in each process and is left out of the profile.

To move the wheel to a newer Rust, record once with `PGO_TOOLCHAIN=1.X.Y`. The
new toolchain is written to `pgo/profile.txt` and the workflow uses it.

## Regenerating third-party notices

`python tools/generate_third_party.py` rewrites `THIRD_PARTY_NOTICES.md` from
`cargo metadata` for the crates that are linked into the wheel.
