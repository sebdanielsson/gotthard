#!/bin/sh
# Install Gotthard next to OpenWrt on the BPI-R4's SD card (dual boot).
# Runs ON the router, under OpenWrt (BusyBox ash). See docs/flashing-poc.md.
#
#   install-sd-dualboot.sh [--apply] [--wipe-var] <gotthard-bpir4.itb> <media.tar.gz>
#   install-sd-dualboot.sh [--apply] --uninstall
#
# Without --apply it only prints what it would do. It never touches OpenWrt's
# partitions (p1-p7) or OpenWrt's boot command: it adds four partitions in the
# unused space after them and one extra U-Boot boot menu entry, so OpenWrt stays
# the default and Gotthard is picked from the boot menu on the serial console.
#
#   p8  gotthard_a   64 MiB  FIT image (kernel, DTBs, Alpine initramfs)
#   p9  gotthard_b   64 MiB  reserved for A/B updates
#   p10 gtmedia       1 GiB  ext4 GTMEDIA: boot repository, modloop, apkovl
#   p11 gtvar         4 GiB  ext4 GTVAR: persistent /var (kept on reinstall
#                            unless --wipe-var)
set -eu

DISK=${DISK:-/dev/mmcblk0}
FW_SETENV=${FW_SETENV:-fw_setenv}
FW_PRINTENV=${FW_PRINTENV:-fw_printenv}
TEST_MODE=${TEST_MODE:-0} # 1 = skip the "is this an OpenWrt BPI-R4 booted from SD" checks
BACKUP_DIR=${BACKUP_DIR:-/root/gotthard-backup}

# Partition layout, in 512-byte sectors. OpenWrt's last partition ends at
# sector 1048575; its old backup GPT sits right after it, so start at 513 MiB.
P8_START=1050624
P8_SIZE=131072    # 64 MiB
P9_SIZE=131072    # 64 MiB
P10_SIZE=2097152  # 1 GiB
P11_SIZE=8388608  # 4 GiB

apply=0 uninstall=0 wipe_var=0
while [ $# -gt 0 ]; do
	case "$1" in
	--apply) apply=1 ;;
	--uninstall) uninstall=1 ;;
	--wipe-var) wipe_var=1 ;;
	-h | --help) sed -n '2,20p' "$0"; exit 0 ;;
	-*) echo "unknown option $1" >&2; exit 2 ;;
	*) break ;;
	esac
	shift
done
ITB=${1:-}
MEDIA_TGZ=${2:-}

log() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
run() { # print, and execute only with --apply
	log "  + $*"
	[ "$apply" = 1 ] && "$@"
	return 0
}

part() { echo "${DISK}p$1"; }

# Make sure /dev/<disk>pN exists (hotplug may lag behind, containers have none).
ensure_node() {
	local n=$1 dev sys
	dev=$(part "$n")
	[ -b "$dev" ] && return 0
	sys=/sys/class/block/$(basename "$dev")/dev
	[ -r "$sys" ] || die "kernel doesn't know $dev yet"
	mknod "$dev" b "$(cut -d: -f1 "$sys")" "$(cut -d: -f2 "$sys")"
}

partition_name() { # <n>
	sed -n 's/^PARTNAME=//p' "/sys/class/block/$(basename "$(part "$1")")/uevent" 2>/dev/null
}

# ---------------------------------------------------------------------------
log "== Checks"
if [ "$TEST_MODE" != 1 ]; then
	[ -f /etc/openwrt_release ] || die "run this on the router under OpenWrt"
	tr -d '\0' </proc/device-tree/model | grep -q 'BPI-R4' || die "not a Banana Pi BPI-R4"
	[ "$(cat /sys/block/mmcblk0/device/type)" = SD ] || die "mmcblk0 is not an SD card (boot DIP switch on eMMC?)"
fi
[ -b "$DISK" ] || die "$DISK not found"
for n in 1 2 3 4 5 6 7; do
	[ -b "$(part "$n")" ] || [ -r "/sys/class/block/$(basename "$(part "$n")")/dev" ] || die "OpenWrt partition $n missing on $DISK"
done
[ "$(partition_name 7)" = production ] || die "partition 7 is '$(partition_name 7)', expected 'production'"
disk_sectors=$(cat "/sys/class/block/$(basename "$DISK")/size")
end=$((P8_START + P8_SIZE + P9_SIZE + P10_SIZE + P11_SIZE))
[ "$disk_sectors" -gt "$end" ] || die "disk too small: $disk_sectors sectors, need $end"
log "disk $DISK: $((disk_sectors / 2048)) MiB, OpenWrt partitions 1-7 present"

need_tools() {
	missing=""
	for t in "$@"; do command -v "$t" >/dev/null 2>&1 || missing="$missing $t"; done
	[ -z "$missing" ] && return 0
	if [ "$TEST_MODE" != 1 ] && [ "$apply" = 1 ]; then
		log "installing missing tools:$missing"
		apk update >/dev/null
		apk add gdisk partx-utils e2fsprogs
	else
		log "missing tools:$missing (would run: apk add gdisk partx-utils e2fsprogs)"
	fi
}
need_tools sgdisk partx mkfs.ext4

# ---------------------------------------------------------------------------
log "== Backup"
stamp=$(date +%Y%m%d-%H%M%S)
run mkdir -p "$BACKUP_DIR"
if [ "$apply" = 1 ]; then
	$FW_PRINTENV >"$BACKUP_DIR/uboot-env-$stamp.txt"
	sgdisk --backup="$BACKUP_DIR/gpt-$stamp.bin" "$DISK" >/dev/null
	log "  saved $BACKUP_DIR/uboot-env-$stamp.txt and gpt-$stamp.bin (copy them off the router)"
fi

# ---------------------------------------------------------------------------
if [ "$uninstall" = 1 ]; then
	log "== Removing the U-Boot boot menu entry and variables"
	for v in bootmenu_9 boot_gotthard gt_part gt_bootargs; do run $FW_SETENV "$v"; done
	log "== Removing partitions 8-11"
	run sgdisk -d 8 -d 9 -d 10 -d 11 "$DISK"
	log "  + partx -d --nr 8:11 $DISK"
	if [ "$apply" = 1 ]; then
		partx -d --nr 8:11 "$DISK" 2>/dev/null || true
	fi
	log "done. OpenWrt is untouched."
	exit 0
fi

[ -f "$ITB" ] || die "FIT image not found: '$ITB'"
[ -f "$MEDIA_TGZ" ] || die "media archive not found: '$MEDIA_TGZ'"
itb_sectors=$((($(wc -c <"$ITB") + 511) / 512))
[ "$itb_sectors" -lt "$P8_SIZE" ] || die "FIT image larger than gotthard_a"

# ---------------------------------------------------------------------------
log "== Partitions"
if [ "$(partition_name 8)" = gotthard_a ] && [ "$(partition_name 11)" = gtvar ]; then
	log "partitions 8-11 already exist, reusing them"
else
	[ -z "$(partition_name 8)" ] || die "partition 8 exists and is not gotthard_a; refusing to touch it"
	s8=$P8_START
	s9=$((s8 + P8_SIZE))
	s10=$((s9 + P9_SIZE))
	s11=$((s10 + P10_SIZE))
	# -e moves the backup GPT to the real end of the card (OpenWrt's image
	# put it at 513 MiB), which makes the rest of the card usable.
	run sgdisk -e \
		-n "8:$s8:+$P8_SIZE" -c 8:gotthard_a -t 8:8300 \
		-n "9:$s9:+$P9_SIZE" -c 9:gotthard_b -t 9:8300 \
		-n "10:$s10:+$P10_SIZE" -c 10:gtmedia -t 10:8300 \
		-n "11:$s11:+$P11_SIZE" -c 11:gtvar -t 11:8300 \
		"$DISK"
	# Tell the kernel about the new partitions without re-reading the whole
	# table, which fails while OpenWrt's partitions are in use. (On an idle
	# disk sgdisk's own re-read already added them and partx has nothing to do.)
	log "  + partx -a --nr 8:11 $DISK"
	if [ "$apply" = 1 ]; then
		partx -a --nr 8:11 "$DISK" 2>/dev/null || true
	fi
	wipe_var=1
fi
if [ "$apply" = 1 ]; then
	for n in 8 9 10 11; do ensure_node "$n"; done
	[ "$(partition_name 8)" = gotthard_a ] || die "kernel doesn't see the new partitions; reboot OpenWrt and re-run"
fi

log "== FIT image -> $(part 8)"
run dd if="$ITB" of="$(part 8)" bs=1M conv=fsync

log "== Boot media -> $(part 10) (GTMEDIA)"
run mkfs.ext4 -q -F -L GTMEDIA "$(part 10)"
mnt=/tmp/gtmedia
run mkdir -p "$mnt"
run mount -t ext4 "$(part 10)" "$mnt"
run tar -xzf "$MEDIA_TGZ" -C "$mnt"
run sync
run umount "$mnt"

if [ "$wipe_var" = 1 ]; then
	log "== Persistent /var -> $(part 11) (GTVAR, new)"
	run mkfs.ext4 -q -F -L GTVAR "$(part 11)"
else
	log "== Persistent /var on $(part 11) kept (use --wipe-var to reset)"
fi

# ---------------------------------------------------------------------------
log "== U-Boot boot menu entry (OpenWrt stays the default)"
# shellcheck disable=SC2016 # U-Boot expands these, not the shell
{
	run $FW_SETENV gt_part gotthard_a
	run $FW_SETENV gt_bootargs 'console=ttyS0,115200n1 pci=pcie_bus_perf'
	run $FW_SETENV boot_gotthard 'led $bootled_pwr on ; part start mmc 0 $gt_part part_addr && part size mmc 0 $gt_part part_size && run mmc_read_vol && setenv bootargs $gt_bootargs && bootm $loadaddr#$bootconf#$bootconf_sd ; led $bootled_pwr off'
	run $FW_SETENV bootmenu_9 'Boot Gotthard (Alpine) from SD card.=run boot_gotthard ; run bootmenu_confirm_return'
}

if [ "$apply" = 1 ]; then
	log "== Result"
	$FW_PRINTENV boot_gotthard bootmenu_9 gt_part gt_bootargs
	for n in 8 9 10 11; do log "  $(part "$n"): $(partition_name "$n")"; done
	log "done. Reboot with the serial console attached and pick 'Boot Gotthard' in the U-Boot menu."
else
	log "dry run only; nothing was changed. Re-run with --apply."
fi
