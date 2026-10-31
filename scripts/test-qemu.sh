#!/bin/sh
# Boot the Gotthard boot media in QEMU (aarch64 virt, Alpine linux-virt
# kernel) and run the boot self-test twice on the same disks: the second boot
# checks that `apk add`, `lbu commit` and /var survived a reboot. Needs
# out/qemu from `scripts/build-image.sh --qemu` (or image/build.sh with QEMU=1).
#
#   scripts/test-qemu.sh                self-test, exit 0 if all checks pass
#   scripts/test-qemu.sh --interactive  boot to a login prompt (Ctrl-A X quits)
#
# Uses HVF on macOS, KVM on Linux when available, else TCG (slow).
set -eu

cd "$(dirname "$0")/.."
Q=out/qemu
for f in vmlinuz-virt initramfs-virt media.qcow2 var.qcow2; do
	[ -f "$Q/$f" ] || { echo "missing $Q/$f; run scripts/build-image.sh --qemu" >&2; exit 2; }
done

case "$(uname -s)" in
Darwin) accel="-accel hvf -cpu host" ;;
*) if [ -w /dev/kvm ]; then accel="-accel kvm -cpu host"; else accel="-accel tcg -cpu max"; fi ;;
esac

# Work on copies so a test run never changes the built images.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp "$Q/media.qcow2" "$Q/var.qcow2" "$tmp/"

qemu() { # <extra kernel args>
	# shellcheck disable=SC2086
	qemu-system-aarch64 -machine virt $accel -smp 4 -m 2048 -nographic -no-reboot \
		-kernel "$Q/vmlinuz-virt" -initrd "$Q/initramfs-virt" -append "console=ttyAMA0 $1" \
		-drive if=virtio,format=qcow2,file="$tmp/media.qcow2" \
		-drive if=virtio,format=qcow2,file="$tmp/var.qcow2" \
		-netdev user,id=n0 -device virtio-net-pci,netdev=n0
}

if [ "${1:-}" = "--interactive" ]; then
	qemu ""
	exit
fi

timeout=${TIMEOUT:-900}
rc=0
for phase in 1 2; do
	log=$Q/selftest-$phase.log
	qemu "gotthard.selftest=$phase" >"$log" 2>&1 &
	pid=$!
	i=0
	while kill -0 "$pid" 2>/dev/null; do
		if [ "$i" -ge "$timeout" ]; then
			echo "boot $phase: timeout after ${timeout}s" >&2
			kill "$pid"
			break
		fi
		sleep 1
		i=$((i + 1))
	done
	wait "$pid" 2>/dev/null || true

	tr -d '\r' <"$log" | grep 'GOTTHARD-SELFTEST' || true
	if tr -d '\r' <"$log" | grep -q 'GOTTHARD-SELFTEST: END failures=0'; then
		echo "boot $phase: passed in ${i}s"
	else
		echo "boot $phase: FAILED (console log: $log)" >&2
		rc=1
	fi
done
exit $rc
