use core::{convert::TryInto, marker::PhantomData};

use crate::{Block, BlockDevice};

use super::{
    block_byte_cache::BlockByteCache,
    cluster::{Cluster, ClusterSectorIterator},
    FatType, FatVolume, PhysicalLocation, SectorIter,
};

bitflags::bitflags! {
    pub struct Attributes: u8 {
        const READ_ONLY = (1 << 0);
        const HIDDEN = (1 << 1);
        const SYSTEM = (1 << 2);
        const VOLUME_ID = (1 << 3);
        const DIRECTORY = (1 << 4);
        const ARCHIVE = (1 << 5);
    }
}

impl Attributes {
    pub fn is_long_name(&self) -> bool {
        self.contains(Self::READ_ONLY | Self::HIDDEN | Self::SYSTEM | Self::VOLUME_ID)
    }

    pub fn is_dir(&self) -> bool {
        self.contains(Self::DIRECTORY)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DirEntryError {
    Fat16FistClusHiNotZero,
}

#[cfg(feature = "lfn")]
pub use lfn::LongNameRaw;

#[cfg(feature = "lfn")]
pub mod lfn {

    pub struct LongNameCharIter<'a> {
        idx: usize,
        len: usize,
        long_name: &'a LongNameRaw,
    }

    impl Iterator for LongNameCharIter<'_> {
        type Item = char;

        fn next(&mut self) -> Option<Self::Item> {
            let data = &self.long_name.long_name_data[..self.len];
            let value = data.get(self.idx)?;
            self.idx += 1;
            char::from_u32(*value as u32)
        }
    }

    #[derive(Debug, Clone)]
    pub struct LongNameRaw {
        pub(super) long_name_data: [u16; 256],
    }

    impl LongNameRaw {
        pub fn data(&self) -> &[u16] {
            &self.long_name_data[..]
        }

        pub fn chars(&self) -> LongNameCharIter {
            LongNameCharIter {
                idx: 0,
                len: self.len(),
                long_name: self,
            }
        }

        pub fn to_str<'a>(&self, data: &'a mut [u8]) -> Option<&'a str> {
            let mut char_idx = 0;
            for char in self.chars() {
                let len = char.len_utf8();
                if data.len() - char_idx > len {
                    char.encode_utf8(&mut data[char_idx..char_idx + len]);
                    char_idx += len;
                } else {
                    return None;
                }
            }
            // SAFTEY: all data until char_idx is validly encoded UTF-8
            Some(unsafe { core::str::from_utf8_unchecked(&data[..char_idx]) })
        }

        pub fn len(&self) -> usize {
            self.long_name_data
                .iter()
                .enumerate()
                .find_map(|(idx, v)| if *v == 0 { Some(idx) } else { None })
                .unwrap_or(self.long_name_data.len())
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ShortNameRaw {
    main_name: [u8; 8],
    extension: [u8; 3],
}

impl ShortNameRaw {
    // Main name, extension, implied '.'
    pub const MAX_STR_LEN: usize = 8 + 3 + 1;
    pub const STR: [u8; Self::MAX_STR_LEN] = [0u8; Self::MAX_STR_LEN];

    pub fn main_name_len(&self) -> usize {
        let mut len = 8;
        while len > 0 && self.main_name[len - 1] == 0x20 {
            len -= 1;
        }
        len
    }

    pub fn main_name_str(&self) -> Option<&str> {
        core::str::from_utf8(self.main_name()).ok()
    }

    pub fn extension_len(&self) -> usize {
        let mut len = 3;
        while len > 0 && self.extension[len - 1] == 0x20 {
            len -= 1;
        }
        len
    }

    pub fn extension_str(&self) -> Option<&str> {
        core::str::from_utf8(self.extension()).ok()
    }

    pub fn main_name(&self) -> &[u8] {
        &self.main_name[..self.main_name_len()]
    }

    pub fn extension(&self) -> &[u8] {
        &self.extension[..self.extension_len()]
    }

    pub fn to_str<'a>(&self, data: &'a mut [u8]) -> Option<&'a str> {
        let main_name = self.main_name_str()?.as_bytes();
        let main_name_len = main_name.len();

        let extension = self.extension_str()?.as_bytes();
        let extension_len = extension.len();

        data[..main_name.len()].copy_from_slice(main_name);
        let extension_len = if extension.len() > 0 {
            data[main_name_len] = b'.';
            data[main_name_len + 1..main_name_len + 1 + extension.len()].copy_from_slice(extension);
            1 + extension_len
        } else {
            0
        };

        // SAFETY: all bytes in data[..main_name_len + extension_len] are produced
        // by copying previously existing utf-8 data (and a single '.')
        unsafe {
            Some(core::str::from_utf8_unchecked(
                &data[..main_name_len + extension_len],
            ))
        }
    }

    pub fn is_free(&self) -> bool {
        self.main_name[0] == 0x00 || self.main_name[0] == 0xE5 || self.main_name[0] == 0x05
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DirEntryParent<'parent> {
    DirEntry(&'parent DirEntryInfo),
    RootDir,
}

#[derive(Debug)]
pub struct DirEntry<'parent, BD>
where
    BD: BlockDevice,
{
    parent: DirEntryParent<'parent>,
    info: DirEntryInfo,
    _block_device: PhantomData<FatVolume<BD>>,
}

impl<'parent, BD> DirEntry<'parent, BD>
where
    BD: BlockDevice,
{
    fn new(parent: DirEntryParent<'parent>, info: DirEntryInfo) -> Self {
        Self {
            parent,
            info,
            _block_device: Default::default(),
        }
    }

    pub(crate) fn copy(&self) -> Self {
        Self {
            parent: self.parent,
            info: self.info.copy(),
            _block_device: Default::default(),
        }
    }

    pub fn info(&self) -> &DirEntryInfo {
        &self.info
    }

    pub fn iter_subdir<'a, 'me>(
        &'me self,
        volume: &'a mut FatVolume<BD>,
    ) -> Option<DirIter<'a, 'me, BD, ClusterSectorIterator>>
    where
        BD: BlockDevice,
    {
        if self.info.is_dir() {
            let cluster_iterator = volume.all_sectors(self.info.first_cluster);
            Some(DirIter::new(
                volume,
                DirEntryParent::DirEntry(&self.info),
                cluster_iterator,
            ))
        } else {
            None
        }
    }

    pub fn delete(self, volume: &mut FatVolume<BD>) -> Result<(), Self> {
        let mut block = if let Ok(block) = volume.block_device.read_block(self.info.location.sector)
        {
            block
        } else {
            return Err(self);
        };

        let entry_start = self.info.location.byte_offset;
        let entry_end = self.info.location.byte_offset + 32;
        let mut raw = DirEntryRaw::new(&mut block.contents[entry_start..entry_end]);
        raw.set_name(&[0x5E]);

        if volume
            .block_device
            .write(&[block], self.info.location.sector)
            .is_err()
        {
            Err(self)
        } else {
            Ok(())
        }
    }
}

impl<BD> core::ops::Deref for DirEntry<'_, BD>
where
    BD: BlockDevice,
{
    type Target = DirEntryInfo;

    fn deref(&self) -> &Self::Target {
        self.info()
    }
}

#[derive(Debug)]
pub struct DirEntryInfo {
    name: ShortNameRaw,
    attributes: Attributes,
    file_size: u32,
    first_cluster: Cluster,
    location: PhysicalLocation,
    #[cfg(feature = "lfn")]
    long_name: LongNameRaw,
}

impl PartialEq<DirEntryInfo> for DirEntryInfo {
    fn eq(&self, other: &DirEntryInfo) -> bool {
        self.first_cluster == other.first_cluster
    }
}

impl DirEntryInfo {
    pub(crate) fn copy(&self) -> Self {
        Self {
            location: self.location.clone(),
            name: self.name.clone(),
            #[cfg(feature = "lfn")]
            long_name: self.long_name.clone(),
            attributes: self.attributes,
            file_size: self.file_size,
            first_cluster: self.first_cluster,
        }
    }

    fn new(
        #[cfg(feature = "lfn")] long_name: LongNameRaw,
        raw: &DirEntryRaw,
        fat_type: FatType,
        location: PhysicalLocation,
    ) -> Result<Self, DirEntryError> {
        let name = raw.name();
        let attributes = Attributes::from_bits_truncate(raw.attr());

        let short_name = ShortNameRaw {
            main_name: name[0..8].try_into().expect("Infallible"),
            extension: name[8..11].try_into().expect("Infallible"),
        };

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
            location,
            name: short_name,
            #[cfg(feature = "lfn")]
            long_name,
            attributes,
            file_size,
            first_cluster,
        })
    }

    pub fn short_name(&self) -> &ShortNameRaw {
        &self.name
    }

    pub fn attributes(&self) -> &Attributes {
        &self.attributes
    }

    pub fn is_dir(&self) -> bool {
        self.attributes().is_dir()
    }

    pub fn file_size(&self) -> u32 {
        self.file_size
    }

    pub fn first_cluster(&self) -> &Cluster {
        &self.first_cluster
    }

    pub(crate) fn get_free_status(&self) -> (bool, bool) {
        let first_name_char = self.short_name().main_name[0];
        (self.short_name().is_free(), first_name_char == 0x00)
    }

    #[cfg(feature = "lfn")]
    pub fn long_name(&self) -> &LongNameRaw {
        &self.long_name
    }
}

#[derive(Debug)]
pub struct DirEntryRaw<'a> {
    data: &'a mut [u8],
}

impl<'a> DirEntryRaw<'a> {
    pub const LFN_ENTRY_LEN: usize = (5 + 6 + 2);

    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    fn data(&self) -> &[u8] {
        self.data
    }

    fn data_mut(&mut self) -> &mut [u8] {
        self.data
    }

    pub fn name(&self) -> [u8; 11] {
        self.data()[0..11].try_into().expect("Infallible")
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let len = name.len().min(11);
        self.data[0..len].copy_from_slice(&name[..len]);
        for idx in len..11 {
            self.data[idx] = 0x20;
        }
    }

    define_field!(attr, set_attr, u8, 11);
    define_field!(crt_time_tenth, set_crt_time_tenth, u8, 13);
    define_field!(crt_time, set_crt_time, u16, 14);
    define_field!(crt_date, set_crt_date, u16, 16);
    define_field!(lst_acc_date, set_lst_acc_date, u16, 18);
    define_field!(fst_clus_hi, set_fst_clus_hi, u16, 20);
    define_field!(wrt_time, set_wrt_time, u16, 22);
    define_field!(wrt_date, set_wrt_date, u16, 24);
    define_field!(fst_clus_lo, set_fst_clus_lo, u16, 26);
    define_field!(file_size, set_file_size, u32, 28);

    pub fn is_long_name(&self) -> bool {
        Attributes::from_bits_truncate(self.attr()).is_long_name()
    }

    // The following items are only valid for LFN entries
    define_field!(ldir_ord, set_ldir_ord, u8, 0);
    pub fn ldir_name1(&self) -> [u16; 5] {
        let mut data = [0u16; 5];
        for idx in 0..5 {
            data[idx] = u16::from_le_bytes(
                self.data[1 + (idx * 2)..1 + ((idx + 1) * 2)]
                    .try_into()
                    .expect("Infallible"),
            );
        }
        data
    }
    define_field!(ldir_type, set_ldir_type, u8, 12);
    define_field!(ldir_chksum, set_ldir_chksum, u8, 13);
    pub fn ldir_name2(&self) -> [u16; 6] {
        let mut data = [0u16; 6];
        for idx in 0..6 {
            data[idx] = u16::from_le_bytes(
                self.data[14 + (idx * 2)..14 + ((idx + 1) * 2)]
                    .try_into()
                    .expect("Infallible"),
            );
        }
        data
    }
    define_field!(ldir_fst_clus_lo, set_ldir_fst_clus_lo, u16, 26);
    pub fn ldir_name3(&self) -> [u16; 2] {
        let mut data = [0u16; 2];
        for idx in 0..2 {
            data[idx] = u16::from_le_bytes(
                self.data[28 + (idx * 2)..28 + ((idx + 1) * 2)]
                    .try_into()
                    .expect("Infallible"),
            );
        }
        data
    }
}

pub struct DirIter<'volume, 'parent, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD>,
{
    volume: &'volume mut FatVolume<BD>,
    directory: DirEntryParent<'parent>,
    block_cache: BlockByteCache<BD, Iter>,
    buffer: [u8; 32],
    total_entries_read: usize,
}

impl<BD, Iter> core::fmt::Debug for DirIter<'_, '_, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD> + core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DirIter")
            .field("total_entries_read", &self.total_entries_read)
            .finish()
    }
}

impl<'a, 'parent, BD, Iter> DirIter<'a, 'parent, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD>,
{
    pub fn new(
        volume: &'a mut FatVolume<BD>,
        directory: DirEntryParent<'parent>,
        sectors: Iter,
    ) -> Self {
        Self {
            directory,
            volume,
            block_cache: BlockByteCache::new(None, sectors),
            buffer: [0u8; 32],
            total_entries_read: 0,
        }
    }

    pub fn total_entries_read(&self) -> usize {
        self.total_entries_read
    }

    #[cfg(feature = "lfn")]
    fn handle_lfn(raw_dir_entry: &DirEntryRaw, long_name_data: &mut Option<LongNameRaw>) {
        // TODO: handle "bad" long name entries
        let mut ord = raw_dir_entry.ldir_ord();
        let is_last_entry = (ord & 0x40) == 0x40;
        ord &= !0x40;
        ord = ord.wrapping_sub(1);

        if is_last_entry {
            *long_name_data = Some(LongNameRaw {
                long_name_data: [0u16; 256],
            });
        }

        if let Some(long_entry_data) = long_name_data {
            let d1 = raw_dir_entry.ldir_name1();
            let d2 = raw_dir_entry.ldir_name2();
            let d3 = raw_dir_entry.ldir_name3();

            let start_index = DirEntryRaw::LFN_ENTRY_LEN * ord as usize;
            let end_index = start_index + DirEntryRaw::LFN_ENTRY_LEN;

            if end_index <= 255 {
                long_entry_data.long_name_data[start_index..start_index + 5].copy_from_slice(&d1);
                long_entry_data.long_name_data[start_index + 5..start_index + (5 + 6)]
                    .copy_from_slice(&d2);
                long_entry_data.long_name_data[start_index + (5 + 6)..start_index + (5 + 6 + 2)]
                    .copy_from_slice(&d3);
            }
        }
    }
}

impl<'parent, BD, Iter> Iterator for DirIter<'_, 'parent, BD, Iter>
where
    BD: BlockDevice,
    Iter: SectorIter<BD>,
{
    type Item = DirEntry<'parent, BD>;

    fn next(&mut self) -> Option<Self::Item> {
        let Self {
            block_cache,
            buffer,
            volume,
            total_entries_read,
            ..
        } = self;

        #[cfg(feature = "lfn")]
        let mut long_name_data = None;

        loop {
            if !block_cache.more_data(volume) {
                break None;
            }

            if let (32, Some(current_sector)) = block_cache.read(buffer) {
                *total_entries_read += 1;
                let raw_dir_entry = DirEntryRaw::new(&mut buffer[..]);

                if raw_dir_entry.is_long_name() {
                    #[cfg(feature = "lfn")]
                    Self::handle_lfn(&raw_dir_entry, &mut long_name_data);
                } else {
                    let dir_entry = DirEntryInfo::new(
                        #[cfg(feature = "lfn")]
                        long_name_data.take().unwrap_or(LongNameRaw {
                            long_name_data: [0u16; 256],
                        }),
                        &raw_dir_entry,
                        volume.bpb.fat_type(),
                        PhysicalLocation {
                            sector: current_sector,
                            byte_offset: (*total_entries_read * 32) % (Block::LEN),
                        },
                    )
                    .ok()?;
                    let (this_entry_free, all_following_free) = dir_entry.get_free_status();
                    if all_following_free {
                        break None;
                    } else if this_entry_free {
                        continue;
                    } else {
                        return Some(DirEntry::new(self.directory, dir_entry));
                    }
                }
            } else {
                return None;
            }
        }
    }
}
