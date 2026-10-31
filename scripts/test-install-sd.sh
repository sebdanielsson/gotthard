#!/bin/sh
# Test scripts/install-sd-dualboot.sh against a replica of the BPI-R4's SD
# card (same GPT as OpenWrt 25.12 writes it) on a loop device, in a privileged
# Alpine container. U-Boot's fw_setenv/fw_printenv are mocked.
#
#   scripts/test-install-sd.sh      (needs out/bpir4 from scripts/build-image.sh)
set -eu

cd "$(dirname "$0")/.."
. image/builder.env
[ -f out/bpir4/gotthard-bpir4.itb ] || { echo "run scripts/build-image.sh first" >&2; exit 2; }

exec docker run --rm --privileged --platform linux/arm64 \
	--volume "$PWD:/src:ro" \
	"$BUILDER_IMAGE" sh -eu -c '
apk add --no-cache --quiet sgdisk util-linux e2fsprogs coreutils >/dev/null

# --- replica of the router card: GPT ends at 513 MiB, card is 32 GB ---------
img=/tmp/sd.img
truncate -s $((1048609 * 512)) $img
sgdisk -o -a 1 \
	-n 1:34:+8158 -c 1:bl2 \
	-n 2:8192:+1024 -c 2:ubootenv \
	-n 3:9216:+4096 -c 3:factory \
	-n 4:13312:+8192 -c 4:fip \
	-n 5:24576:+65536 -c 5:recovery \
	-n 6:90112:+40960 -c 6:install \
	-n 7:131072:+917504 -c 7:production \
	$img >/dev/null
# fill OpenWrt partitions with data so we can prove they are untouched
dd if=/dev/urandom of=$img bs=512 seek=131072 count=4096 conv=notrunc 2>/dev/null
truncate -s $((62521344 * 512)) $img
before=$(dd if=$img bs=512 skip=34 count=1048542 2>/dev/null | sha256sum)

loop=$(losetup -P -f --show $img)
echo "replica card on $loop"
for p in /sys/class/block/$(basename $loop)p*; do
	n=/dev/$(basename $p); [ -b $n ] || mknod $n b $(cut -d: -f1 $p/dev) $(cut -d: -f2 $p/dev)
done

# --- mock U-Boot env tools ---------------------------------------------------
cat >/tmp/fw <<"EOF"
#!/bin/sh
env=/tmp/uboot.env
touch $env
case "$(basename "$0")" in
fw_setenv)
	name=$1; shift
	grep -v "^$name=" $env >$env.new || true; mv $env.new $env
	if [ $# -gt 0 ]; then echo "$name=$*" >>$env; fi
	;;
fw_printenv)
	if [ $# -eq 0 ]; then cat $env; else for n; do grep "^$n=" $env; done; fi
	;;
esac
EOF
chmod +x /tmp/fw; ln -sf /tmp/fw /usr/local/bin/fw_setenv; ln -sf /tmp/fw /usr/local/bin/fw_printenv
echo "bootcmd=if pstore check ; then run boot_recovery ; else run boot_sdmmc ; fi" >/tmp/uboot.env

export DISK=$loop TEST_MODE=1 BACKUP_DIR=/tmp/backup
inst="sh /src/scripts/install-sd-dualboot.sh"
itb=/src/out/bpir4/gotthard-bpir4.itb
media=/src/out/bpir4/media.tar.gz

echo; echo "### dry run"
$inst $itb $media | tail -5
sgdisk -p $loop | grep -q " gotthard_a" && { echo "FAIL: dry run changed the disk"; exit 1; }

echo; echo "### install"
$inst --apply $itb $media

fail=0
check() { if eval "$2"; then echo "PASS $1"; else echo "FAIL $1"; fail=1; fi; }
mnt=/tmp/m; mkdir -p $mnt
check "partitions named"      "sgdisk -p $loop | grep -q gotthard_a && sgdisk -p $loop | grep -q gtvar"
check "backup GPT at card end" "sgdisk -v $loop | grep -q \"No problems found\""
check "FIT written"           "cmp -n $(wc -c <$itb) $itb ${loop}p8"
check "GTMEDIA label"         "blkid ${loop}p10 | grep -q LABEL=\\\"GTMEDIA\\\""
check "GTVAR label"           "blkid ${loop}p11 | grep -q LABEL=\\\"GTVAR\\\""
mount ${loop}p10 $mnt
check "boot repository"       "test -f $mnt/apks/.boot_repository && test -f $mnt/apks/aarch64/APKINDEX.tar.gz"
check "modloop"               "test -f $mnt/boot/modloop-bpir4"
check "apkovl"                "ls $mnt/*.apkovl.tar.gz >/dev/null"
umount $mnt
check "menu entry"            "grep -q \"^bootmenu_9=Boot Gotthard\" /tmp/uboot.env"
check "boot command"          "grep -q \"^boot_gotthard=.*bootm \\\$loadaddr#\\\$bootconf#\\\$bootconf_sd\" /tmp/uboot.env"
check "bootcmd untouched"     "grep -q \"^bootcmd=if pstore check\" /tmp/uboot.env"
after=$(dd if=$img bs=512 skip=34 count=1048542 2>/dev/null | sha256sum)
check "OpenWrt partitions untouched" "[ \"$before\" = \"$after\" ]"
check "backups written"       "ls /tmp/backup/uboot-env-*.txt /tmp/backup/gpt-*.bin >/dev/null"

echo; echo "### reinstall keeps /var"
mount ${loop}p11 $mnt; echo keep >$mnt/marker; umount $mnt
$inst --apply $itb $media >/dev/null
mount ${loop}p11 $mnt
check "var kept on reinstall" "grep -qx keep $mnt/marker"
umount $mnt

echo; echo "### uninstall"
$inst --apply --uninstall >/dev/null
check "partitions removed"    "! sgdisk -p $loop | grep -q gotthard_a"
check "env removed"           "! grep -q -e boot_gotthard -e bootmenu_9 /tmp/uboot.env"
after=$(dd if=$img bs=512 skip=34 count=1048542 2>/dev/null | sha256sum)
check "OpenWrt still untouched" "[ \"$before\" = \"$after\" ]"

losetup -d $loop
exit $fail
'
