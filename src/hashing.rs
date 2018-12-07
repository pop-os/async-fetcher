use digest::Digest;
use hex_view::HexView;
use std::{
    fs::File as SyncFile,
    io::{self, Read},
    path::Path,
};

pub fn hash_from_path<D: Digest>(path: &Path, checksum: &str) -> io::Result<()> {
    trace!("constructing hasher for {}", path.display());
    let reader = SyncFile::open(path)?;
    hasher::<D, SyncFile>(reader, checksum)
}

pub fn hasher<D: Digest, R: Read>(mut reader: R, checksum: &str) -> io::Result<()> {
    let mut buffer = [0u8; 8 * 1024];
    let mut hasher = D::new();

    loop {
        let read = reader.read(&mut buffer)?;

        if read == 0 {
            break;
        }
        hasher.input(&buffer[..read]);
    }

    let result = hasher.result();
    let hash = format!("{:x}", HexView::from(result.as_slice()));
    if hash == checksum {
        trace!("checksum is valid");
        Ok(())
    } else {
        debug!("invalid checksum found: {} != {}", hash, checksum);
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid checksum",
        ))
    }
}
