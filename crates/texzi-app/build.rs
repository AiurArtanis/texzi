// Emit a Win32 .res directly: no resource compiler or new dependency required.
// NSH loads RT_GROUP_ICON resource 1 for native window chrome.
use std::{env, fs, path::PathBuf};

fn word(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_le_bytes());
}

fn dword(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}

fn resource(out: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    dword(out, data.len() as u32);
    dword(out, 32); // Fixed header, ordinal type and name.
    for value in [0xffff, kind, 0xffff, id] {
        word(out, value);
    }
    dword(out, 0); // Data version.
    word(out, 0x1030); // MOVEABLE | PURE | DISCARDABLE.
    word(out, 0); // Language neutral.
    dword(out, 0); // Version.
    dword(out, 0); // Characteristics.
    out.extend(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn title_icon(frame: &[u8]) -> Vec<u8> {
    const SIZE: usize = 32;
    assert_eq!(
        &frame[..16],
        &[40, 0, 0, 0, 32, 0, 0, 0, 64, 0, 0, 0, 1, 0, 32, 0]
    );
    assert_eq!(&frame[16..20], &[0; 4]);
    let pixels = &frame[40..40 + SIZE * SIZE * 4];
    let mut rgba = Vec::with_capacity(pixels.len());
    for row in pixels.chunks_exact(SIZE * 4).rev() {
        for pixel in row.chunks_exact(4) {
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    rgba
}

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = fs::read("assets/app.ico").expect("read application icon");
    assert_eq!(&ico[..4], &[0, 0, 1, 0], "expected ICO file");
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut res = Vec::new();
    resource(&mut res, 0, 0, &[]); // Required null resource header.
    let mut group = ico[..6].to_vec();
    let mut title_pixels = None;
    for index in 0..count as usize {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let id = index as u16 + 1;
        let frame = &ico[offset..offset + size];
        resource(&mut res, 3, id, frame); // RT_ICON.
        if entry[..2] == [32, 32] {
            title_pixels = Some(title_icon(frame));
        }
        group.extend(&entry[..12]);
        word(&mut group, id);
    }
    resource(&mut res, 14, 1, &group); // RT_GROUP_ICON, NSH's expected ID.
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(
        out.join("app-icon.rgba"),
        title_pixels.expect("32 px application icon frame"),
    )
    .expect("write title icon");
    let path = out.join("app-icon.res");
    fs::write(&path, res).expect("write application icon resources");
    println!("cargo:rustc-link-arg-bin=texzi-app={}", path.display());
}
