# Gotthard plan

Working name: Gotthard, after the Alpine pass and tunnel that carries traffic through the Alps. Renaming is cheap now and gets expensive once packages and config paths ship.

## Goal

A small, efficient router OS that also runs the owner's services (Docker containers, web servers). It should feel like normal Linux (bash, sudo, familiar tools), be hard to break (read-only OS, atomic updates), use an nftables firewall, and offer a web GUI. Personal first, published later. First board: Banana Pi BPI-R4 (MT7988A, 4x Cortex-A73, 4 GB RAM, 8 GB eMMC, 2x SFP+); then x86_64.

## Decisions

| Area | Decision | Notes |
| --- | --- | --- |
| Base | Alpine Linux 3.24, diskless "data disk" mode | OS in RAM, config in an apkovl on the boot media, `/var` on disk |
| Init and devices | OpenRC; BusyBox `mdev` now, `mdevd` once the kernel has devtmpfs | systemd on musl is still experimental |
| Userland | uutils-coreutils, bash, sudo; BusyBox for the rest | uutils 0.11 has gaps (`df -m` is unsupported); CI boots catch regressions |
| Kernel (BPI-R4) | OpenWrt 25.12's kernel with MediaTek's patches | Official release build for the PoC, own CI build next ([docs/kernel.md](docs/kernel.md)) |
| Kernel (x86_64) | Alpine `linux-lts` | Also the CI/QEMU path today (`linux-virt`) |
| Boot | OpenWrt's existing U-Boot loads a FIT image (kernel, DTBs, initramfs) | No bootloader changes |
| Updates | A/B FIT images and boot sets on the boot media, U-Boot fallback | Partitions for slot B already exist |
| Storage | eMMC only (8 GB): OS slots plus boot media plus `/var` | PoC runs from the SD card next to OpenWrt |
| Containers | Docker + Compose, `firewall-backend: nftables` | Docker keeps its own nftables tables |
| Network | ifupdown-ng, dhcpcd (WAN, DHCPv6-PD), dnsmasq (DHCP, DNS, RA) | |
| Firewall | Own nftables ruleset (`table inet gotthard`) with zones; hardware flowtable offload later | firewalld can't do hardware offload |
| Config | Native files only; the GUI owns files it marks, shows hand-edited ones read-only | Apply with snapshot and timed rollback, then `lbu commit` |
| GUI | Rust backend (axum), SvelteKit static frontend with shadcn-svelte | One static binary that embeds the frontend |
| Auth | System accounts (`/etc/shadow`, group `wheel`), sessions, TOTP, passkeys, API keys | Rust crates: tower-sessions, axum-login, totp-rs, webauthn-rs |
| v1 features | WireGuard + Tailscale, DNS ad-blocking, per-device internet pause, container management | |
| Wi-Fi | Wi-Fi 7 (BE14 card) planned for v1, prepared but not built yet | Kernel already has the mt7996 driver and firmware |
| Throughput target | 10 Gbps | Needs PPE hardware offload |
| Pinning | Everything pinned: OpenWrt commit and file hashes, image digests, action SHAs, Debian snapshot, lockfiles | Alpine packages follow the 3.24 branch; each build records exact versions in `packages.lock` |

## Architecture

```text
U-Boot (OpenWrt, unchanged)
  └─ FIT image: OpenWrt kernel + BPI-R4 DTBs + Alpine initramfs        partition gotthard_a (later A/B)
       └─ initramfs finds GTMEDIA: boot repository, apkovl, modloop      partition gtmedia
            └─ tmpfs root: installs world packages, applies apkovl
                 └─ OpenRC: board detect, /var from GTVAR, firewall,      partition gtvar
                    networking, dhcpcd, dnsmasq, sshd, Docker, Tailscale,
                    gotthard-web (GUI)
```

Packages:

- `gotthard-base`: boot services (`gotthard-board`, `gotthard-var`, `gotthard-media`, `gotthard-selftest`), the apk commit hook that persists `apk add`, default configs (sysctl, sudoers, sshd, dnsmasq, Docker, nftables), and per-board network defaults.
- `gotthard-web` (next): the GUI.
- `linux-bpir4` (later): the kernel as an Alpine package, so `update-kernel` and A/B updates can handle it.

## Phases

### 0. Foundations (done)

- [x] Repository, pinned CI (one `ci` gate: lint, web, image build plus QEMU boot test; separate kernel build), Renovate
- [x] Image builder: FIT image, modloop from OpenWrt modules plus extra kmods, Alpine boot repository, apkovl from site config
- [x] `gotthard-base` package with boot services and defaults
- [x] QEMU self-test: two boots, checks services, `apk add`, `lbu commit` and `/var` persistence
- [x] SD dual-boot installer, tested against a replica of the router's card
- [x] Runbooks: serial console, flashing, site config
- [x] Web GUI skeleton: login (system accounts, TOTP), API keys, dashboard, firewall viewer; packaged as `gotthard-web`, listens on localhost

### 1. First boot on hardware (waiting for the serial adapter)

- [ ] Flash with [docs/flashing-poc.md](docs/flashing-poc.md)
- [ ] Ethernet: DSA ports, both SFP+ ports, MAC addresses, bridges
- [ ] Routing: WAN DHCPv4 and DHCPv6-PD, LAN DHCP/RA, NAT, port forwards, firewall
- [ ] Docker container with a published port
- [ ] Measure: boot time, RAM, routing throughput with iperf3 (software forwarding)
- [ ] Note every difference from OpenWrt in an issue

### 2. Own kernel

- [ ] Run the kernel workflow; boot the CI kernel bundle (devtmpfs, compressed initramfs, PSI)
- [ ] Switch `mdev` to `mdevd`
- [ ] Package the kernel as `linux-bpir4`

### 3. Updates

- [ ] Boot media layout per slot (FIT, modloop, boot repository), shared apkovl and apk cache
- [ ] U-Boot: boot counter and fallback to the other slot, without changing OpenWrt's default until Gotthard moves to eMMC
- [ ] Stable signing key in CI, signed update bundles, `gotthard-update` CLI
- [ ] Health check after boot (network up, GUI up) marks a slot good

### 4. GUI v1

- [ ] Package `gotthard-web`, TLS (self-signed, ACME later), OpenRC service
- [ ] Dashboard, interfaces, firewall zones and port forwards, DHCP leases and static leases, DNS, services, logs
- [ ] Apply with snapshot and timed rollback, then `lbu commit`
- [ ] Auth hardening: passkeys, TOTP, API keys with scopes, rate limiting

### 5. v1 features

- [ ] WireGuard and Tailscale pages
- [ ] DNS ad-blocking (blocklists for dnsmasq, scheduled updates)
- [ ] Per-device internet pause (port of luci-app-netpause: nftables sets, cut existing and offloaded flows)
- [ ] Container management (Docker API, Compose stacks, logs, updates)

### 6. Performance

- [ ] Software flowtable, then hardware offload (`flags offload`, MediaTek PPE) and 10 Gbps measurements
- [ ] Make sure blocking and firewall changes flush offloaded flows

### 7. Wi-Fi 7

- [ ] BE14 card: mt7996 firmware from the modloop, Wi-Fi offload (WED)
- [ ] hostapd: test Alpine's 2.11; package OpenWrt's patched hostapd if multi-link (MLO) needs it
- [ ] GUI page

### 8. eMMC, x86_64, release

- [ ] Installer for the eMMC (from the SD card or the NAND OpenWrt)
- [ ] x86_64 image with Alpine `linux-lts`
- [ ] Public release: install docs, signed update feed, issue templates

## Risks

| Risk | Impact | Mitigation |
| --- | --- | --- |
| OpenWrt's kernel and U-Boot behave differently outside OpenWrt (module autoload, netdev names, `fitblk`) | PoC doesn't boot or lacks ports | Same device tree and module list as OpenWrt; serial console; QEMU covers the userland |
| uutils 0.11 incompatibilities in Alpine's own scripts | Boot or `lbu` breaks | CI boot test exercises `lbu`, `apk` and services; GNU coreutils as fallback |
| RAM use of a diskless OS with Docker | Less RAM for services | About 450 MiB root filesystem measured; trim the world list |
| Alpine package versions move inside 3.24 | Builds aren't bit-for-bit reproducible | `packages.lock` per build; the apk cache keeps exact files |
| hostapd for Wi-Fi 7 | Wi-Fi 7 features missing | Package OpenWrt's hostapd for Alpine |
| Hardware offload with an nftables ruleset that isn't fw4 | 10 Gbps not reached | Test early in phase 6; same kernel as OpenWrt |
| Kernel maintenance follows OpenWrt releases | Ongoing work | Bumps are one file plus CI |
| SD/eMMC wear from `/var` and Docker | Shorter storage life | `noatime`, Docker local log driver with rotation; NVMe later if needed |

## Open questions for the owner

- Name: keep "Gotthard"?
- License for the repository and packages.
- Tailscale: Alpine 3.24 ships 1.98; use upstream's static builds to stay current?
- When Gotthard moves to the eMMC, does OpenWrt stay on the SD card or NAND as a rescue system?
