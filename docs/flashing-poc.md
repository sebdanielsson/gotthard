# Flashing the proof of concept (dual boot next to OpenWrt)

This installs Gotthard on the BPI-R4's SD card next to OpenWrt. OpenWrt's partitions and boot command are not changed: Gotthard gets four new partitions in the unused space and one extra entry in the U-Boot boot menu. OpenWrt stays the default, so a normal reboot always comes back to OpenWrt.

While Gotthard runs, it is the router: it uses the same LAN address (192.168.1.1), DHCP pool, static lease, port forwards and WAN MAC address as OpenWrt (from the site config), so the network should keep working.

## Before you start

- A working serial console ([serial-console.md](serial-console.md)). You need it to pick the boot menu entry.
- The router runs OpenWrt from the SD card (boot DIP switch on SD) and has internet access (the installer runs `apk add gdisk partx-utils`).
- SSH access to OpenWrt (`ssh root@bpi-r4`).
- A fresh build on the Mac, with the site config in `site/`:

```sh
scripts/build-image.sh
cat out/bpir4/credentials.txt   # passwords for root and the admin user
```

Recovery if anything goes wrong: the full OpenWrt on the SPI-NAND boots with the NAND DIP switch setting, and the installer saves the U-Boot environment and the partition table to `/root/gotthard-backup` before changing anything.

## 1. Copy the files to the router

OpenWrt has no SFTP server, so stream the files over SSH:

```sh
for f in out/bpir4/gotthard-bpir4.itb out/bpir4/media.tar.gz out/bpir4/SHA256SUMS scripts/install-sd-dualboot.sh; do
  ssh root@bpi-r4 "cat > /tmp/$(basename "$f")" < "$f"
done
ssh root@bpi-r4 'cd /tmp && grep -E "itb|media" SHA256SUMS | sha256sum -c'
```

## 2. Dry run

```sh
ssh root@bpi-r4 'sh /tmp/install-sd-dualboot.sh /tmp/gotthard-bpir4.itb /tmp/media.tar.gz'
```

It prints every command it would run and changes nothing. Check that it found OpenWrt's seven partitions and the card size.

## 3. Install

```sh
ssh root@bpi-r4 'sh /tmp/install-sd-dualboot.sh --apply /tmp/gotthard-bpir4.itb /tmp/media.tar.gz'
ssh root@bpi-r4 'tar -C /root -cf - gotthard-backup' | tar -C out -xf -
```

The second line copies the backups off the router.

## 4. Boot Gotthard

1. Open the serial console and run `reboot` on OpenWrt.
2. When the `( ( ( OpenWrt ) ) )` menu appears, press an arrow key within 3 seconds, select **Boot Gotthard (Alpine) from SD card** and press Enter.
3. Expect kernel messages, then the Alpine initramfs installing about 210 packages from the SD card, then OpenRC starting services. Watch for `[ !! ]` lines.
4. Log in on the console as the admin user with the password from `credentials.txt`, or over SSH from the LAN: `ssh sebastian@192.168.1.1` (expect a host key warning, the key differs from OpenWrt's).

## 5. Check it

```sh
cat /run/gotthard-board           # bananapi-bpi-r4
ip -br link                       # eth0, lan1-3, wan, sfp-lan, sfp-wan, br-lan, br-wan
ip -br addr                       # 192.168.1.1 on br-lan, WAN address on br-wan
rc-status --all | grep -v started # anything not started
sudo nft list table inet gotthard
ping -c 3 1.1.1.1 && ping -c 3 alpinelinux.org
docker run --rm hello-world
free -m; df -h / /var /media/*
dmesg | grep -iE 'error|fail|firmware' | head -40
```

From a LAN client, check that it got a DHCP lease and has internet (IPv4 and IPv6).

Save evidence for later; `/var` is persistent:

```sh
sudo sh -c 'dmesg > /var/log/first-boot-dmesg.txt; rc-status --all > /var/log/first-boot-rc-status.txt; lsmod > /var/log/first-boot-lsmod.txt'
```

The web GUI (`gotthard-web`) listens on localhost only until TLS is done. Open it through an SSH tunnel and log in with the admin user's password:

```sh
ssh -L 8080:127.0.0.1:8080 sebastian@192.168.1.1   # then open http://localhost:8080
```

## 6. Back to OpenWrt

`sudo reboot` and do nothing: the menu's default entry boots OpenWrt after 3 seconds.

## Try the CI kernel (optional, after the first boot works)

The CI-built kernel adds devtmpfs, a compressed initramfs and a few other options ([kernel.md](kernel.md)). To try it, build with the kernel bundle and run the installer again. It reuses the partitions and keeps `/var`; config in `/etc` comes from the new apkovl, so commit anything you changed with `sudo lbu commit` and copy it out first, or rebuild with it in `site/`.

```sh
gh run download <kernel-run-id> -R sebdanielsson/gotthard -p 'kernel-bundle-bpir4*' -D out/kernel-bundle.tmp
mv out/kernel-bundle.tmp/kernel-bundle-bpir4* out/kernel-bundle && rmdir out/kernel-bundle.tmp
scripts/build-image.sh
```

Then repeat steps 1, 3 and 4.

## Remove Gotthard

```sh
ssh root@bpi-r4 'sh /tmp/install-sd-dualboot.sh --apply --uninstall'
```

This deletes partitions 8-11 and the U-Boot variables the installer added.

## Known limits of the proof of concept

- Uses the official OpenWrt 25.12.6 kernel: no devtmpfs, so `/dev` is managed by BusyBox `mdev`, and the initramfs is uncompressed.
- No Wi-Fi configuration (the BE14 card isn't installed yet), no DNS-over-HTTPS proxy (dnsmasq uses the ISP's DNS servers from DHCP), and Tailscale isn't logged in.
- The web GUI is read-only (system, interfaces, firewall ruleset) and only reachable through SSH. TOTP and API keys created in it are saved only after `sudo lbu commit`.
- Hardware NAT offload is off; forwarding is in software.
- Booting Gotthard always needs the serial console to pick the menu entry.
