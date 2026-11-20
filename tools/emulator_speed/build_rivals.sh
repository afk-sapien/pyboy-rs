#!/bin/sh
# Fetch and build the emulators that tools/emulator_speed/run.py times.
#
# Usage: tools/emulator_speed/build_rivals.sh WORK [PYTHON]
#
# WORK gets src/ (pinned sources), build/ (binaries) and one virtual
# environment per Python package. PYTHON makes those environments (default
# python3). JOBS sets build parallelism (default 4). Each source is checked
# against a pinned commit or sha256 before it is built. Each emulator is built
# with its own release settings. Only the GUI frontends and optional mGBA
# libraries are turned off.
set -eu

[ $# -ge 1 ] || { echo "usage: build_rivals.sh WORK [PYTHON]" >&2; exit 2; }
mkdir -p "$1"
WORK=$(cd "$1" && pwd)
PYTHON=${2:-python3}
JOBS=${JOBS:-4}
HERE=$(cd "$(dirname "$0")" && pwd)
SRC=$WORK/src
OUT=$WORK/build
mkdir -p "$SRC" "$OUT"

# fetch NAME REPOSITORY COMMIT
fetch() {
    if [ ! -d "$SRC/$1/.git" ]; then
        git init --quiet "$SRC/$1"
        git -C "$SRC/$1" fetch --quiet --depth 1 "$2" "$3"
        git -C "$SRC/$1" checkout --quiet FETCH_HEAD
    fi
    actual=$(git -C "$SRC/$1" rev-parse HEAD)
    if [ "$actual" != "$3" ]; then
        echo "$1: expected commit $3, found $actual" >&2
        exit 1
    fi
}

# SameBoy v1.0.5, mGBA 0.10.5, gambatte-core master on 2026-04-16 (no releases).
fetch sameboy https://github.com/LIJI32/SameBoy ea667ad79aa28409f034b1fd6da7304e1844750d
fetch mgba https://github.com/mgba-emu/mgba 26b7884bc25a5933960f3cdcd98bac1ae14d42e2
fetch gambatte-core https://github.com/pokemon-speedrunning/gambatte-core 5a41a68c25402421fb1983ddadc9faf2418ddb0f

# rgbds v1.0.4 assembles SameBoy's own boot ROMs.
RGBDS_URL=https://github.com/gbdev/rgbds/releases/download/v1.0.4/rgbds-linux-x86_64.tar.xz
RGBDS_SHA256=3e19e50ea602e316be709160acaf0a708b1fa6d32d219ec541f244442895dbb7
if [ ! -x "$SRC/rgbds/rgbasm" ]; then
    mkdir -p "$SRC/rgbds"
    curl -fsSL -o "$SRC/rgbds/rgbds.tar.xz" "$RGBDS_URL"
    echo "$RGBDS_SHA256  $SRC/rgbds/rgbds.tar.xz" | sha256sum -c --quiet -
    tar -xJf "$SRC/rgbds/rgbds.tar.xz" -C "$SRC/rgbds"
fi

# SameBoy: CONF=release adds -O3 -ffast-math -flto. The library target is
# built directly because the lib target also generates headers with cppp.
make -C "$SRC/sameboy" -j"$JOBS" CONF=release CC=gcc RGBDS="$SRC/rgbds/" \
    build/bin/BootROMs/dmg_boot.bin build/bin/BootROMs/cgb_boot.bin build/lib/libsameboy.a
gcc -O3 -std=gnu11 -I"$SRC/sameboy" "$HERE/sameboy_speed.c" \
    "$SRC/sameboy/build/lib/libsameboy.a" -lm -o "$OUT/sameboy_speed"
cp "$SRC/sameboy/build/bin/BootROMs/dmg_boot.bin" "$SRC/sameboy/build/bin/BootROMs/cgb_boot.bin" "$OUT/"

# mGBA: Release build of its own benchmark tool, mgba-perf.
cmake -S "$SRC/mgba" -B "$OUT/mgba" -DCMAKE_BUILD_TYPE=Release -DBUILD_PERF=ON \
    -DBUILD_QT=OFF -DBUILD_SDL=OFF -DBUILD_GL=OFF -DBUILD_GLES2=OFF -DBUILD_GLES3=OFF \
    -DUSE_EPOXY=OFF -DUSE_FFMPEG=OFF -DUSE_LUA=OFF -DENABLE_SCRIPTING=OFF \
    -DUSE_SQLITE3=OFF -DUSE_LIBZIP=OFF -DUSE_MINIZIP=OFF -DUSE_ELF=OFF -DUSE_LZMA=OFF \
    -DUSE_DISCORD_RPC=OFF -DUSE_EDITLINE=OFF >"$OUT/mgba-cmake.log"
cmake --build "$OUT/mgba" --target mgba-perf -j "$JOBS" >"$OUT/mgba-build.log"

# Gambatte: the default scons target, libgambatte.a with -O2.
"$PYTHON" -m venv "$WORK/venv-scons"
"$WORK/venv-scons/bin/python" -m pip install --quiet scons==4.11.1
(cd "$SRC/gambatte-core/libgambatte" && "$WORK/venv-scons/bin/scons" --quiet -j"$JOBS")
g++ -O2 -fomit-frame-pointer -fno-exceptions -fno-rtti -std=c++11 -DHAVE_STDINT_H \
    -I"$SRC/gambatte-core/libgambatte/include" "$HERE/gambatte_speed.cpp" \
    "$SRC/gambatte-core/libgambatte/libgambatte.a" -lz -o "$OUT/gambatte_speed"

# PyBoy 2.9.0 from PyPI and the published pyboy-rs 0.1.9 Linux wheel.
if [ ! -x "$WORK/venv-pyboy/bin/python" ]; then
    "$PYTHON" -m venv "$WORK/venv-pyboy"
    "$WORK/venv-pyboy/bin/python" -m pip install --quiet pyboy==2.9.0
fi
if [ ! -x "$WORK/venv-pyboy-rs/bin/python" ]; then
    base=https://github.com/afk-sapien/pyboy-rs/releases/download/v0.1.9
    wheel=pyboy_rs-0.1.9-cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64.whl
    curl -fsSL -o "$WORK/SHA256SUMS-0.1.9.txt" "$base/SHA256SUMS.txt"
    sha=$(grep " $wheel\$" "$WORK/SHA256SUMS-0.1.9.txt" | cut -d' ' -f1)
    "$PYTHON" -m venv "$WORK/venv-pyboy-rs"
    "$WORK/venv-pyboy-rs/bin/python" -m pip install --quiet "pyboy-rs @ $base/$wheel#sha256=$sha"
fi

echo "built in $OUT"
