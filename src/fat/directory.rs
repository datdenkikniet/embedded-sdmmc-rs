use core::convert::TryInto;

use crate::BlockDevice;

use super::{
    block_byte_cache::BlockByteCache, cluster::Cluster, Attributes, FatType, FatVolume, SectorIter,
};

#[derive(Debug, Clone, Copy)]
pub enum DirEntryError {
    Fat16FistClusHiNotZero,
}

#[derive(Debug, Clone, Copy)]
pub struct ShortNameRaw {
    main_name: [u8; 8],
    extension: [u8; 3],
}

impl ShortNameRaw {
    pub unsafe fn main_name_str(&self) -> &str {
        let mut name = &self.main_name[..];
        while name.len() > 0 && name[name.len() - 1] == 0x20 {
            name = &name[..name.len() - 1];
        }

        core::str::from_utf8_unchecked(name)
    }

    pub unsafe fn extension_str(&self) -> &str {
        let mut name = &self.extension[..];
        while name.len() > 0 && name[name.len() - 1] == 0x20 {
            name = &name[..name.len() - 1];
        }
        core::str::from_utf8_unchecked(name)
    }

    pub fn main_name(&self) -> &[u8; 8] {
        &self.main_name
    }

    pub fn extension(&self) -> &[u8; 3] {
        &self.extension
    }

    pub fn is_free(&self) -> bool {
        self.main_name[0] == 0x00 || self.main_name[0] == 0xE5 || self.main_name[0] == 0x05
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    name: ShortNameRaw,
    attributes: Attributes,
    file_size: u32,
    first_cluster: Cluster,
}

impl DirEntry {
    pub fn new(raw: &DirEntryRaw, fat_type: FatType) -> Result<Self, DirEntryError> {
        let name = raw.name();
        let attributes = Attributes::from_bits_truncate(raw.attr());

        let file_size = raw.file_size();

        let clus_hi = raw.fst_clus_hi() as u32;
        let clus_lo = raw.fst_clus_lo() as u32;
        let first_cluster = match fat_type {
            FatType::Fat16 => {
                if clus_hi == 0 {
                    Cluster(clus_lo)
                } else {
                    return Err(DirEntryError::Fat16FistClusHiNotZero);
                }
            }
            FatType::Fat32 => Cluster(clus_hi << 16 | clus_lo),
        };

        Ok(Self {
            name: ShortNameRaw {
                main_name: name[0..8].try_into().expect("Infallible"),
                extension: name[8..11].try_into().expect("Infallible"),
            },
            attributes,
            file_size,
            first_cluster,
        })
    }

    pub fn name(&self) -> &ShortNameRaw {
        &self.name
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn file_size(&self) -> u32 {
        self.file_size
    }

    pub fn first_cluster(&self) -> &Cluster {
        &self.first_cluster
    }

    pub(crate) fn get_free_status(&self) -> (bool, bool) {
        let first_name_char = self.name.main_name[0];
        (self.name.is_free(), first_name_char == 0x00)
    }
}

#[derive(Debug)]
pub struct DirEntryRaw<'a> {
    data: &'a [u8],
}

impl<'a> DirEntryRaw<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn name(&self) -> [u8; 11] {
        self.data()[0..11].try_into().expect("Infallible")
    }

    define_field!(attr, u8, 11);
    define_field!(crt_time_tenth, u8, 13);
    define_field!(crt_time, u16, 14);
    define_field!(crt_date, u16, 16);
    define_field!(lst_acc_date, u16, 18);
    define_field!(fst_clus_hi, u16, 20);
    define_field!(wrt_time, u16, 22);
    define_field!(wrt_date, u16, 24);
    define_field!(fst_clus_lo, u16, 26);
    define_field!(file_size, u32, 28);
}

pub struct DirIter<'a, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD>,
{
    volume: &'a mut FatVolume<BD>,
    sectors: Iter,
    block_cache: BlockByteCache,
    buffer: [u8; 32],
    total_entries_read: usize,
}

impl<'a, BD, Iter> core::fmt::Debug for DirIter<'a, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD> + core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileIter")
            .field("sectors", &self.sectors)
            .finish()
    }
}

impl<'a, BD, Iter> DirIter<'a, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD>,
{
    pub fn new(volume: &'a mut FatVolume<BD>, sectors: Iter) -> Self {
        Self {
            volume,
            sectors,
            block_cache: BlockByteCache::new(None),
            buffer: [0u8; 32],
            total_entries_read: 0,
        }
    }

    pub fn total_entries_read(&self) -> usize {
        self.total_entries_read
    }
}

impl<'a, BD, Iter> Iterator for DirIter<'a, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD>,
{
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.block_cache.all_cached_bytes_read() {
                if let Some(next_sector) = self.sectors.next(self.volume) {
                    let block = self.volume.block_device.read_block(next_sector).ok()?;
                    self.block_cache.feed(block);
                } else {
                    break None;
                }
            }

            let Self {
                block_cache,
                buffer,
                volume,
                ..
            } = self;

            if block_cache.read(buffer) == 32 {
                self.total_entries_read += 1;
                let raw_dir_entry = DirEntryRaw::new(&buffer[..]);
                let dir_entry = DirEntry::new(&raw_dir_entry, volume.bpb.fat_type()).ok()?;
                let (this_entry_free, all_following_free) = dir_entry.get_free_status();
                if all_following_free {
                    break None;
                } else if this_entry_free {
                    continue;
                } else {
                    return Some(dir_entry);
                }
            } else {
                return None;
            }
        }
    }
}
