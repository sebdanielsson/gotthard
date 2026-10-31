#!/bin/sh
# Build the BPI-R4 kernel, DTBs and kernel modules with the OpenWrt build
# system, with Gotthard's kernel options. Produces a "kernel bundle" in $OUT:
#
#   sysupgrade.itb   OpenWrt FIT image (kernel, DTB, overlays, OpenWrt rootfs)
#   kmods/           all kmod-*.apk built, plus an unsigned packages.adb
#                    (integrity comes from SHA256SUMS of the CI artifact)
#   kernel.config    the final kernel .config
#   SHA256SUMS
#
# image/build.sh consumes the bundle the same way it consumes an official
# OpenWrt release. Runs as a non-root user inside the builder container
# (kernel/Dockerfile.builder); see .github/workflows/kernel.yml.
set -eu

SRC=$(cd "$(dirname "$0")/.." && pwd)
. "$SRC/kernel/openwrt.env"

WORK=${WORK:-$HOME/openwrt}
OUT=${OUT:-$HOME/out}
JOBS=${JOBS:-$(nproc)}

log() { printf '\n=== %s\n' "$*"; }

log "Checking out OpenWrt $OPENWRT_VERSION ($OPENWRT_COMMIT)"
if [ ! -d "$WORK/.git" ]; then
	git init -q "$WORK"
	git -C "$WORK" remote add origin "$OPENWRT_REPO"
fi
git -C "$WORK" fetch -q --depth 1 origin "$OPENWRT_COMMIT"
git -C "$WORK" checkout -q --force FETCH_HEAD
git -C "$WORK" clean -q -fdx -e dl -e staging_dir -e build_dir -e .ccache
cd "$WORK"

log "Applying kernel config fragment"
cfg=target/linux/$OPENWRT_TARGET/$OPENWRT_SUBTARGET/config-6.12
# Drop any existing line for a symbol we set, then append ours.
grep -E '^CONFIG_[A-Z0-9_]+=' "$SRC/kernel/config/kernel.fragment" | cut -d= -f1 | while read -r sym; do
	sed -i -e "/^$sym=/d" -e "/^# $sym is not set/d" "$cfg"
done
grep -E '^CONFIG_' "$SRC/kernel/config/kernel.fragment" >>"$cfg"

log "Writing .config"
cp "$SRC/kernel/config/openwrt.diffconfig" .config
grep -vE '^\s*(#|$)' "$SRC/kernel/kmods.list" | while read -r p; do
	echo "CONFIG_PACKAGE_$p=m" >>.config
done
make defconfig >/dev/null

# Fail early if defconfig silently dropped a requested option.
for want in CONFIG_TARGET_mediatek_filogic_DEVICE_bananapi_bpi-r4=y CONFIG_KERNEL_DEVTMPFS=y CONFIG_KERNEL_PSI=y; do
	grep -qx "$want" .config || { echo "defconfig dropped $want" >&2; exit 1; }
done
grep -vE '^\s*(#|$)' "$SRC/kernel/kmods.list" | while read -r p; do
	grep -qx "CONFIG_PACKAGE_$p=m" .config || { echo "defconfig dropped $p" >&2; exit 1; }
done

log "Downloading sources"
make -j"$JOBS" download

log "Building (jobs=$JOBS)"
if ! make -j"$JOBS"; then
	log "Parallel build failed, retrying single-threaded for a readable log"
	make -j1 V=s
fi

log "Collecting kernel bundle"
rm -rf "$OUT" && mkdir -p "$OUT/kmods"
bin=bin/targets/$OPENWRT_TARGET/$OPENWRT_SUBTARGET
cp "$bin"/*-"$OPENWRT_DEVICE"-squashfs-sysupgrade.itb "$OUT/sysupgrade.itb"
cp "$bin"/packages/kmod-*.apk "$OUT/kmods/"
# Index only the kmods and the "kernel" meta package they depend on.
cp "$bin"/packages/kernel-*.apk "$OUT/kmods/"
staging_dir/host/bin/apk mkndx --allow-untrusted --output "$OUT/kmods/packages.adb" "$OUT"/kmods/*.apk
cp "$(find build_dir/target-*/linux-"$OPENWRT_TARGET"_"$OPENWRT_SUBTARGET"/linux-*/ -maxdepth 1 -name .config | head -1)" "$OUT/kernel.config"
(cd "$OUT" && find . -type f ! -name SHA256SUMS -exec sha256sum {} + | sort -k2 >SHA256SUMS)

log "Done"
grep -E '^CONFIG_(DEVTMPFS|RD_ZSTD|SQUASHFS_ZSTD|AUTOFS_FS|PSI|IKCONFIG_PROC)=' "$OUT/kernel.config"
ls -la "$OUT"
