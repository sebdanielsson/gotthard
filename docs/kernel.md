# Kernel

The BPI-R4 needs MediaTek's out-of-tree patches (Ethernet and switch, SFP+, hardware NAT offload, Wi-Fi offload, the mt7996 Wi-Fi driver) that mainline Linux doesn't fully have yet. Gotthard therefore uses OpenWrt's kernel, pinned in [`kernel/openwrt.env`](../kernel/openwrt.env) (OpenWrt 25.12.6, Linux 6.12.112).

## Two sources

The image builder takes the kernel from one of two places. Both are a FIT image plus a kmods package feed, and `image/build.sh` handles them the same way.

| Source | When | Notes |
| --- | --- | --- |
| Official OpenWrt release | Default | Downloaded from downloads.openwrt.org, checked against the pinned SHA-256; the kmods feed is verified with OpenWrt's signing key in `kernel/keys/`. |
| Gotthard CI build | `out/kernel-bundle/` exists, or `kernel_run_id` in the image workflow | Built by `.github/workflows/kernel.yml` with OpenWrt's build system and Gotthard's extra options. |

The official kernel works for the proof of concept: Alpine's initramfs copes without devtmpfs and the initramfs is stored uncompressed. The CI kernel adds what Alpine expects, listed in [`kernel/config/kernel.fragment`](../kernel/config/kernel.fragment) and [`kernel/config/openwrt.diffconfig`](../kernel/config/openwrt.diffconfig):

| Option | Why |
| --- | --- |
| `DEVTMPFS` | udev/mdevd and Alpine's initramfs expect it |
| `RD_GZIP`, `RD_ZSTD` | Compressed initramfs |
| `SQUASHFS_ZSTD` | zstd modloop images |
| `AUTOFS_FS`, `PSI` | Automounts, pressure stall information |
| `IKCONFIG_PROC`, `KALLSYMS` | Debugging |

## Modules

The OpenWrt root filesystem in the FIT image provides the device's default modules and firmware. [`kernel/kmods.list`](../kernel/kmods.list) adds modules for Docker (veth, bridge netfilter, nftables), WireGuard, Tailscale (tun), traffic shaping and USB storage. The builder merges both, runs `depmod`, and packs them with the firmware into `boot/modloop-bpir4` on the boot media.

OpenWrt loads most drivers from lists in `/etc/modules.d` rather than by modalias. The builder keeps that list as `gotthard-modules-load.conf` inside the modloop, and the `gotthard-board` service hands it to Alpine's `modules` service at boot.

## Bumping OpenWrt

1. Update every value in `kernel/openwrt.env` (version, commit of the tag, kernel version, sysupgrade file name and SHA-256, kmods directory).
2. Check that every package in `kernel/kmods.list` still exists in the new kmods feed.
3. If OpenWrt's signing key changed, replace `kernel/keys/`.
4. Run the image workflow, then the kernel workflow.
