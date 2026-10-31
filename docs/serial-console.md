# Serial console

The BPI-R4's debug UART is the only way to see U-Boot, choose a boot menu entry, and log in when networking doesn't come up. Get it working before installing anything.

## Adapter

Use a 3.3 V USB-to-TTL adapter with an FTDI chip. Banana Pi's forum reports garbled output with Prolific adapters on the R3 and R4.

Bought for this project: Electrokit art. 41035672, "USB-A till UART adapter (TTL) 3.3/5V 8-pin med flödeskontroll" (FTDI FT232RNL, 3.3/5 V switch, comes with a cable with female jumper leads). It has a USB-A plug, so a Mac with only USB-C needs a USB-C to USB-A adapter.

## Wiring

- Set the adapter's voltage switch to **3.3 V** before connecting anything.
- Use the 3-pin debug UART header on the side of the board (labelled G, RX, TX), not the 26-pin GPIO header.
- Connect only three wires, crossing TX and RX:

| Adapter | BPI-R4 debug UART |
| --- | --- |
| GND | G |
| TXD | RX |
| RXD | TX |

Leave the adapter's VCC and 5V pins unconnected; the board has its own power supply.

## Terminal on macOS

macOS includes the FTDI driver. Plug in the adapter and find its device:

```sh
ls /dev/cu.usbserial-*
```

Connect at 115200 baud, 8N1, no flow control:

```sh
screen /dev/cu.usbserial-XXXXXXXX 115200
```

Quit `screen` with Ctrl-A then K. `tio` (`brew install tio`) is a friendlier alternative: `tio -b 115200 /dev/cu.usbserial-XXXXXXXX`.

## Check it

With the terminal open, power-cycle the router. You should see BL2 and U-Boot messages, then the `( ( ( OpenWrt ) ) )` boot menu with a 3-second countdown. Use the arrow keys and Enter to pick an entry. If you see nothing, swap TX and RX; if you see garbage, check the baud rate and the 3.3 V setting.
