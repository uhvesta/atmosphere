//! The Linux builds of `stack` are static: an ELF executable for the right
//! CPU with no interpreter and no shared library to load, so it runs on any
//! Linux box Humata links, whatever its libc (D411). A static-pie build has a
//! dynamic section, for relocating itself, but needs nothing from it.

use std::path::PathBuf;

/// ELF program header types.
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;

/// The dynamic section's tag for a shared library the program needs, and
/// its end.
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;

/// `e_machine` values.
const EM_X86_64: u16 = 0x3e;
const EM_AARCH64: u16 = 0xb7;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

/// A program header: its type, and where its bytes are in the file.
struct Segment {
    kind: u32,
    offset: usize,
    size: usize,
}

fn usize_at(bytes: &[u8], at: usize) -> usize {
    usize::try_from(u64_at(bytes, at)).expect("fits in memory")
}

/// The machine and the program headers of a 64-bit little-endian ELF.
fn elf(bytes: &[u8]) -> (u16, Vec<Segment>) {
    assert!(bytes.len() > 64 && bytes[..4] == *b"\x7fELF", "not an ELF file");
    assert_eq!(bytes[4], 2, "not a 64-bit ELF");
    assert_eq!(bytes[5], 1, "not little-endian");
    let machine = u16_at(bytes, 0x12);
    let offset = usize_at(bytes, 0x20);
    let size = usize::from(u16_at(bytes, 0x36));
    let count = usize::from(u16_at(bytes, 0x38));
    let segments = (0..count)
        .map(|index| {
            let at = offset + index * size;
            Segment {
                kind: u32_at(bytes, at),
                offset: usize_at(bytes, at + 8),
                size: usize_at(bytes, at + 32),
            }
        })
        .collect();
    (machine, segments)
}

/// How many shared libraries a dynamic section asks the loader for.
fn needed(bytes: &[u8], dynamic: &Segment) -> usize {
    let mut count = 0;
    for at in (dynamic.offset..dynamic.offset + dynamic.size).step_by(16) {
        match u64_at(bytes, at) {
            DT_NULL => break,
            DT_NEEDED => count += 1,
            _ => {}
        }
    }
    count
}

#[test]
fn every_linux_build_is_a_static_executable_for_its_cpu() {
    let root = PathBuf::from(std::env::var("STACK_LINUX").expect("STACK_LINUX names the builds folder"));
    for (platform, machine) in [("linux-aarch64", EM_AARCH64), ("linux-x86_64", EM_X86_64)] {
        let path = root.join(platform).join("stack");
        let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let (found, segments) = elf(&bytes);
        assert_eq!(found, machine, "{platform}: wrong CPU");
        assert!(
            segments.iter().all(|segment| segment.kind != PT_INTERP),
            "{platform}: has an interpreter, so it is dynamically linked"
        );
        for dynamic in segments.iter().filter(|segment| segment.kind == PT_DYNAMIC) {
            assert_eq!(needed(&bytes, dynamic), 0, "{platform}: needs shared libraries");
        }
    }
}
