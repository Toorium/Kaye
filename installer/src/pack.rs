//! The installer is one file: the setup program, with Kaye squeezed onto the end of it.
//!
//! Keeping Kaye out of the setup program's own code means the two can be built in any order,
//! and that the first part of the file on its own is a working uninstaller.
//!
//! ```text
//! setup program | deflated kaye.exe | MAGIC, setup size, kaye.exe size, checksum
//! ```

use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

const MAGIC: &[u8; 8] = b"KAYEPACK";
const FOOTER: usize = MAGIC.len() + 8 + 8 + 4;
/// How far back from the end to look for the footer. A code signature goes after it, so it
/// can't be expected at the very end.
const LOOK_BACK: u64 = 256 * 1024;
const CHUNK: usize = 256 * 1024;

/// Where the packed program is in the installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Payload {
    /// How long the setup program is, which is also where the packed program starts.
    pub stub: u64,
    pub packed: u64,
    /// How long the program is once it's unpacked.
    pub size: u64,
    checksum: u32,
}

pub fn pack(stub: &[u8], program: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = DeflateEncoder::new(stub.to_vec(), Compression::best());
    encoder.write_all(program)?;
    let mut file = encoder.finish()?;
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(&(stub.len() as u64).to_le_bytes());
    file.extend_from_slice(&(program.len() as u64).to_le_bytes());
    file.extend_from_slice(&crc32fast::hash(program).to_le_bytes());
    Ok(file)
}

/// Finds the packed program. `None` means this is the plain setup program on its own.
pub fn find<F: Read + Seek>(file: &mut F) -> io::Result<Option<Payload>> {
    let length = file.seek(SeekFrom::End(0))?;
    let start = length.saturating_sub(LOOK_BACK);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)?;
    let Some(at) = tail.windows(MAGIC.len()).rposition(|window| window == MAGIC) else {
        return Ok(None);
    };
    let Some(footer) = tail.get(at..at + FOOTER) else {
        return Ok(None);
    };
    let number = |from: usize| u64::from_le_bytes(footer[from..from + 8].try_into().expect("eight bytes"));
    let (stub, size) = (number(8), number(16));
    let checksum = u32::from_le_bytes(footer[24..28].try_into().expect("four bytes"));
    let footer_at = start + at as u64;
    if stub > footer_at {
        return Ok(None);
    }
    Ok(Some(Payload { stub, packed: footer_at - stub, size, checksum }))
}

/// Writes the program out, saying how far along it is from 0 to 1. A checksum that doesn't
/// match is an error, since that means the installer was damaged on its way here.
pub fn unpack<F: Read + Seek, W: Write>(file: &mut F, payload: &Payload, out: &mut W, mut progress: impl FnMut(f32)) -> io::Result<()> {
    file.seek(SeekFrom::Start(payload.stub))?;
    let mut decoder = DeflateDecoder::new(BufReader::new(file.by_ref().take(payload.packed)));
    let mut chunk = vec![0; CHUNK];
    let mut hasher = crc32fast::Hasher::new();
    let mut written = 0u64;
    let mut shown = 0.0;
    loop {
        // Anything going wrong in here is the installer, not the disk being written to.
        let count = decoder.read(&mut chunk).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
        out.write_all(&chunk[..count])?;
        written += count as u64;
        // A read can be tiny, and nobody can see less than a percent.
        let share = written as f32 / payload.size.max(1) as f32;
        if share - shown >= 0.01 || share >= 1.0 {
            shown = share;
            progress(share);
        }
    }
    if written != payload.size || hasher.finalize() != payload.checksum {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the packed program doesn't match its checksum"));
    }
    Ok(())
}

/// The setup program by itself, which goes beside Kaye to uninstall it.
pub fn stub_bytes<F: Read + Seek>(file: &mut F, payload: &Payload) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut stub = vec![0; payload.stub as usize];
    file.read_exact(&mut stub)?;
    Ok(stub)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn program() -> Vec<u8> {
        (0..300_000u32).flat_map(|i| (i * 7 % 251).to_le_bytes()).collect()
    }

    #[test]
    fn what_goes_in_comes_back_out() {
        let (stub, program) = (b"setup program".repeat(50), program());
        let mut file = Cursor::new(pack(&stub, &program).unwrap());
        let payload = find(&mut file).unwrap().expect("it was packed");
        assert_eq!(payload.stub, stub.len() as u64);
        assert_eq!(payload.size, program.len() as u64);
        assert!(payload.packed < payload.size, "it should be squeezed");

        let mut steps = Vec::new();
        let mut out = Vec::new();
        unpack(&mut file, &payload, &mut out, |share| steps.push(share)).unwrap();
        assert_eq!(out, program);
        assert!(steps.windows(2).all(|w| w[0] <= w[1]), "progress only goes forward");
        assert_eq!(*steps.last().unwrap(), 1.0);
        assert_eq!(stub_bytes(&mut file, &payload).unwrap(), stub);
    }

    #[test]
    fn the_setup_program_alone_has_nothing_packed() {
        assert_eq!(find(&mut Cursor::new(b"just a program".repeat(100))).unwrap(), None);
        assert_eq!(find(&mut Cursor::new(Vec::new())).unwrap(), None);
    }

    #[test]
    fn a_signature_after_the_footer_doesnt_hide_it() {
        let mut bytes = pack(b"setup", &program()).unwrap();
        bytes.extend_from_slice(&[9; 16 * 1024]);
        let mut file = Cursor::new(bytes);
        let payload = find(&mut file).unwrap().expect("it's still there");
        let mut out = Vec::new();
        unpack(&mut file, &payload, &mut out, |_| {}).unwrap();
        assert_eq!(out, program());
    }

    #[test]
    fn a_damaged_download_is_caught() {
        let stub = b"setup".to_vec();
        let mut bytes = pack(&stub, &program()).unwrap();
        let middle = stub.len() + (bytes.len() - stub.len()) / 2;
        bytes[middle] ^= 0xFF;
        let mut file = Cursor::new(bytes);
        let payload = find(&mut file).unwrap().expect("the footer's fine");
        assert!(unpack(&mut file, &payload, &mut std::io::sink(), |_| {}).is_err());
    }

    #[test]
    fn a_cut_off_download_is_caught() {
        let bytes = pack(b"setup", &program()).unwrap();
        let mut file = Cursor::new(bytes[..bytes.len() / 2].to_vec());
        assert_eq!(find(&mut file).unwrap(), None, "with the footer gone it reads as no program at all");
    }
}
