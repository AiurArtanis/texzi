"""Gate package identity using the actual PE bytes; does not run the application."""
import argparse
import hashlib
import json
from pathlib import Path
import struct

def require(condition, message):
    if not condition:
        raise ValueError(message)

def verify(executable, icon):
    pe, ico = (executable.read_bytes(), icon.read_bytes())

    def u16(at):
        return struct.unpack_from('<H', pe, at)[0]

    def u32(at):
        return struct.unpack_from('<I', pe, at)[0]
    require(pe[:2] == b'MZ', 'Missing DOS header')
    header = u32(60)
    require(pe[header:header + 4] == b'PE\x00\x00', 'Missing PE header')
    require(u16(header + 4) == 34404, 'Expected Windows x64')
    optional = header + 24
    require(u16(optional) == 523, 'Expected PE32+')
    subsystem = u16(optional + 68)
    require(subsystem == 2, f'Expected Windows GUI subsystem 2, got {subsystem}')
    sections = header + 24 + u16(header + 20)

    def offset(rva, length=1):
        for index in range(u16(header + 6)):
            section = sections + index * 40
            va, size, raw = (u32(section + 12), u32(section + 16), u32(section + 20))
            if va <= rva and rva + length <= va + size:
                result = raw + rva - va
                require(result + length <= len(pe), 'Resource exceeds PE file')
                return result
        raise AssertionError(f'RVA {rva:x} not backed by raw PE data')
    resource_rva = u32(optional + 112 + 16)
    require(resource_rva, 'No resource directory')
    base = offset(resource_rva)

    def directory(relative):
        start = base + relative
        count = u16(start + 12) + u16(start + 14)
        return [(u32(start + 16 + i * 8), u32(start + 20 + i * 8)) for i in range(count)]
    images, groups = ({}, [])

    def walk(relative, trail=()):
        require(len(trail) < 4, 'Unexpected resource nesting')
        for name, target in directory(relative):
            require(not name & 2147483648, 'Expected ordinal resource identity')
            path = trail + (name,)
            if target & 2147483648:
                walk(target & 2147483647, path)
            else:
                entry = base + target
                size = u32(entry + 4)
                data = offset(u32(entry), size)
                value = pe[data:data + size]
                if path[0] == 3:
                    require(path[1] not in images, 'Duplicate icon language/resource')
                    images[path[1]] = value
                elif path[0] == 14:
                    groups.append((path, value))
    walk(0)
    require(len(groups) == 1 and groups[0][0][1] == 1, 'NSH requires RT_GROUP_ICON 1')
    group = groups[0][1]
    require(ico[:4] == b'\x00\x00\x01\x00' and group[:6] == ico[:6], 'Icon group header mismatch')
    count = struct.unpack_from('<H', ico, 4)[0]
    require(len(images) == count and len(group) == 6 + count * 14, 'Icon frame count mismatch')
    dimensions = []
    for index in range(count):
        source = ico[6 + index * 16:22 + index * 16]
        entry = group[6 + index * 14:20 + index * 14]
        require(source[:12] == entry[:12], 'Icon frame descriptor changed')
        size, at = struct.unpack_from('<II', source, 8)
        identity = struct.unpack_from('<H', entry, 12)[0]
        require(images[identity] == ico[at:at + size], 'Embedded icon frame bytes changed')
        dimensions.append([source[0] or 256, source[1] or 256])
    return {'exe': str(executable), 'exe_sha256': hashlib.sha256(pe).hexdigest().upper(), 'subsystem': subsystem, 'architecture': 'x86_64', 'icon_resource': 1, 'ico_sha256': hashlib.sha256(ico).hexdigest().upper(), 'frames': dimensions, 'scope': 'Static actual PE identity; Explorer launch and visible icon require application testing.'}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--ico', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    result = verify(args.exe, args.ico)
    args.report.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding='utf-8')
    print(json.dumps(result, ensure_ascii=False))
if __name__ == '__main__':
    main()
