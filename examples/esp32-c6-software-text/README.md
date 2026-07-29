<!--
Copyright © SixtyFPS GmbH <info@slint.dev>
SPDX-License-Identifier: MIT
-->

# ESP32-C6 software-renderer text build

This compile-only example exercises Slint's normal imported-font workflow with changing Hebrew,
ASCII, numbers, combining marks, wrapping, and elision. It renders into an
in-memory RGB565 buffer; board-specific display transfer is intentionally left to the BSP.
The development profile optimizes for size and uses two OTA application slots in 4 MB of flash.
Each application image must fit in 1,984 KiB. The Cargo configuration uses Espressif's global tool
installation directory to keep native compiler paths short enough for Windows.

The checked-in font is fixed at weight 400 and subset by Unicode block, not by strings from the
UI source. It therefore supports arbitrary runtime strings within the included ASCII, Hebrew,
Hebrew presentation-form, and punctuation ranges. See [fonts/README.md](fonts/README.md) for the
reproducible commands.

Build it with the Espressif Rust toolchain:

```sh
cargo +esp build \
    --target riscv32imac-esp-espidf \
    -Zbuild-std=std,panic_abort
```

On Windows, ESP-IDF requires short native-tool and output paths. For example:

```powershell
$env:CARGO_TARGET_DIR = "C:\esp"
$env:ESP_IDF_TOOLS_INSTALL_DIR = "custom:C:\.embuild\espressif"
```

Validate the application against the first OTA slot:

```sh
espflash save-image \
    --chip esp32c6 \
    --flash-size 4mb \
    --partition-table partitions.csv \
    --target-app-partition ota_0 \
    <target>/riscv32imac-esp-espidf/debug/esp32-c6-software-text \
    esp32-c6-software-text.bin
```

Record the resulting size after every capability change. The final measured size appears below.

## Measured result

The size-optimized development build produces a 1,260,480-byte application image, including the
27,756-byte packaged font and the ESP-IDF application run time.
That uses 62.04% of the 2,031,616-byte OTA slot and leaves 771,136 bytes, or about 753.1 KiB, for
board support, display transfer, networking, and application code.

This result uses the `renderer-software-embedded-ttf-only` profile.
It keeps Unicode bidi, OpenType shaping, font fallback, wrapping, alignment, measurement, and
cluster-safe elision.

The example also enables ESP32-C6 NimBLE in `sdkconfig.defaults`. Because this sample does not
reference Bluetooth APIs, the linker removes the unused NimBLE implementation; the measured image
remains 1,260,480 bytes. Adding actual Bluetooth functionality will increase the image and should
be measured separately.

The flash savings come from accepting these rasterization and font restrictions:

- Fonts must use static TrueType `glyf` outlines.
- CFF, CFF2, variable, color, and bitmap-font rasterization is not included.
- Outline hinting is not included.
- The checked-in font has one real weight; other requested weights use lightweight synthesis.
- The font contains ASCII, Hebrew, Hebrew presentation forms, and punctuation rather than all
  Unicode scripts.

Malformed or unsupported fonts fail safely instead of panicking, but they cannot paint glyphs in
this compact profile.
