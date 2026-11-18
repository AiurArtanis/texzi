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

fn main() {
    println!("cargo:rerun-if-changed=assets/ink-z.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ico = fs::read("assets/ink-z.ico").expect("read Ink Z icon");
    assert_eq!(&ico[..4], &[0, 0, 1, 0], "expected ICO file");
    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut res = Vec::new();
    resource(&mut res, 0, 0, &[]); // Required null resource header.
    let mut group = ico[..6].to_vec();
    for index in 0..count as usize {
        let entry = &ico[6 + index * 16..6 + (index + 1) * 16];
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let id = index as u16 + 1;
        resource(&mut res, 3, id, &ico[offset..offset + size]); // RT_ICON.
        group.extend(&entry[..12]);
        word(&mut group, id);
    }
    resource(&mut res, 14, 1, &group); // RT_GROUP_ICON, NSH's expected ID.
    let path = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("ink-z.res");
    fs::write(&path, res).expect("write Ink Z resources");
    println!("cargo:rustc-link-arg-bin=texzi-app={}", path.display());
}
