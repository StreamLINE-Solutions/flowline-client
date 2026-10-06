#!/usr/bin/env bash
# https://docs.flutter.dev/deployment/ios
# flutter build ipa --release --obfuscate --split-debug-info=./split-debug-info
# no obfuscate, because no easy to check errors
# Prérequis : ./ios_arm64.sh (liblibrustdesk.a) puis ce script.
set -euo pipefail
PATCH="$(cd "$(dirname "$0")/.." && pwd)/.github/patches/flutter_3.24.4_dropdown_menu_enableFilter.diff"
cd "$(dirname "$(dirname "$(which flutter)")")"
# SDK Flutter persistant : patch idempotent (déjà appliqué par un run précédent ?).
if git apply --reverse --check "$PATCH" 2>/dev/null; then
  echo "Patch flutter déjà appliqué."
else
  git apply "$PATCH"
fi
cd -
flutter build ipa --release