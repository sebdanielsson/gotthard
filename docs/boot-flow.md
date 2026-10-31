# Boot flow and storage

## From power-on to login

1. **BootROM, BL2, U-Boot:** unchanged from OpenWrt. The boot DIP switch picks SD card, eMMC or SPI-NAND; SD and eMMC share one controller, so only one of them is visible at a time.
2. **U-Boot menu:** Gotthard adds one entry, `Boot Gotthard (Alpine) from SD card`, and leaves OpenWrt as the default. The entry reads the FIT image from the `gotthard_a` partition, sets the kernel command line to `console=ttyS0,115200n1 pci=pcie_bus_perf` and runs `bootm` with OpenWrt's configuration names (`config-mt7988a-bananapi-bpi-r4` plus the `-sd` overlay).
3. **Kernel and initramfs:** the FIT image holds OpenWrt's kernel, the BPI-R4 device tree and overlays, and Alpine's initramfs.
4. **Alpine initramfs:** finds the partition with `apks/.boot_repository` (label GTMEDIA), loads the config overlay `<hostname>.apkovl.tar.gz` from it, mounts a tmpfs root, and installs every package in `/etc/apk/world` from the boot repository and the apk cache on that partition.
5. **OpenRC:**
   - `modloop` mounts `boot/modloop-bpir4` (kernel modules and firmware)
   - `gotthard-board` detects the board and installs default network and firewall-zone config if there is none
   - `gotthard-var` mounts the GTVAR partition on `/var`
   - `gotthard-media` points lbu and the apk cache at the boot media
   - then the firewall, networking, dhcpcd, dnsmasq, sshd, Docker and Tailscale start

## Persistence

| What | Where | How |
| --- | --- | --- |
| OS | RAM | Reinstalled from the boot repository at every boot |
| `/etc`, `/home`, `/root` | `<hostname>.apkovl.tar.gz` on GTMEDIA | `lbu commit` (the apk commit hook runs it after `apk add` and `apk del`) |
| Packages added with `apk add` | `cache/` on GTMEDIA | The apk cache; the world file in the apkovl lists them |
| `/var` (Docker, leases, logs, Tailscale) | GTVAR partition | Mounted at boot; new package directories are seeded without overwriting state |

The boot media stays mounted read-only except during `lbu commit` and apk operations.

## SD card layout (proof of concept, dual boot with OpenWrt)

| # | Name | Size | Content |
| --- | --- | --- | --- |
| 1-7 | OpenWrt | 512 MiB | bl2, ubootenv, factory, fip, recovery, install, production; untouched |
| 8 | `gotthard_a` | 64 MiB | FIT image |
| 9 | `gotthard_b` | 64 MiB | Reserved for A/B updates |
| 10 | `gtmedia` | 1 GiB | ext4, label GTMEDIA |
| 11 | `gtvar` | 4 GiB | ext4, label GTVAR |

OpenWrt's image puts the backup GPT at 513 MiB; the installer moves it to the end of the card (`sgdisk -e`) to use the rest.

## Memory

In QEMU the booted system uses about 450 MiB of RAM for the root filesystem and about 700 MiB in total, with Docker running and no containers. The BPI-R4 has 4 GiB.
