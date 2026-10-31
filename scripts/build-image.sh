#!/bin/sh
# Build the Gotthard BPI-R4 image locally (macOS or Linux with Docker on arm64).
#
#   scripts/build-image.sh            image only
#   scripts/build-image.sh --qemu     plus QEMU test assets
#
# Uses site config from ./site if present (gitignored, see docs/site-config.md)
# and a kernel bundle from ./out/kernel-bundle if present (else the official
# OpenWrt release kernel). Output goes to ./out.
set -eu

cd "$(dirname "$0")/.."
. image/builder.env

qemu=0
[ "${1:-}" = "--qemu" ] && qemu=1

mkdir -p out site
args=""
[ -f out/kernel-bundle/sysupgrade.itb ] && args="--volume $PWD/out/kernel-bundle:/bundle:ro"

# shellcheck disable=SC2086
docker run --rm --platform linux/arm64 \
	--volume "$PWD:/src:ro" \
	--volume "$PWD/out:/out" \
	--volume "$PWD/site:/site:ro" \
	--volume gotthard-build-cache:/cache \
	--env QEMU="$qemu" \
	$args \
	"$BUILDER_IMAGE" sh /src/image/build.sh
