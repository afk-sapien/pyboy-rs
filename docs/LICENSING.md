# Licensing

pyboy-rs is a translation of PyBoy and is licensed LGPL-3.0-only, matching
PyBoy. See [LICENSE.md](../LICENSE.md), [COPYING](../COPYING) and
[NOTICE](../NOTICE), which ship in the sdist and in every wheel. Notices for the
Rust crates linked into the wheel are in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Replacing the library (LGPL relinking)

Each release is built from the source tagged `vX.Y.Z` at
<https://github.com/afk-sapien/pyboy-rs> (for example
<https://github.com/afk-sapien/pyboy-rs/tree/v0.1.3>). The sdist attached to
the GitHub release is the same source. The wheel's `pyboy_rs/_native`
extension is the compiled library. To run an application against a modified
build:

1. Check out the release tag (or unpack the sdist) and edit the source.
2. Build with `maturin build --release --out dist` (Rust 1.85 or newer, Python
   3.11 or newer. `Cargo.lock` pins every dependency).
3. Install over the release with
   `python -m pip install --force-reinstall dist/pyboy_rs-*.whl`.

The application imports `pyboy_rs` dynamically and needs no relinking of its
own. Your modified library replaces the released one.

## Bundled data

The wheel includes PyBoy's redistributable demo ROM, used by installed-runtime
checks. Commercial ROMs, user checkpoints and private gameplay data are never
included.
