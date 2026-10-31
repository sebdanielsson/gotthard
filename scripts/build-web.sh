#!/bin/sh
# Build the Gotthard web UI and the static gotthard-web binary.
#
# Usage: scripts/build-web.sh [TARGET]
#
#   TARGET  aarch64-unknown-linux-musl (default) or x86_64-unknown-linux-musl
#
# Output: out/web/gotthard-web-<arch>  (static musl binary with the UI embedded)
#
# Everything runs in digest-pinned containers, so the result does not depend on
# the host toolchain: the UI in node:*-alpine, the backend in rust:*-alpine
# running natively for the target architecture (linux/amd64 is emulated on an
# arm64 host). Works the same on a Mac (Docker/OrbStack) and in CI.
#
# Environment:
#   GOTTHARD_WEB_CACHE_VOLUME  named Docker volume holding the npm cache, the
#                              cargo registry and the cargo target dirs
#                              (default: gotthard-web-cache). Remove it with
#                              `docker volume rm gotthard-web-cache`.
#   GOTTHARD_VERSION           version string baked into the UI bundle
#                              (default: `git describe`, else "dev")

set -eu

NODE_IMAGE="docker.io/library/node:26.13.0-alpine3.24@sha256:c6ad9e9c1d80f6eb607c26b4b0a132ff7a2344fad1c78ea3baa2a7d4c8de6d94"
RUST_IMAGE="docker.io/library/rust:1.99.1-alpine3.24@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d"

usage() {
	sed -n '2,/^$/s/^# \{0,1\}//p' "$0"
}

case "${1:-}" in
-h | --help)
	usage
	exit 0
	;;
esac

target="${1:-aarch64-unknown-linux-musl}"
case "$target" in
aarch64-unknown-linux-musl) platform=linux/arm64 arch=aarch64 ;;
x86_64-unknown-linux-musl) platform=linux/amd64 arch=x86_64 ;;
*)
	echo "error: unsupported target '$target'" >&2
	usage >&2
	exit 2
	;;
esac

repo="$(cd "$(dirname "$0")/.." && pwd)"
cache_volume="${GOTTHARD_WEB_CACHE_VOLUME:-gotthard-web-cache}"
version="${GOTTHARD_VERSION:-$(git -C "$repo" describe --always --dirty 2>/dev/null || echo dev)}"
host_uid="$(id -u)"
host_gid="$(id -g)"

# Single sources of truth for tool versions.
npm_version="$(sed -n 's/.*"packageManager": *"npm@\([^"]*\)".*/\1/p' "$repo/ui/package.json")"
rust_toolchain="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "$repo/web/rust-toolchain.toml")"
if [ -z "$npm_version" ] || [ -z "$rust_toolchain" ]; then
	echo "error: cannot read npm or Rust version from ui/package.json / web/rust-toolchain.toml" >&2
	exit 1
fi

command -v docker >/dev/null || {
	echo "error: docker is required" >&2
	exit 1
}

mkdir -p "$repo/out/web"

node_version="$(printf '%s\n' "$NODE_IMAGE" | sed -n 's/.*node:\([^-@]*\).*/\1/p')"
echo "==> [1/2] UI: npm ci && npm run build (node $node_version, npm $npm_version)"
# The sources are copied into the container so the host tree only gains
# ui/build; node_modules stays inside the throwaway container.
docker run --rm \
	-v "$repo/ui:/src/ui" \
	-v "$cache_volume:/cache" \
	-e npm_config_cache=/cache/npm \
	-e npm_config_update_notifier=false \
	-e npm_config_fund=false \
	-e npm_config_audit=false \
	-e GOTTHARD_VERSION="$version" \
	-e HOST_UID="$host_uid" -e HOST_GID="$host_gid" \
	-e NPM_VERSION="$npm_version" \
	"$NODE_IMAGE" sh -euc '
		umask 022
		npm install --global --no-audit --no-fund --loglevel=error "npm@$NPM_VERSION"
		mkdir /build && cd /src/ui
		tar -c --exclude=./node_modules --exclude=./build --exclude=./.svelte-kit . | tar -x -C /build
		cd /build
		npm ci --ignore-scripts --loglevel=error
		npm run build --silent
		rm -rf /src/ui/build
		cp -R build /src/ui/build
		chown -R "$HOST_UID:$HOST_GID" /src/ui/build
	'

echo "==> [2/2] gotthard-web: cargo build --release --locked --target $target (Rust $rust_toolchain, $platform)"
docker run --rm \
	--platform "$platform" \
	-v "$repo/web:/src/web:ro" \
	-v "$repo/ui/build:/src/ui/build:ro" \
	-v "$repo/out/web:/out" \
	-v "$cache_volume:/cache" \
	-w /src/web \
	-e CARGO_HOME=/cache/cargo \
	-e CARGO_TARGET_DIR="/cache/target-$arch" \
	-e CARGO_TERM_COLOR=never \
	-e RUSTUP_TOOLCHAIN="$rust_toolchain" \
	-e TARGET="$target" -e ARCH="$arch" \
	-e HOST_UID="$host_uid" -e HOST_GID="$host_gid" \
	"$RUST_IMAGE" sh -euc '
		umask 022
		rustc --version
		cargo build --release --locked --target "$TARGET"
		bin="$CARGO_TARGET_DIR/$TARGET/release/gotthard-web"
		# Refuse to ship anything that needs a dynamic loader.
		if readelf -l "$bin" | grep -q INTERP || readelf -d "$bin" | grep -q NEEDED; then
			echo "error: $bin is not statically linked" >&2
			exit 1
		fi
		install -m 0755 "$bin" "/out/gotthard-web-$ARCH"
		chown "$HOST_UID:$HOST_GID" "/out/gotthard-web-$ARCH"
	'

out="$repo/out/web/gotthard-web-$arch"
size="$(wc -c <"$out" | tr -d " ")"
echo "==> Built ${out#"$repo"/} ($size bytes)"
