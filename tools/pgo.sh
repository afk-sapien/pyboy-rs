#!/bin/bash
# Profile for the profile-guided (PGO) Linux x86_64 wheel.
#
#   tools/pgo.sh record [--frames N] --rom GAME STATE... [--rom GAME STATE...]
#   tools/pgo.sh check
#
# record builds an instrumented wheel the way the Wheels workflow builds the
# Linux x86_64 wheel (same maturin, toolchain and target), installs it in a
# scratch virtual environment and plays tools/pgo_play.py on it from the given
# ROMs and save states. It merges the counts into pgo/linux-x86_64.profdata and
# writes pgo/profile.txt with the toolchain and a fingerprint of the source the
# profile was recorded from. Commit both files. ROMs and states stay local: the
# profile holds function names and counters only, and record checks that no
# 64-byte stretch of any ROM or state appears in it.
#
# check compares that fingerprint and the workspace version with HEAD. It exits
# 1 if the Rust source, a manifest, Cargo.lock or the version changed since the
# profile was recorded. A stale profile is still safe to build with: changed
# functions lose their profile and are optimized as without PGO. The Wheels
# workflow fails a tag build on a stale profile and only warns otherwise.
#
# Environment:
#   PGO_TOOLCHAIN     toolchain to record with (default: the one in pgo/profile.txt)
#   PGO_WORK          scratch directory (default: a new temporary directory)
#   PGO_OUT           directory to write the profile and manifest to (default: pgo)
#   PGO_BUILD_PREFIX  command prefix for the build, for example "nice -n 10"
#   PGO_PLAY_PREFIX   command prefix for the play, for example "taskset -c 3"
#   MATURIN, PYTHON   programs to use (default: maturin and python3 on PATH)
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
target=x86_64-unknown-linux-gnu
manifest=pgo/profile.txt
# Everything that changes the compiled wheel. A change to tests, examples, docs
# or the Python package does not make the profile stale.
sources="Cargo.toml Cargo.lock crates/pyboy-core/Cargo.toml crates/pyboy-core/src crates/pyboy-python/Cargo.toml crates/pyboy-python/src"

die() {
  echo "tools/pgo.sh: $*" >&2
  exit 1
}

value() {
  [ -f "$manifest" ] || return 0
  sed -n "s/^$1=//p" "$manifest"
}

version() {
  git show HEAD:Cargo.toml | sed -n '/^\[workspace\.package\]/,/^\[/s/^version = "\(.*\)"$/\1/p'
}

fingerprint() {
  local paths=()
  for path in $sources
  do
    paths+=("HEAD:$path")
  done
  git rev-parse "${paths[@]}" | git hash-object --stdin
}

check() {
  local recorded current recorded_version current_version
  recorded=$(value source)
  current=$(fingerprint)
  recorded_version=$(value version)
  current_version=$(version)
  [ -n "$recorded" ] || die "no source fingerprint in $manifest"
  [ -n "$recorded_version" ] || die "no version in $manifest"
  if [ "$recorded" = "$current" ] && [ "$recorded_version" = "$current_version" ]
  then
    echo "pgo profile matches the source at HEAD (version $current_version, source $current)"
    return 0
  fi
  if [ "$recorded_version" != "$current_version" ]
  then
    echo "pgo profile is stale: recorded for version $recorded_version, HEAD has $current_version"
  fi
  if [ "$recorded" = "$current" ]
  then
    return 1
  fi
  echo "pgo profile is stale: recorded from source $recorded at $(value commit), HEAD has $current"
  if git cat-file -e "$(value commit)^{commit}" 2>/dev/null
  then
    # shellcheck disable=SC2086
    git diff --stat "$(value commit)" HEAD -- $sources
  fi
  return 1
}

record() {
  local frames=9000
  if [ "${1:-}" = --frames ]
  then
    frames=$2
    shift 2
  fi
  [ "${1:-}" = --rom ] || die "usage: tools/pgo.sh record [--frames N] --rom GAME STATE... [--rom GAME STATE...]"
  [ "$(uname -sm)" = "Linux x86_64" ] || die "record runs on Linux x86_64 only, like the profile it makes"
  local toolchain=${PGO_TOOLCHAIN:-$(value toolchain)}
  [ -n "$toolchain" ] || die "set PGO_TOOLCHAIN, for example PGO_TOOLCHAIN=1.97.1"
  local maturin=${MATURIN:-maturin} python=${PYTHON:-python3}
  local pinned have
  pinned=$(sed -n 's/^ *maturin-version: v//p' .github/workflows/wheels.yml | head -1)
  have=$("$maturin" --version | awk '{print $2}')
  [ "$have" = "$pinned" ] || die "maturin is $have, but wheels.yml pins $pinned (pip install maturin==$pinned)"
  # shellcheck disable=SC2086
  [ -z "$(git status --porcelain -- $sources)" ] || die "commit the source first, the profile records what HEAD builds"
  local sysroot profdata
  sysroot=$(RUSTUP_TOOLCHAIN=$toolchain rustc --print sysroot)
  profdata=$sysroot/lib/rustlib/$target/bin/llvm-profdata
  [ -x "$profdata" ] || die "no llvm-profdata, run: rustup component add llvm-tools --toolchain $toolchain"

  local work=${PGO_WORK:-}
  if [ -z "$work" ]
  then
    work=$(mktemp -d)
  fi
  mkdir -p "$work"
  work=$(cd "$work" && pwd)
  rm -rf "$work/raw" "$work/wheel" "$work/venv"
  mkdir -p "$work/raw" "$work/wheel"
  echo "== building an instrumented wheel with Rust $toolchain in $work"
  # shellcheck disable=SC2086
  ${PGO_BUILD_PREFIX:-} env RUSTUP_TOOLCHAIN="$toolchain" RUSTFLAGS="-Cprofile-generate=$work/raw" \
    "$maturin" build --release --locked --target "$target" --target-dir "$work/target" --out "$work/wheel"
  "$python" -m venv "$work/venv"
  "$work/venv/bin/python" -m pip install --quiet --no-index --no-deps "$work"/wheel/*.whl

  echo "== playing $frames frames per state"
  (
    cd "$work"
    # shellcheck disable=SC2086
    LLVM_PROFILE_FILE="$work/raw/play-%p-%m.profraw" ${PGO_PLAY_PREFIX:-} \
      "$work/venv/bin/python" -I "$root/tools/pgo_play.py" --frames "$frames" "$@"
  )

  echo "== merging"
  local out=${PGO_OUT:-pgo}
  mkdir -p "$out"
  "$profdata" merge -o "$work/merged.profdata" "$work"/raw/*.profraw
  local files=() roms=0
  for arg in "$@"
  do
    if [ "$arg" = --rom ]
    then
      roms=$((roms + 1))
    else
      files+=("$arg")
    fi
  done
  "$work/venv/bin/python" -I "$root/tools/pgo_play.py" --scan "$work/merged.profdata" "${files[@]}" ||
    die "ROM or state bytes found in the profile, nothing written"
  mv "$work/merged.profdata" "$out/linux-x86_64.profdata"
  {
    echo "# Written by tools/pgo.sh record. See docs/RELEASING.md."
    echo "toolchain=$toolchain"
    echo "target=$target"
    echo "maturin=$have"
    echo "version=$(version)"
    echo "source=$(fingerprint)"
    echo "commit=$(git rev-parse HEAD)"
    echo "frames=$frames"
    echo "states=$(( ${#files[@]} - roms ))"
  } > "$out/profile.txt"
  "$profdata" show "$out/linux-x86_64.profdata" | tail -n 4
  ls -l "$out/linux-x86_64.profdata" "$out/profile.txt"
  echo "== done, commit $out/linux-x86_64.profdata and $out/profile.txt"
}

if [ "${1:-}" = record ]
then
  shift
  record "$@"
elif [ "${1:-}" = check ]
then
  check
else
  die "usage: tools/pgo.sh record [--frames N] --rom GAME STATE... | tools/pgo.sh check"
fi
