//! Paged directory manifests and chunked file manifests with binary and JSON
//! serialization.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cdc::ChunkSlice;
use crate::hash::{ChunkHash, HASH_LEN};

const FILE_MANIFEST_MAGIC: &[u8; 4] = b"FCFM";
const DIR_PAGE_MAGIC: &[u8; 4] = b"FCDP";
const DIR_MANIFEST_MAGIC: &[u8; 4] = b"FCDM";
const FORMAT_VERSION: u8 = 1;

/// Errors returned while serializing or deserializing manifests.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("json serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid binary magic header")]
    InvalidMagic,
    #[error("unsupported binary format version: {0}")]
    UnsupportedVersion(u8),
    #[error("unexpected end of binary manifest")]
    UnexpectedEof,
    #[error("invalid entry kind tag: {0}")]
    InvalidEntryKind(u8),
    #[error("invalid utf-8 string in binary manifest: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
}

/// Filesystem entry type inside a directory page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

impl EntryKind {
    fn to_tag(self) -> u8 {
        match self {
            Self::File => 1,
            Self::Directory => 2,
            Self::Symlink => 3,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, ManifestError> {
        match tag {
            1 => Ok(Self::File),
            2 => Ok(Self::Directory),
            3 => Ok(Self::Symlink),
            other => Err(ManifestError::InvalidEntryKind(other)),
        }
    }
}

/// Metadata for one entry in a [`DirPage`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: u64,
    pub mode: u32,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub manifest_hash: Option<ChunkHash>,
}

/// One page of a paginated directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirPage {
    pub entries: Vec<DirEntry>,
    pub next_cursor: Option<String>,
    pub generation: u64,
}

impl DirPage {
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_binary_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(DIR_PAGE_MAGIC);
        out.push(FORMAT_VERSION);
        encode_dir_page_body(self, &mut out);
        out
    }

    pub fn from_binary_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let mut r = BinReader::new(bytes);
        r.expect_header(DIR_PAGE_MAGIC)?;
        decode_dir_page_body(&mut r)
    }
}

/// Complete paged directory manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirManifest {
    pub generation: u64,
    pub pages: Vec<DirPage>,
}

impl DirManifest {
    /// Splits `entries` into deterministic pages of at most `page_size` entries.
    pub fn paginate(entries: Vec<DirEntry>, page_size: usize, generation: u64) -> Self {
        assert!(page_size > 0, "page_size must be non-zero");
        if entries.is_empty() {
            return Self {
                generation,
                pages: vec![DirPage {
                    entries: Vec::new(),
                    next_cursor: None,
                    generation,
                }],
            };
        }
        let total_pages = entries.len().div_ceil(page_size);
        let mut pages = Vec::with_capacity(total_pages);
        let mut iter = entries.into_iter();
        for page_idx in 0..total_pages {
            let chunk: Vec<DirEntry> = iter.by_ref().take(page_size).collect();
            let next_cursor = if page_idx + 1 < total_pages {
                Some(format!("p{}", page_idx + 1))
            } else {
                None
            };
            pages.push(DirPage {
                entries: chunk,
                next_cursor,
                generation,
            });
        }
        Self { generation, pages }
    }

    /// Returns the page addressed by `cursor` (`None` returns the first page).
    pub fn page(&self, cursor: Option<&str>) -> Option<&DirPage> {
        match cursor {
            None => self.pages.first(),
            Some(c) => {
                let idx: usize = c.strip_prefix('p')?.parse().ok()?;
                self.pages.get(idx)
            }
        }
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_binary_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(DIR_MANIFEST_MAGIC);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.generation.to_le_bytes());
        out.extend_from_slice(&(self.pages.len() as u32).to_le_bytes());
        for page in &self.pages {
            encode_dir_page_body(page, &mut out);
        }
        out
    }

    pub fn from_binary_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let mut r = BinReader::new(bytes);
        r.expect_header(DIR_MANIFEST_MAGIC)?;
        let generation = r.read_u64()?;
        let page_count = r.read_u32()? as usize;
        let mut pages = Vec::with_capacity(page_count);
        for _ in 0..page_count {
            pages.push(decode_dir_page_body(&mut r)?);
        }
        Ok(Self { generation, pages })
    }
}

/// Reference to one content-addressed chunk within a [`FileManifest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub offset: u64,
    pub len: u32,
    pub hash: ChunkHash,
}

impl ChunkRef {
    #[inline]
    pub const fn end(&self) -> u64 {
        self.offset + self.len as u64
    }
}

impl From<ChunkSlice> for ChunkRef {
    fn from(s: ChunkSlice) -> Self {
        Self {
            offset: s.offset,
            len: s.len,
            hash: s.hash,
        }
    }
}

/// Ordered chunk table describing a file at a specific generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifest {
    pub size: u64,
    pub generation: u64,
    pub chunks: Vec<ChunkRef>,
}

impl FileManifest {
    /// Builds a [`FileManifest`] from FastCDC [`ChunkSlice`]s.
    pub fn from_slices(size: u64, generation: u64, slices: Vec<ChunkSlice>) -> Self {
        Self {
            size,
            generation,
            chunks: slices.into_iter().map(ChunkRef::from).collect(),
        }
    }

    /// Returns the chunk covering `offset`, if any.
    pub fn chunk_at(&self, offset: u64) -> Option<&ChunkRef> {
        let idx = self.chunks.partition_point(|c| c.end() <= offset);
        let candidate = self.chunks.get(idx)?;
        if offset >= candidate.offset && offset < candidate.end() {
            Some(candidate)
        } else {
            None
        }
    }

    /// Returns all chunks overlapping `[offset, offset + len)`.
    pub fn chunks_in_range(&self, offset: u64, len: u64) -> Vec<&ChunkRef> {
        if len == 0 {
            return Vec::new();
        }
        let end = offset.saturating_add(len);
        let start_idx = self.chunks.partition_point(|c| c.end() <= offset);
        let mut result = Vec::new();
        for chunk in &self.chunks[start_idx..] {
            if chunk.offset >= end {
                break;
            }
            result.push(chunk);
        }
        result
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn to_binary_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(25 + self.chunks.len() * (8 + 4 + HASH_LEN));
        out.extend_from_slice(FILE_MANIFEST_MAGIC);
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&self.size.to_le_bytes());
        out.extend_from_slice(&self.generation.to_le_bytes());
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        for c in &self.chunks {
            out.extend_from_slice(&c.offset.to_le_bytes());
            out.extend_from_slice(&c.len.to_le_bytes());
            out.extend_from_slice(c.hash.as_bytes());
        }
        out
    }

    pub fn from_binary_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let mut r = BinReader::new(bytes);
        r.expect_header(FILE_MANIFEST_MAGIC)?;
        let size = r.read_u64()?;
        let generation = r.read_u64()?;
        let count = r.read_u32()? as usize;
        let mut chunks = Vec::with_capacity(count);
        for _ in 0..count {
            let offset = r.read_u64()?;
            let len = r.read_u32()?;
            let hash = r.read_hash()?;
            chunks.push(ChunkRef { offset, len, hash });
        }
        Ok(Self {
            size,
            generation,
            chunks,
        })
    }
}

fn encode_string(s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn encode_opt_string(opt: &Option<String>, out: &mut Vec<u8>) {
    match opt {
        None => out.push(0),
        Some(s) => {
            out.push(1);
            encode_string(s, out);
        }
    }
}

fn encode_dir_page_body(page: &DirPage, out: &mut Vec<u8>) {
    out.extend_from_slice(&page.generation.to_le_bytes());
    encode_opt_string(&page.next_cursor, out);
    out.extend_from_slice(&(page.entries.len() as u32).to_le_bytes());
    for entry in &page.entries {
        encode_string(&entry.name, out);
        out.push(entry.kind.to_tag());
        out.extend_from_slice(&entry.size.to_le_bytes());
        out.extend_from_slice(&entry.mode.to_le_bytes());
        out.extend_from_slice(&entry.mtime_secs.to_le_bytes());
        out.extend_from_slice(&entry.mtime_nanos.to_le_bytes());
        match &entry.manifest_hash {
            None => out.push(0),
            Some(h) => {
                out.push(1);
                out.extend_from_slice(h.as_bytes());
            }
        }
    }
}

fn decode_dir_page_body(r: &mut BinReader<'_>) -> Result<DirPage, ManifestError> {
    let generation = r.read_u64()?;
    let next_cursor = r.read_opt_string()?;
    let count = r.read_u32()? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.read_string()?;
        let kind = EntryKind::from_tag(r.read_u8()?)?;
        let size = r.read_u64()?;
        let mode = r.read_u32()?;
        let mtime_secs = r.read_i64()?;
        let mtime_nanos = r.read_u32()?;
        let manifest_hash = match r.read_u8()? {
            0 => None,
            _ => Some(r.read_hash()?),
        };
        entries.push(DirEntry {
            name,
            kind,
            size,
            mode,
            mtime_secs,
            mtime_nanos,
            manifest_hash,
        });
    }
    Ok(DirPage {
        entries,
        next_cursor,
        generation,
    })
}

struct BinReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> BinReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ManifestError> {
        let end = self.pos.checked_add(n).ok_or(ManifestError::UnexpectedEof)?;
        if end > self.buf.len() {
            return Err(ManifestError::UnexpectedEof);
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn expect_header(&mut self, magic: &[u8; 4]) -> Result<(), ManifestError> {
        let got = self.take(4)?;
        if got != magic {
            return Err(ManifestError::InvalidMagic);
        }
        let version = self.read_u8()?;
        if version != FORMAT_VERSION {
            return Err(ManifestError::UnsupportedVersion(version));
        }
        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8, ManifestError> {
        Ok(self.take(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32, ManifestError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn read_u64(&mut self) -> Result<u64, ManifestError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn read_i64(&mut self) -> Result<i64, ManifestError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn read_hash(&mut self) -> Result<ChunkHash, ManifestError> {
        let bytes: [u8; HASH_LEN] = self.take(HASH_LEN)?.try_into().unwrap();
        Ok(ChunkHash(bytes))
    }

    fn read_string(&mut self) -> Result<String, ManifestError> {
        let len = self.read_u32()? as usize;
        let bytes = self.take(len)?;
        Ok(String::from_utf8(bytes.to_vec())?)
    }

    fn read_opt_string(&mut self) -> Result<Option<String>, ManifestError> {
        match self.read_u8()? {
            0 => Ok(None),
            _ => Ok(Some(self.read_string()?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;

    #[test]
    fn file_manifest_json_and_binary_roundtrip_and_lookup() {
        let manifest = FileManifest {
            size: 300_000,
            generation: 7,
            chunks: vec![
                ChunkRef {
                    offset: 0,
                    len: 100_000,
                    hash: hash_bytes(b"chunk-0"),
                },
                ChunkRef {
                    offset: 100_000,
                    len: 150_000,
                    hash: hash_bytes(b"chunk-1"),
                },
                ChunkRef {
                    offset: 250_000,
                    len: 50_000,
                    hash: hash_bytes(b"chunk-2"),
                },
            ],
        };

        let json = manifest.to_json_bytes().unwrap();
        assert_eq!(FileManifest::from_json_bytes(&json).unwrap(), manifest);

        let bin = manifest.to_binary_bytes();
        assert_eq!(FileManifest::from_binary_bytes(&bin).unwrap(), manifest);
        assert!(bin.len() < json.len());

        assert_eq!(manifest.chunk_at(0).unwrap().offset, 0);
        assert_eq!(manifest.chunk_at(100_000).unwrap().offset, 100_000);
        assert_eq!(manifest.chunk_at(299_999).unwrap().offset, 250_000);
        assert!(manifest.chunk_at(300_000).is_none());

        let span = manifest.chunks_in_range(99_999, 150_002);
        assert_eq!(span.len(), 3);
    }

    #[test]
    fn dir_manifest_pagination_and_serialization() {
        let entries: Vec<DirEntry> = (0..5)
            .map(|i| DirEntry {
                name: format!("file_{i}.txt"),
                kind: if i == 4 {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: (i as u64 + 1) * 1024,
                mode: 0o644,
                mtime_secs: 1_700_000_000 + i as i64,
                mtime_nanos: 123,
                manifest_hash: (i < 4).then(|| hash_bytes(format!("f{i}").as_bytes())),
            })
            .collect();

        let dir = DirManifest::paginate(entries, 2, 42);
        assert_eq!(dir.pages.len(), 3);

        let p0 = dir.page(None).unwrap();
        assert_eq!(p0.entries.len(), 2);
        assert_eq!(p0.next_cursor.as_deref(), Some("p1"));
        assert_eq!(p0.generation, 42);

        let p1 = dir.page(p0.next_cursor.as_deref()).unwrap();
        assert_eq!(p1.entries.len(), 2);
        assert_eq!(p1.next_cursor.as_deref(), Some("p2"));

        let p2 = dir.page(p1.next_cursor.as_deref()).unwrap();
        assert_eq!(p2.entries.len(), 1);
        assert_eq!(p2.next_cursor, None);

        // JSON & binary round-trips for both DirPage and DirManifest.
        let p0_bin = p0.to_binary_bytes();
        assert_eq!(&DirPage::from_binary_bytes(&p0_bin).unwrap(), p0);
        let p0_json = p0.to_json_bytes().unwrap();
        assert_eq!(&DirPage::from_json_bytes(&p0_json).unwrap(), p0);

        let dir_bin = dir.to_binary_bytes();
        assert_eq!(DirManifest::from_binary_bytes(&dir_bin).unwrap(), dir);
        let dir_json = dir.to_json_bytes().unwrap();
        assert_eq!(DirManifest::from_json_bytes(&dir_json).unwrap(), dir);
    }
}
