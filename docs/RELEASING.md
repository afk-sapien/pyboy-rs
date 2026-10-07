# Releasing

Releases are built by `.github/workflows/wheels.yml` and published to PyPI with
trusted publishing (no API token). Publishing runs only for a pushed tag `v*`,
never for a pull request or a manual run.

## One-time setup (owner)

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
3. Tag and push: `git tag v0.1.1 && git push origin v0.1.1`.
4. Approve the `pypi` environment when the publish job waits. The job uploads the
   sdist and wheels with attestations.
5. The tag URL (`https://github.com/afk-sapien/pyboy-rs/tree/vX.Y.Z`) is the
   corresponding source that the README points to for LGPL purposes.

## Regenerating third-party notices

`python tools/generate_third_party.py` rewrites `THIRD_PARTY_NOTICES.md` from
`cargo metadata` for the crates that are linked into the wheel.
