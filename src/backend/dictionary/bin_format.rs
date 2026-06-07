//! dict.bin バイナリ形式の読み書き

use std::cmp::Ordering;
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use memmap2::Mmap;

use super::{AcceptedExactEntry, ParsedHeader, DICT_BIN_HEADER_LEN, DICT_BIN_MAGIC, DICT_BIN_TABLE_ENTRY_LEN, DICT_BIN_VERSION};

pub fn write_u32<W: Write>(writer: &mut W, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

pub fn write_u64<W: Write>(writer: &mut W, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

pub fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

pub fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let bytes: [u8; 8] = bytes.get(offset..end)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

pub fn checked_u32(value: usize, field: &'static str) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, field))
}

pub fn save_dict_bin(bin_file: &Path, entries: &[AcceptedExactEntry]) -> io::Result<usize> {
    if let Some(parent) = bin_file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut blob = Vec::new();
    let mut table = Vec::with_capacity(entries.len());
    for entry in entries {
        let key_offset = checked_u32(blob.len(), "key offset too large")?;
        blob.extend_from_slice(entry.source.as_bytes());
        let key_len = checked_u32(entry.source.len(), "key length too large")?;
        let value_offset = checked_u32(blob.len(), "value offset too large")?;
        blob.extend_from_slice(entry.value.as_bytes());
        let value_len = checked_u32(entry.value.len(), "value length too large")?;
        table.push((key_offset, key_len, value_offset, value_len));
    }

    let exact_table_offset = DICT_BIN_HEADER_LEN as u64;
    let string_blob_offset = exact_table_offset + (table.len() * DICT_BIN_TABLE_ENTRY_LEN) as u64;
    let tmp_file = bin_file.with_extension("bin.tmp");

    {
        let mut file = File::create(&tmp_file)?;
        file.write_all(DICT_BIN_MAGIC)?;
        write_u32(&mut file, DICT_BIN_VERSION)?;
        write_u32(
            &mut file,
            checked_u32(entries.len(), "exact count too large")?,
        )?;
        write_u64(&mut file, exact_table_offset)?;
        write_u64(&mut file, string_blob_offset)?;
        write_u64(&mut file, blob.len() as u64)?;

        for (key_offset, key_len, value_offset, value_len) in table {
            write_u32(&mut file, key_offset)?;
            write_u32(&mut file, key_len)?;
            write_u32(&mut file, value_offset)?;
            write_u32(&mut file, value_len)?;
        }
        file.write_all(&blob)?;
        file.sync_all()?;
    }

    if bin_file.exists() {
        std::fs::remove_file(bin_file)?;
    }
    std::fs::rename(&tmp_file, bin_file)?;
    Ok(blob.len())
}

pub fn parse_dict_bin_header(bytes: &[u8]) -> Option<ParsedHeader> {
    if bytes.len() < DICT_BIN_HEADER_LEN || bytes.get(0..8)? != DICT_BIN_MAGIC {
        return None;
    }
    let header = ParsedHeader {
        version: read_u32(bytes, 8)?,
        exact_count: read_u32(bytes, 12)?,
        exact_table_offset: read_u64(bytes, 16)?,
        string_blob_offset: read_u64(bytes, 24)?,
        string_blob_len: read_u64(bytes, 32)?,
    };
    if header.version != DICT_BIN_VERSION {
        return None;
    }

    let table_len = (header.exact_count as usize).checked_mul(DICT_BIN_TABLE_ENTRY_LEN)?;
    let table_end = (header.exact_table_offset as usize).checked_add(table_len)?;
    let blob_end =
        (header.string_blob_offset as usize).checked_add(header.string_blob_len as usize)?;
    if header.exact_table_offset as usize != DICT_BIN_HEADER_LEN
        || table_end > bytes.len()
        || blob_end > bytes.len()
        || table_end > header.string_blob_offset as usize
    {
        return None;
    }
    Some(header)
}

pub struct DictBinIndex {
    pub(crate) mmap: Mmap,
    pub(crate) header: ParsedHeader,
}

impl DictBinIndex {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let header = parse_dict_bin_header(&mmap)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid dict.bin v2"))?;
        Ok(Self { mmap, header })
    }

    fn table_entry(&self, index: usize) -> Option<(usize, usize, usize, usize)> {
        if index >= self.header.exact_count as usize {
            return None;
        }
        let offset = (self.header.exact_table_offset as usize)
            .checked_add(index.checked_mul(DICT_BIN_TABLE_ENTRY_LEN)?)?;
        Some((
            read_u32(&self.mmap, offset)? as usize,
            read_u32(&self.mmap, offset + 4)? as usize,
            read_u32(&self.mmap, offset + 8)? as usize,
            read_u32(&self.mmap, offset + 12)? as usize,
        ))
    }

    fn blob_slice(&self, offset: usize, len: usize) -> Option<&[u8]> {
        let blob_start = self.header.string_blob_offset as usize;
        let start = blob_start.checked_add(offset)?;
        let end = start.checked_add(len)?;
        let blob_end = blob_start.checked_add(self.header.string_blob_len as usize)?;
        if end > blob_end {
            return None;
        }
        self.mmap.get(start..end)
    }

    fn key_bytes(&self, index: usize) -> Option<&[u8]> {
        let (key_offset, key_len, _, _) = self.table_entry(index)?;
        self.blob_slice(key_offset, key_len)
    }

    pub fn value_string(&self, index: usize) -> Option<String> {
        let (_, _, value_offset, value_len) = self.table_entry(index)?;
        std::str::from_utf8(self.blob_slice(value_offset, value_len)?)
            .ok()
            .map(str::to_string)
    }

    pub fn lookup(&self, source: &str) -> Option<String> {
        let needle = source.as_bytes();
        let mut low = 0usize;
        let mut high = self.header.exact_count as usize;

        while low < high {
            let mid = low + (high - low) / 2;
            let key = self.key_bytes(mid)?;
            match key.cmp(needle) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid,
                Ordering::Equal => return self.value_string(mid),
            }
        }
        None
    }
}
