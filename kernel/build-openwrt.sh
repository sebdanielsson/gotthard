#!/bin/sh
# Build the BPI-R4 kernel, DTBs and kernel modules with the OpenWrt build
# system, with Gotthard's kernel options. Produces a "kernel bundle" in $OUT:
#
#   sysupgrade.itb   OpenWrt FIT image (kernel, DTB, overlays, OpenWrt rootfs)
#   kmods/           all kmod-*.apk built, plus an unsigned packages.adb
#                    (integrity comes from SHA256SUMS of the CI artifact)
#   kernel.config    config embedded in the main kernel (IKCONFIG)
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
	grep -qE "^CONFIG_PACKAGE_$p=(y|m)$" .config || { echo "defconfig dropped $p" >&2; exit 1; }
done

log "Downloading sources"
make -j"$JOBS" download

log "Building (jobs=$JOBS)"
if ! make -j"$JOBS"; then
	log "Parallel build failed, retrying single-threaded for a readable log"
	make -j1 V=s
fi

log "Collecting kernel bundle"
# $OUT may be a bind mount: empty it instead of removing it.
mkdir -p "$OUT"
find "$OUT" -mindepth 1 -delete
mkdir -p "$OUT/kmods"
bin=bin/targets/$OPENWRT_TARGET/$OPENWRT_SUBTARGET
ls -la "$bin"
cp "$bin"/*-"$OPENWRT_DEVICE"-squashfs-sysupgrade.itb "$OUT/sysupgrade.itb"
# Only the kmods and the "kernel" meta package they depend on.
cp "$bin"/packages/kmod-*.apk "$OUT/kmods/"
cp "$bin"/packages/kernel*.apk "$OUT/kmods/"
# The kernel package depends on libc; ship it so the bundle is self-contained.
cp "$bin"/packages/libc-*.apk "$bin"/packages/libgcc*.apk "$OUT/kmods/" 2>/dev/null || true
echo "kmods: $(find "$OUT/kmods" -name '*.apk' | wc -l) packages"
apk=staging_dir/host/bin/apk
[ -x "$apk" ] || apk=$(find staging_dir -path '*/bin/apk' -type f | head -1)
if ! "$apk" mkndx --allow-untrusted --output "$OUT/kmods/packages.adb" "$OUT"/kmods/*.apk; then
	# OpenWrt's own index of all target packages works too (apk only fetches
	# what the kmods need, and all of that is in the bundle).
	echo "apk mkndx failed, using OpenWrt's index"
	cp "$bin/packages/packages.adb" "$OUT/kmods/packages.adb"
fi
# The config embedded in the main kernel (KDIR/vmlinux, the one in
# sysupgrade.itb). The .config left in the kernel tree belongs to the
# recovery-initramfs kernel, which OpenWrt builds last with other
# initramfs options.
kdir=$(find build_dir -maxdepth 2 -type d -name "linux-${OPENWRT_TARGET}_${OPENWRT_SUBTARGET}" | head -1)
off=$(grep -abo IKCFG_ST "$kdir/vmlinux" | head -1 | cut -d: -f1)
if [ -n "$off" ]; then
	tail -c +$((off + 9)) "$kdir/vmlinux" | gzip -dc >"$OUT/kernel.config" 2>/dev/null || true
fi
if [ ! -s "$OUT/kernel.config" ]; then
	# Informational only: image/build.sh reads the config from the kernel itself.
	echo "warning: no config embedded in $kdir/vmlinux, saving the tree's .config instead" >&2
	cp "$(find build_dir/target-*/linux-"$OPENWRT_TARGET"_"$OPENWRT_SUBTARGET"/linux-*/ -maxdepth 1 -name .config | head -1)" "$OUT/kernel.config"
fi
(cd "$OUT" && find . -type f ! -name SHA256SUMS -exec sha256sum {} + | sort -k2 >SHA256SUMS)

log "Done"
grep -E '^CONFIG_(DEVTMPFS|RD_GZIP|RD_ZSTD|SQUASHFS_ZSTD|AUTOFS_FS|PSI|IKCONFIG_PROC)=' "$OUT/kernel.config" || true
ls -la "$OUT"
