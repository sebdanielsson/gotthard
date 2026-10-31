#!/bin/sh
# Build a Gotthard image for the Banana Pi BPI-R4 (and QEMU test assets).
#
# Runs as root inside an aarch64 Alpine container (scripts/build-image.sh on
# a Mac, .github/workflows/image.yml in CI). Output in $OUT:
#
#   bpir4/gotthard-bpir4.itb   FIT image for U-Boot: OpenWrt kernel, DTBs, Alpine initramfs
#   bpir4/media.tar.gz         contents of the boot media partition (label GTMEDIA):
#                              apks/ boot repository, boot/modloop-bpir4, <host>.apkovl.tar.gz
#   bpir4/packages.lock        exact package versions in this build
#   bpir4/credentials.txt      generated passwords (never commit)
#   qemu/                      Alpine linux-virt kernel + the same media, for tests
#
# Inputs (environment):
#   SRC=/src        repository (read-only)
#   OUT=/out        output directory
#   SITE=/site      optional site config: site.env, overlay/, authorized_keys
#   BUNDLE=/bundle  optional kernel bundle from .github/workflows/kernel.yml;
#                   without it the official OpenWrt release kernel is used
#   QEMU=1          also build QEMU test assets
#   CACHE=/cache    optional download cache
set -eu

SRC=${SRC:-/src}
OUT=${OUT:-/out}
SITE=${SITE:-/site}
BUNDLE=${BUNDLE:-/bundle}
CACHE=${CACHE:-/cache}
QEMU=${QEMU:-0}
WORK=/work

ALPINE_BRANCH=v3.24
ALPINE_MIRROR=${ALPINE_MIRROR:-https://dl-cdn.alpinelinux.org/alpine}

# shellcheck source=../kernel/openwrt.env
. "$SRC/kernel/openwrt.env"

log() { printf '\n=== %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(apk --print-arch)" = aarch64 ] || die "must run on aarch64 (package scripts run in a chroot)"

GOTTHARD_VERSION=$(sed -n 's/^pkgver=//p' "$SRC/packages/gotthard-base/APKBUILD")
BUILD_ID=$(date -u +%Y%m%d%H%M%S)

# Site settings (defaults for a generic build)
HOSTNAME=gotthard
ADMIN_USER="admin"
if [ -f "$SITE/site.env" ]; then
	# shellcheck disable=SC1091
	. "$SITE/site.env"
fi

log "Installing build tools"
apk add --no-cache --quiet \
	abuild alpine-conf cpio curl dtc e2fsprogs kmod mkinitfs openssl qemu-img \
	squashfs-tools tar u-boot-tools xz

rm -rf "$WORK" "$OUT/bpir4" "$OUT/qemu"
mkdir -p "$WORK" "$OUT/bpir4" "$CACHE"
MEDIA=$WORK/media
mkdir -p "$MEDIA/apks/aarch64" "$MEDIA/boot"

# ---------------------------------------------------------------------------
log "Package signing key"
export PACKAGER="Gotthard build <build@gotthard.invalid>"
if ls "$SITE"/keys/*.rsa >/dev/null 2>&1; then
	KEY=$(ls "$SITE"/keys/*.rsa | head -1)
	mkdir -p /root/.abuild
	echo "PACKAGER_PRIVKEY=\"$KEY\"" >/root/.abuild/abuild.conf
	cp "$KEY.pub" /etc/apk/keys/
	echo "using site key $(basename "$KEY")"
else
	abuild-keygen -a -n -q
	KEY=$(sed -n 's/^PACKAGER_PRIVKEY="\(.*\)"/\1/p' /root/.abuild/abuild.conf)
	cp "$KEY.pub" /etc/apk/keys/
	echo "generated ephemeral key $(basename "$KEY")"
fi

# ---------------------------------------------------------------------------
log "Building Gotthard packages"
mkdir -p "$WORK/aports/gotthard"
build_pkg() { # <name>
	cp -r "$SRC/packages/$1" "$WORK/aports/gotthard/"
	[ -n "${2:-}" ] && cp "$2" "$WORK/aports/gotthard/$1/$1"
	(
		cd "$WORK/aports/gotthard/$1"
		abuild -F checksum
		REPODEST=$WORK/repo abuild -F -d -q
	)
}
build_pkg gotthard-base
# The web GUI is a prebuilt static binary from scripts/build-web.sh; without
# it the image is built without the GUI.
WEB_BIN=$OUT/web/gotthard-web-aarch64
if [ -f "$WEB_BIN" ]; then
	build_pkg gotthard-web "$WEB_BIN"
	EXTRA_PKGS=gotthard-web
else
	echo "no $WEB_BIN; building without the web GUI (run scripts/build-web.sh)"
	EXTRA_PKGS=
fi
ls "$WORK/repo/gotthard/aarch64/"

# ---------------------------------------------------------------------------
log "Kernel bundle"
K=$WORK/kernel
mkdir -p "$K"
if [ -f "$BUNDLE/sysupgrade.itb" ]; then
	echo "using CI kernel bundle from $BUNDLE"
	(cd "$BUNDLE" && sha256sum -c -s SHA256SUMS)
	cp "$BUNDLE/sysupgrade.itb" "$K/sysupgrade.itb"
	KMODS_REPO=$BUNDLE/kmods/packages.adb
	KMODS_TRUST=--allow-untrusted
	if grep -q '^CONFIG_RD_ZSTD=y' "$BUNDLE/kernel.config"; then
		INITRAMFS_COMP=zstd
	else
		INITRAMFS_COMP=none
	fi
	KERNEL_SOURCE="Gotthard CI build of OpenWrt $OPENWRT_VERSION"
else
	echo "using official OpenWrt $OPENWRT_VERSION kernel"
	itb=$CACHE/$OPENWRT_SYSUPGRADE
	if [ ! -f "$itb" ]; then
		curl -fsSL -o "$itb.tmp" "$OPENWRT_DOWNLOAD/$OPENWRT_SYSUPGRADE"
		mv "$itb.tmp" "$itb"
	fi
	echo "$OPENWRT_SYSUPGRADE_SHA256  $itb" | sha256sum -c -s
	cp "$itb" "$K/sysupgrade.itb"
	KMODS_REPO=$OPENWRT_DOWNLOAD/$OPENWRT_KMODS_DIR/packages.adb
	KMODS_TRUST=
	INITRAMFS_COMP=none # the release kernel only accepts uncompressed initramfs
	KERNEL_SOURCE="OpenWrt $OPENWRT_VERSION release"
fi

# Split the FIT image into its parts, named like the FIT nodes.
dumpimage -l "$K/sysupgrade.itb" >"$K/fit.txt"
sed -n 's/^ Image \([0-9][0-9]*\) (\(.*\))$/\1 \2/p' "$K/fit.txt" | while read -r idx name; do
	dumpimage -T flat_dt -p "$idx" -o "$K/$name.bin" "$K/sysupgrade.itb" >/dev/null
done
KVER=$(sed -n 's/.*Description: *ARM64 OpenWrt Linux-\([0-9.]*\).*/\1/p' "$K/fit.txt" | head -1)
[ -n "$KVER" ] || die "could not read the kernel version from the FIT image"
echo "kernel $KVER ($KERNEL_SOURCE)"
OVERLAYS=$(sed -n 's/^ Image [0-9]* (fdt-\(mt7988a-.*\))$/\1/p' "$K/fit.txt")
echo "DT overlays: $(echo "$OVERLAYS" | tr '\n' ' ')"

# Modules and firmware from the OpenWrt root filesystem...
unsquashfs -q -no-xattrs -d "$K/owrt" "$K/rootfs-1.bin" \
	lib/modules lib/firmware etc/modules.d etc/modules-boot.d >/dev/null
[ -d "$K/owrt/lib/modules/$KVER" ] || die "rootfs has no modules for $KVER"

# ...plus the extra kmod packages from the matching kmods feed.
KR=$WORK/kroot
mkdir -p "$KR/etc/apk/keys"
cp "$SRC"/kernel/keys/*.pem "$KR/etc/apk/keys/"
echo "$KMODS_REPO" >"$KR/etc/apk/repositories"
[ -n "$KMODS_TRUST" ] || echo "$OPENWRT_DOWNLOAD/packages/packages.adb" >>"$KR/etc/apk/repositories"
# shellcheck disable=SC2046
apk --root "$KR" --arch "$OPENWRT_PKGARCH" --initdb $KMODS_TRUST --no-cache --quiet \
	add --no-scripts $(grep -vE '^\s*(#|$)' "$SRC/kernel/kmods.list")

MT=$WORK/modtree
mkdir -p "$MT/lib/modules/$KVER" "$MT/lib/firmware"
cp -a "$K/owrt/lib/modules/$KVER/." "$MT/lib/modules/$KVER/"
cp -a "$KR/lib/modules/$KVER/." "$MT/lib/modules/$KVER/"
cp -a "$K/owrt/lib/firmware/." "$MT/lib/firmware/"
[ -d "$KR/lib/firmware" ] && cp -a "$KR/lib/firmware/." "$MT/lib/firmware/"
# OpenWrt lists built-in modules by bare file name; kmod's depmod wants a path,
# otherwise `modprobe <built-in>` (e.g. overlay, which dockerd loads) fails.
sed -i 's,^\([^/]*\.ko\)$,kernel/\1,' "$MT/lib/modules/$KVER/modules.builtin"
depmod -b "$MT" "$KVER"
echo "modules: $(find "$MT/lib/modules/$KVER" -name '*.ko' | wc -l)"

# OpenWrt loads most drivers from lists, not by modalias. Keep its order.
cat "$K"/owrt/etc/modules-boot.d/* "$K"/owrt/etc/modules.d/* "$KR"/etc/modules.d/* 2>/dev/null |
	sed -e 's/#.*//' -e '/^[[:space:]]*$/d' | awk '!seen[$1]++' \
	>"$MT/lib/modules/$KVER/gotthard-modules-load.conf"

log "modloop"
ML=$WORK/modloop
mkdir -p "$ML/modules"
cp -a "$MT/lib/modules/$KVER" "$ML/modules/"
cp -a "$MT/lib/firmware" "$ML/modules/firmware"
mksquashfs "$ML" "$MEDIA/boot/modloop-bpir4" -comp xz -all-root -noappend -quiet
ls -la "$MEDIA/boot/modloop-bpir4"

# ---------------------------------------------------------------------------
log "Boot repository ($ALPINE_BRANCH)"
REPO_ARGS="--repository $ALPINE_MIRROR/$ALPINE_BRANCH/main --repository $ALPINE_MIRROR/$ALPINE_BRANCH/community --repository $WORK/repo/gotthard"
PKGS="$(grep -vE '^\s*(#|$)' "$SRC/image/world" | tr '\n' ' ') $EXTRA_PKGS"
FR=$WORK/fetchroot
mkdir -p "$FR/etc/apk" "$CACHE/apk"
cp -a /etc/apk/keys "$FR/etc/apk/"
apk --root "$FR" --initdb --quiet add
# shellcheck disable=SC2086
apk --root "$FR" --quiet $REPO_ARGS --cache-dir "$CACHE/apk" \
	fetch --recursive --output "$MEDIA/apks/aarch64" $PKGS
ls "$MEDIA"/apks/aarch64/*.apk >/dev/null 2>&1 || die "apk fetch failed"
(
	cd "$MEDIA/apks/aarch64"
	apk index --quiet --rewrite-arch aarch64 --description "gotthard $GOTTHARD_VERSION $BUILD_ID" \
		-o APKINDEX.tar.gz *.apk
	abuild-sign -q -k "$KEY" APKINDEX.tar.gz
)
touch "$MEDIA/apks/.boot_repository"
(cd "$MEDIA/apks/aarch64" && ls -- *.apk | sed 's/\.apk$//' | sort) >"$OUT/bpir4/packages.lock"
echo "$(wc -l <"$OUT/bpir4/packages.lock") packages, $(du -sm "$MEDIA/apks" | cut -f1) MiB"

# ---------------------------------------------------------------------------
log "System config (apkovl)"
SR=$WORK/sysroot
mkdir -p "$SR/etc/apk"
cp -a /etc/apk/keys "$SR/etc/apk/"
# shellcheck disable=SC2086
apk --root "$SR" --initdb --quiet --repository "$MEDIA/apks" --no-cache add $PKGS

cp -a "$SRC/image/overlay/." "$SR/"
[ -d "$SITE/overlay" ] && cp -a "$SITE/overlay/." "$SR/"
echo "$HOSTNAME" >"$SR/etc/hostname"

# Admin account (wheel + sudo), root and admin passwords
ROOT_PW=$(openssl rand -base64 18)
ADMIN_PW=$(openssl rand -base64 18)
chroot "$SR" adduser -D -s /bin/bash -g "Gotthard administrator" "$ADMIN_USER"
chroot "$SR" addgroup "$ADMIN_USER" wheel
chroot "$SR" addgroup "$ADMIN_USER" docker
{
	printf 'root:%s\n' "$(openssl passwd -6 "$ROOT_PW")"
	printf '%s:%s\n' "$ADMIN_USER" "$(openssl passwd -6 "$ADMIN_PW")"
} | chroot "$SR" chpasswd -e
if [ -f "$SITE/authorized_keys" ]; then
	install -d -m 700 "$SR/home/$ADMIN_USER/.ssh"
	install -m 600 "$SITE/authorized_keys" "$SR/home/$ADMIN_USER/.ssh/authorized_keys"
	chroot "$SR" chown -R "$ADMIN_USER:$ADMIN_USER" "/home/$ADMIN_USER/.ssh"
fi
umask 077
cat >"$OUT/bpir4/credentials.txt" <<EOF
# Gotthard $GOTTHARD_VERSION build $BUILD_ID - generated passwords, keep private.
# The console (serial) accepts both; SSH accepts only $ADMIN_USER with a key.
root: $ROOT_PW
$ADMIN_USER: $ADMIN_PW
EOF
umask 022

# SSH host keys, so they survive reboots and stay stable across rebuilds of the media
chroot "$SR" ssh-keygen -A >/dev/null

# Services
rc_add() { # <runlevel> <service...>
	lvl=$1
	shift
	for s in "$@"; do chroot "$SR" rc-update -q add "$s" "$lvl"; done
}
rc_add sysinit devfs dmesg mdev hwdrivers modloop
rc_add boot modules sysctl hostname bootmisc syslog seedrng hwclock \
	gotthard-board gotthard-var gotthard-media nftables networking
rc_add default chronyd dhcpcd dnsmasq sshd docker tailscale crond local gotthard-bootlog gotthard-selftest $EXTRA_PKGS
rc_add shutdown mount-ro killprocs savecache

# The apkovl holds what lbu would save: files changed or added compared to
# the installed packages, in /etc plus the protected paths (+home, +root).
apk --root "$SR" audit --backup --quiet --recursive | sed 's,^/,,' | sort -u >"$WORK/apkovl.list"
echo "etc/apk/world" >>"$WORK/apkovl.list"
sort -u -o "$WORK/apkovl.list" "$WORK/apkovl.list"
tar -C "$SR" --numeric-owner --no-recursion -czf "$MEDIA/$HOSTNAME.apkovl.tar.gz" -T "$WORK/apkovl.list"
echo "apkovl: $(wc -l <"$WORK/apkovl.list") files"
cp "$WORK/apkovl.list" "$OUT/bpir4/apkovl.list"

# ---------------------------------------------------------------------------
log "Initramfs"
cp -a "$MT/lib/modules/$KVER" /lib/modules/
mkinitfs -n -K -F base -C "$INITRAMFS_COMP" -o "$WORK/initramfs-bpir4" "$KVER"
ls -la "$WORK/initramfs-bpir4"

log "FIT image"
F=$WORK/fit
mkdir -p "$F"
cp "$K/kernel-1.bin" "$K/fdt-1.bin" "$F/"
cp "$WORK/initramfs-bpir4" "$F/initramfs.bin"
{
	cat <<EOF
/dts-v1/;

/ {
	description = "Gotthard $GOTTHARD_VERSION ($BUILD_ID) for Banana Pi BPI-R4";
	#address-cells = <1>;

	images {
		kernel-1 {
			description = "Linux $KVER ($KERNEL_SOURCE)";
			data = /incbin/("kernel-1.bin");
			type = "kernel";
			arch = "arm64";
			os = "linux";
			compression = "gzip";
			load = <0x46000000>;
			entry = <0x46000000>;
			hash-1 { algo = "crc32"; }; hash-2 { algo = "sha1"; };
		};
		fdt-1 {
			description = "Banana Pi BPI-R4 device tree";
			data = /incbin/("fdt-1.bin");
			type = "flat_dt";
			arch = "arm64";
			compression = "none";
			load = <0x45f00000>;
			hash-1 { algo = "crc32"; }; hash-2 { algo = "sha1"; };
		};
		ramdisk-1 {
			description = "Alpine Linux $ALPINE_BRANCH initramfs";
			data = /incbin/("initramfs.bin");
			type = "ramdisk";
			arch = "arm64";
			os = "linux";
			compression = "none";
			hash-1 { algo = "crc32"; }; hash-2 { algo = "sha1"; };
		};
EOF
	for o in $OVERLAYS; do
		cp "$K/fdt-$o.bin" "$F/"
		cat <<EOF
		fdt-$o {
			description = "device tree overlay $o";
			data = /incbin/("fdt-$o.bin");
			type = "flat_dt";
			arch = "arm64";
			compression = "none";
			hash-1 { algo = "crc32"; }; hash-2 { algo = "sha1"; };
		};
EOF
	done
	cat <<EOF
	};

	configurations {
		default = "config-mt7988a-bananapi-bpi-r4";
		config-mt7988a-bananapi-bpi-r4 {
			description = "Gotthard bananapi_bpi-r4";
			kernel = "kernel-1";
			fdt = "fdt-1";
			ramdisk = "ramdisk-1";
		};
EOF
	for o in $OVERLAYS; do
		cat <<EOF
		$o {
			description = "overlay $o";
			fdt = "fdt-$o";
		};
EOF
	done
	echo "	};"
	echo "};"
} >"$F/gotthard.its"
# External data, 4 KiB aligned: the same layout as OpenWrt's images.
(cd "$F" && mkimage -q -E -B 0x1000 -f gotthard.its "$OUT/bpir4/gotthard-bpir4.itb")
dumpimage -l "$OUT/bpir4/gotthard-bpir4.itb" | grep -E '^ (Image|Configuration)|Data Size' | head -40
cp "$OUT/bpir4/gotthard-bpir4.itb" "$MEDIA/boot/"

cat >"$MEDIA/boot/gotthard-release" <<EOF
GOTTHARD_VERSION=$GOTTHARD_VERSION
GOTTHARD_BUILD=$BUILD_ID
ALPINE_BRANCH=$ALPINE_BRANCH
KERNEL=$KVER
KERNEL_SOURCE=$KERNEL_SOURCE
EOF
cp "$MEDIA/boot/gotthard-release" "$OUT/bpir4/"

log "Boot media archive"
tar -C "$MEDIA" --numeric-owner -czf "$OUT/bpir4/media.tar.gz" .
echo "boot media: $(du -sm "$MEDIA" | cut -f1) MiB unpacked"

# ---------------------------------------------------------------------------
if [ "$QEMU" = 1 ]; then
	log "QEMU test assets (Alpine linux-virt kernel, same boot media)"
	Q=$WORK/qemu
	mkdir -p "$Q/virt" "$Q/modloop/modules" "$OUT/qemu"
	# shellcheck disable=SC2086
	apk --root "$FR" --quiet $REPO_ARGS --cache-dir "$CACHE/apk" \
		fetch --output "$Q" linux-virt
	tar -xzf "$Q"/linux-virt-*.apk -C "$Q/virt" 2>/dev/null
	VKVER=$(ls "$Q/virt/lib/modules")
	cp -a "$Q/virt/lib/modules/$VKVER" "$Q/modloop/modules/"
	cp -a "$Q/virt/lib/modules/$VKVER" /lib/modules/
	depmod "$VKVER"
	cp -a "/lib/modules/$VKVER/." "$Q/modloop/modules/$VKVER/"
	cp -a "$MEDIA" "$Q/media"
	mksquashfs "$Q/modloop" "$Q/media/boot/modloop-virt" -comp xz -all-root -noappend -quiet
	mkinitfs -K -F "base virtio squashfs ext4" -o "$OUT/qemu/initramfs-virt" "$VKVER"
	cp "$Q/virt/boot/vmlinuz-virt" "$OUT/qemu/"
	size=$(($(du -sm "$Q/media" | cut -f1) + 256))
	mke2fs -q -t ext4 -L GTMEDIA -d "$Q/media" "$Q/media.img" "${size}M"
	mke2fs -q -t ext4 -L GTVAR "$Q/var.img" 2048M
	qemu-img convert -O qcow2 "$Q/media.img" "$OUT/qemu/media.qcow2"
	qemu-img convert -O qcow2 "$Q/var.img" "$OUT/qemu/var.qcow2"
	ls -la "$OUT/qemu"
fi

(cd "$OUT/bpir4" && sha256sum gotthard-bpir4.itb media.tar.gz packages.lock gotthard-release >SHA256SUMS)
log "Done: $OUT/bpir4"
ls -la "$OUT/bpir4"
