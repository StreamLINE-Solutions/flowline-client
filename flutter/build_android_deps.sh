#!/bin/bash

set -e -o pipefail

ANDROID_ABI=$1

# Build RustDesk dependencies for Android using vcpkg.json
# Required:
#   1. set VCPKG_ROOT / ANDROID_NDK path environment variables
#   2. vcpkg initialized
#   3. ndk, version: r25c or newer

if [ -z "$ANDROID_NDK_HOME" ]; then
  echo "Failed! Please set ANDROID_NDK_HOME"
  exit 1
fi

if [ -z "$VCPKG_ROOT" ]; then
  echo "Failed! Please set VCPKG_ROOT"
  exit 1
fi

API_LEVEL="21"

# Get directory of this script

SCRIPTDIR="$(readlink -f "$0")"
SCRIPTDIR="$(dirname "$SCRIPTDIR")"

# Check if vcpkg.json is one level up - in root directory of RD

if [ ! -f "$SCRIPTDIR/../vcpkg.json" ]; then
  echo "Failed! Please check where vcpkg.json is!"
  exit 1
fi

# NDK llvm toolchain

HOST_TAG="linux-x86_64" # current platform, set as `ls $ANDROID_NDK/toolchains/llvm/prebuilt/`
TOOLCHAIN=$ANDROID_NDK/toolchains/llvm/prebuilt/$HOST_TAG

# vcpkg manifest mode keeps only ONE triplet per install root: running a second
# `vcpkg install --triplet` against the same --x-install-root WIPES the previous
# triplet (e.g. installing arm-neon-android removes arm64-android). Rust links
# against `installed/arm64-android` (aarch64) and `installed/arm-android` (armv7),
# so both must exist at the end of the step.
#
# Fix: install each triplet into a dedicated temporary root, then move it into
# place under $VCPKG_ROOT/installed (target dir purged first for idempotence on
# self-hosted runners where the vcpkg tool cache persists between runs).

function build {
  ANDROID_ABI=$1

  case "$ANDROID_ABI" in
  arm64-v8a)
     ABI=aarch64-linux-android$API_LEVEL
     VCPKG_TARGET=arm64-android
     MOVE_TARGET=arm64-android
     ;;
  armeabi-v7a)
     ABI=armv7a-linux-androideabi$API_LEVEL
     VCPKG_TARGET=arm-neon-android
     # Rust links the armv7 libs from `arm-android`, not `arm-neon-android`.
     MOVE_TARGET=arm-android
     ;;
  x86_64)
     ABI=x86_64-linux-android$API_LEVEL
     VCPKG_TARGET=x64-android
     MOVE_TARGET=x64-android
     ;;
  x86)
     ABI=i686-linux-android$API_LEVEL
     VCPKG_TARGET=x86-android
     MOVE_TARGET=x86-android
     ;;
  *)
     echo "ERROR: ANDROID_ABI must be one of: arm64-v8a, armeabi-v7a, x86_64, x86" >&2
     return 1
  esac

  INSTALL_TMP="$VCPKG_ROOT/installed-$VCPKG_TARGET"

  echo "*** [$ANDROID_ABI][Start] Build and install vcpkg dependencies"
  pushd "$SCRIPTDIR/.."
  $VCPKG_ROOT/vcpkg install --triplet $VCPKG_TARGET --x-install-root="$INSTALL_TMP"
  popd
  head -n 100 "${VCPKG_ROOT}/buildtrees/ffmpeg/build-$VCPKG_TARGET-rel-out.log" || true
  echo "*** [$ANDROID_ABI][Finished] Build and install vcpkg dependencies"

  if [ ! -d "$INSTALL_TMP/$VCPKG_TARGET" ]; then
    echo "Failed! vcpkg install root '$INSTALL_TMP/$VCPKG_TARGET' not found"
    return 1
  fi

  echo "*** [Start] Move $VCPKG_TARGET to $MOVE_TARGET"
  # $MOVE_TARGET is never a vcpkg triplet in the standard root: it only exists
  # as the result of a previous move. Purge before moving for idempotence.
  if [ -d "$VCPKG_ROOT/installed/$MOVE_TARGET" ]; then
    rm -rf "$VCPKG_ROOT/installed/$MOVE_TARGET"
  fi
  mv "$INSTALL_TMP/$VCPKG_TARGET" "$VCPKG_ROOT/installed/$MOVE_TARGET"
  # Drop the temp root: its vcpkg status would otherwise go stale once the
  # triplet is moved away, causing "file not found" churn on the next run.
  rm -rf "$INSTALL_TMP"
  echo "*** [Finished] Move $VCPKG_TARGET to $MOVE_TARGET"
}

if [ ! -z "$ANDROID_ABI" ]; then
  build "$ANDROID_ABI"
else
  echo "Usage: build-android-deps.sh <ANDROID-ABI>" >&2
  exit 1
fi