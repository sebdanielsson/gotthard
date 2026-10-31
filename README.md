# Gotthard

A small router OS for people who know normal Linux: Alpine Linux in diskless mode with OpenRC, bash, sudo, Docker and an nftables firewall, plus a web GUI. The first target is the Banana Pi BPI-R4 (using OpenWrt's kernel for its MediaTek hardware), then x86_64.

Status: proof of concept. It builds, boots and passes its self-test in QEMU, and it can be installed next to OpenWrt on the BPI-R4's SD card for a first boot on hardware. See [PLAN.md](PLAN.md) for the design and roadmap.

## How it works

- **Kernel:** OpenWrt 25.12's BPI-R4 kernel (6.12, with MediaTek's patches), either the official release build or our own build with a few extra options ([docs/kernel.md](docs/kernel.md)).
- **OS:** Alpine Linux 3.24 in diskless mode. The OS is installed into RAM from a local package repository at every boot, so the boot media stays read-only and every boot starts clean.
- **Config:** native Alpine config files. `lbu commit` saves `/etc` to the boot media; `apk add` saves itself. `/var` (Docker, leases, logs) lives on a separate partition.
- **Boot:** OpenWrt's U-Boot loads a FIT image (kernel, device tree, Alpine initramfs) from its own partition ([docs/boot-flow.md](docs/boot-flow.md)).

## Build

Needs Docker on an arm64 machine (Apple silicon Mac or an arm64 Linux host).

```sh
scripts/build-image.sh --qemu   # out/bpir4 (device image) and out/qemu (test assets)
scripts/test-qemu.sh            # boot twice in QEMU and run the self-test
scripts/test-install-sd.sh      # test the SD installer against a replica card
```

Site-specific settings (admin user, SSH keys, port forwards, MAC addresses) go in the gitignored `site/` directory ([docs/site-config.md](docs/site-config.md)).

## Install on a BPI-R4 next to OpenWrt

Follow [docs/flashing-poc.md](docs/flashing-poc.md). You need a 3.3 V USB serial adapter ([docs/serial-console.md](docs/serial-console.md)).

## Layout

| Path | What |
| --- | --- |
| `kernel/` | OpenWrt release pin, kernel config additions, extra kernel modules, CI kernel build |
| `packages/` | Alpine packages (`gotthard-base`: boot services, defaults, board configs) |
| `image/` | Image builder, package list (`world`), config overlay |
| `scripts/` | Build, test and install helpers |
| `web/`, `ui/` | Web GUI: Rust backend, SvelteKit frontend |
| `docs/` | Design notes and runbooks |
