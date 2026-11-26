# Satin Z application icon

`app.ico` is the Windows export of the user-selected Satin Z (轻缎 Z) artwork,
version 1.0 from 2026-11-16. The design handoff derived this export from the
transparent 512×512 PNG supplied in `texzi-satin-z-developer-pack-v1.zip`.
Pillow 13.0.0 resampled each size independently with Lanczos and encoded
32-bit BGRA DIB frames with alpha and a transparency mask. [Source PNG](https://github.com/AiurArtanis/texzi/blob/093b82b09c5550bb8e93414a697e8b41a9dd4640/assets/branding/satin-z/icon-512.png)

| Input or output | SHA-256 |
| --- | --- |
| Source developer ZIP | `24BC5F527F32883FFA7F42CD48BF2903F17346AEC5EC6328F8B3AE7957695358` |
| Source `icon-512.png` | `13158D006AE764E04DFB4FEE0CA9441532B5064B6053D52D7B4EA5363D5C2241` |
| Derived `app.ico` | `394057449EA52533A08D48040289B9C63AB2756B9E6A191E0CC91145EDB5DD0F` |

The ICO contains 16, 20, 24, 32, 40, 48, 64, 96, 128 and 256 pixel frames.
The source pack describes the original as AI-generated artwork and provides
no separate license for the image.

`build.rs` embeds the frames as Win32 `RT_ICON` resources and `RT_GROUP_ICON`
1. NSH loads group 1 as the window class icon. The custom title bar takes
its RGBA image from the same ICO's 32px frame. The portable ZIP copies the
executable with its embedded icon and the source record.
