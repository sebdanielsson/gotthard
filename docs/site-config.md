# Site config

Settings for one particular router live in `site/` at the top of the repository. It's gitignored, because it holds MAC addresses, SSH keys and port forwards. `scripts/build-image.sh` mounts it into the build; without it you get a generic image with user `admin`.

```text
site/
├── site.env              HOSTNAME, ADMIN_USER
├── authorized_keys       SSH public keys for ADMIN_USER
├── keys/                 optional: stable package signing key (*.rsa + *.rsa.pub)
└── overlay/              files copied over the root filesystem, saved in the apkovl
    └── etc/
        ├── gotthard/boards/bananapi-bpi-r4.interfaces   network config for that board
        ├── dnsmasq.d/50-site.conf                       static leases, DNS overrides
        └── nftables.d/50-site.nft                       port forwards, open ports
```

## site.env

```sh
HOSTNAME=gotthard
ADMIN_USER=sebastian
```

The build creates `ADMIN_USER` in groups `wheel` (sudo) and `docker`, generates random passwords for it and for root, and writes them to `out/bpir4/credentials.txt`. SSH only accepts keys, and only for members of `wheel`.

## Network config per board

`gotthard-board` installs `/etc/network/interfaces` at first boot from `/etc/gotthard/boards/<board>.interfaces` if the site provides one, otherwise from the packaged default. This lets one image boot both in QEMU (board `qemu-virt`) and on the router (board `bananapi-bpi-r4`). Once `/etc/network/interfaces` exists it's never overwritten.

## Firewall rules

Zones are defined as `WAN_IFS` and `LAN_IFS` in `/etc/nftables.d/00-zones.nft`. Add site rules to the `*_site` chains of table `inet gotthard`:

```nft
table inet gotthard {
    chain dstnat_site {
        iifname $WAN_IFS meta nfproto ipv4 meta l4proto { tcp, udp } th dport 443 dnat ip to 192.168.1.100:443 comment "https"
    }
    chain input_site {
        iifname $WAN_IFS udp dport 51820 accept comment "WireGuard"
    }
}
```

Apply with `sudo nft -f /etc/nftables.nft` and save with `sudo lbu commit`. Don't use `rc-service nftables reload`: it flushes every table, including Docker's.

## Package signing key

Without `site/keys`, every build signs its packages with a new throw-away key, which is fine for full reinstalls. For updates in place you want a stable key: generate one with `abuild-keygen -n` in an Alpine container and put both files in `site/keys/`.
