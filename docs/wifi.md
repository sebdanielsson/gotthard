# Wi-Fi 7 (prepared, not built yet)

The BPI-R4's Wi-Fi comes from an add-on card, the BPI-R4-NIC-BE14 (MT7995AV, MT7976CN, MT7977IAN), in the mini PCIe slots. It isn't installed yet, so the first version of Gotthard ships wired-only, with the groundwork for Wi-Fi in place.

## Already in place

| Piece | Where |
| --- | --- |
| Driver `mt7996e` with `mt76`, `mac80211`, `cfg80211` | Device default modules in the modloop (from OpenWrt's root filesystem) |
| Firmware `mediatek/mt7996/*` and the Wi-Fi offload firmware `mediatek/mt7988/mt7988_wo_*` | `modules/firmware` in the modloop |
| Device tree overlay `mt7988a-bananapi-bpi-r4-wifi-be14` | In the FIT image, same name as OpenWrt's |
| Boot command passes OpenWrt's `$bootconf_extra` | `boot_gotthard` in the U-Boot environment |
| `iw`, `wireless-regdb` | Image package list |

## Steps once the card is in

1. Enable the card's device tree overlay for both OpenWrt and Gotthard (the boot commands append `#$bootconf_extra`):

    ```sh
    fw_setenv bootconf_extra mt7988a-bananapi-bpi-r4-wifi-be14
    ```

2. Boot Gotthard and check the radios: `iw dev`, `iw list | grep -A20 'Supported interface modes'`, `dmesg | grep mt7996`.
3. Try Alpine's hostapd 2.11 (`apk add hostapd`) with one 5 GHz access point bridged into `br-lan`.
4. Test Wi-Fi 7 multi-link (MLO). If upstream hostapd 2.11 can't do what OpenWrt's can, package OpenWrt's hostapd snapshot (`package/network/services/hostapd` with its patches) for Alpine as `hostapd-gotthard`.
5. Add hostapd to the image, a board default config (`/etc/hostapd/` per radio, off until configured), and a Wi-Fi page in the GUI.
6. Wi-Fi offload (WED) between the radios and the Ethernet PPE comes with the hardware offload work (PLAN.md phase 6).

## Open questions

- Which hostapd: upstream 2.11 from Alpine, or OpenWrt's patched snapshot.
- Power and cooling: the card plus SFP+ modules need the 12 V/5.2 A or 19 V/3.2 A supply the Banana Pi wiki recommends, and a heatsink.
