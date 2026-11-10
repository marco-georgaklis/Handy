#!/bin/bash
# Build Handy Flow, publish it as a GitHub release on the fork, and install it
# on this Mac. Other Macs pick the release up with scripts/flow-install.sh.
#
#   scripts/flow-release.sh            # build, publish, install here
#   scripts/flow-release.sh --test     # also run the Rust unit tests first
#
# Run it on the Mac that holds the signing certificate, always the same one:
# macOS ties the Accessibility grant to the signature, so a build signed with
# a different identity has to be granted again on every Mac. The identity is
# pinned in .flow-deploy.env (not committed):
#
#   HANDYFLOW_SIGN_ID=<SHA-1 from `security find-identity -v -p codesigning`>
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="${HANDYFLOW_REPO:-marco-georgaklis/Handy}"
APP="src-tauri/target/release/bundle/macos/Handy Flow.app"

[ -f .flow-deploy.env ] && . ./.flow-deploy.env
if [ -z "${HANDYFLOW_SIGN_ID:-}" ]; then
  echo "Set HANDYFLOW_SIGN_ID in .flow-deploy.env to one of:" >&2
  security find-identity -v -p codesigning >&2
  exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
  echo "Commit or stash your changes first: a release is built from a commit." >&2
  exit 1
fi
git push --quiet fork HEAD

if [ "${1:-}" = "--test" ]; then
  (cd src-tauri && CMAKE_POLICY_VERSION_MINIMUM=3.5 cargo test --lib)
fi

APPLE_SIGNING_IDENTITY="$HANDYFLOW_SIGN_ID" CMAKE_POLICY_VERSION_MINIMUM=3.5 \
  bun run tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
codesign --verify --deep --strict "$APP"

TAG="flow-$(date +%Y%m%d-%H%M)-$(git rev-parse --short HEAD)"
OUT="$(mktemp -d "$PWD/release.XXXXXX")"
trap 'rm -rf "$OUT"' EXIT
ditto -c -k --keepParent "$APP" "$OUT/HandyFlow.zip"

gh release create "$TAG" "$OUT/HandyFlow.zip" --repo "$REPO" --target "$(git rev-parse HEAD)" \
  --title "Handy Flow $TAG" --notes "$(git log -1 --format=%s)"
echo "Published $TAG"

scripts/flow-install.sh
