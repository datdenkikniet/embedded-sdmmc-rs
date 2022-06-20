use core::{convert::TryInto, marker::PhantomData, num::NonZeroU8};

use crate::{blockdevice::BlockIter, Block, BlockCount, BlockDevice, BlockIdx};

use self::{
    bios_param_block::{BiosParameterBlock, BpbError},
    block_byte_cache::BlockByteCache,
};

pub mod bios_param_block;
mod block_byte_cache;

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
}

pub struct Timestamp {
    tenths: u8,
    double_second_count: u8,
    minutes: u8,
    hours: u8,
}

pub struct Date {
    day_of_month: NonZeroU8,
    month_of_year: NonZeroU8,
    year: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FatType {
    // Fat12
    Fat16,
    Fat32,
}

pub enum FatError<E> {
    DeviceError(E),
    BpbError(BpbError),
}

impl<E> From<BpbError> for FatError<E> {
    fn from(e: BpbError) -> Self {
        Self::BpbError(e)
    }
}

impl<E> core::fmt::Debug for FatError<E>
where
    E: core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DeviceError(arg0) => f.debug_tuple("DeviceError").field(arg0).finish(),
            Self::BpbError(arg0) => f.debug_tuple("BpbError").field(arg0).finish(),
        }
    }
}

pub struct FatVolume<BD>
where
    BD: BlockDevice,
{
    bpb: BiosParameterBlock,
    block_device: BD,
}

impl<BD> core::fmt::Debug for FatVolume<BD>
where
    BD: BlockDevice,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FatVolume").field("bpb", &self.bpb).finish()
    }
}

impl<BD> FatVolume<BD>
where
    BD: BlockDevice,
{
    pub fn new(mut block_device: BD) -> Result<Self, FatError<BD::Error>> {
        let bpb_block = block_device
            .read_block(BlockIdx(0))
            .map_err(|e| FatError::DeviceError(e))?;

        let bpb = BiosParameterBlock::new(bpb_block)?;

        Ok(Self { bpb, block_device })
    }

    pub fn fat_type(&self) -> FatType {
        self.bpb.fat_type()
    }

    pub fn cluster_for_sector(&self, sector: BlockIdx) -> Cluster {
        match self.bpb.fat_type() {
            FatType::Fat16 => Cluster(sector.0 / 2),
            FatType::Fat32 => Cluster(sector.0 / 4),
        }
    }

    pub fn get_fat_entry(&mut self, fat_number: u32, cluster: u32) -> Result<Entry, BD::Error> {
        let fat_offset = match self.bpb.fat_type() {
            FatType::Fat16 => cluster * 2,
            FatType::Fat32 => cluster * 4,
        };

        let sec_num = self.bpb.reserved_sector_count().0
            + (fat_offset / self.bpb.bytes_per_sector().get() as u32);
        let entry_sector = Sector(sec_num);

        let entry_offset = (fat_offset % self.bpb.bytes_per_sector().get() as u32) as usize;

        let sector = if fat_number == 1 {
            entry_sector
        } else {
            Sector((fat_number * self.bpb.fat_size().0) + entry_sector.0)
        };

        let sector = self.block_device.read_block(sector.into())?;

        let entry_value = match self.bpb.fat_type() {
            FatType::Fat16 => u16::from_le_bytes(
                sector.contents[entry_offset..entry_offset + 2]
                    .try_into()
                    .expect("Infallible"),
            ) as u32,
            FatType::Fat32 => {
                u32::from_le_bytes(
                    sector.contents[entry_offset..entry_offset + 4]
                        .try_into()
                        .expect("Infallible"),
                ) as u32
                    & 0x0FFFFFFF
            }
        };

        Ok(Entry(entry_value))
    }

    pub fn root_directory<'a>(&'a mut self) -> FileIter<'a, BD> {
        FileIter::new(self, self.bpb.root_start().range(self.bpb.root_len()))
    }
}

pub struct FileIter<'a, BD>
where
    BD: BlockDevice,
{
    volume: &'a mut FatVolume<BD>,
    sectors: BlockIter,
    block_cache: BlockByteCache,
    buffer: [u8; 32],
    total_entries_read: usize,
}

impl<'a, BD> core::fmt::Debug for FileIter<'a, BD>
where
    BD: BlockDevice,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileIter")
            .field("sectors", &self.sectors)
            .finish()
    }
}

impl<'a, BD> FileIter<'a, BD>
where
    BD: BlockDevice,
{
    pub fn new(volume: &'a mut FatVolume<BD>, sectors: BlockIter) -> Self {
        let sector_len = sectors.clone().count();
        let bps = volume.bpb.bytes_per_sector().get() as usize;
        Self {
            volume,
            sectors,
            block_cache: BlockByteCache::new(sector_len * bps),
            buffer: [0u8; 32],
            total_entries_read: 0,
        }
    }

    pub fn total_entries_read(&self) -> usize {
        self.total_entries_read
    }
}

impl<'a, BD> Iterator for FileIter<'a, BD>
where
    BD: BlockDevice,
{
    type Item = DirEntry;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.block_cache.all_cached_bytes_read() {
                if let Some(next_sector) = self.sectors.next() {
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
                if dir_entry.first_cluster.0 == 0 {
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry(u32);

impl Entry {
    const ALLOC_MIN: u32 = 2;

    pub const FREE: Self = Self(0);

    pub const FAT32_BAD: Self = Self(0xFFFFFF7);
    pub const FAT32_FINAL: Self = Self(0xFFFFFFFF);
    const FAT32_RESERVED_RANGE_START: u32 = 0xFFFFFF8;
    const FAT32_RESERVED_RANGE_END: u32 = 0xFFFFFFE;

    pub const FAT16_BAD: Self = Self(0xFFF7);
    pub const FAT16_FINAL: Self = Self(0xFFFF);
    const FAT16_RESERVED_RANGE_START: u32 = 0xFFF8;
    const FAT16_RESERVED_RANGE_END: u32 = 0xFFFE;

    pub fn new(value: u32) -> Option<Self> {
        if value > Self::ALLOC_MIN {
            Some(Self(value))
        } else {
            None
        }
    }

    pub fn is_free(&self, fat_type: FatType) -> bool {
        match fat_type {
            FatType::Fat16 => self == &Self::FREE,
            FatType::Fat32 => Entry(self.0 & 0xFFFFFFF) == Self::FREE,
        }
    }

    pub fn is_final(&self, fat_type: FatType) -> bool {
        match fat_type {
            FatType::Fat16 => {
                self == &Self::FAT16_FINAL
                    || (self.0 >= Self::FAT16_RESERVED_RANGE_START
                        && self.0 <= Self::FAT16_RESERVED_RANGE_END)
            }
            FatType::Fat32 => {
                Entry(self.0 & 0xFFFFFFF) == Self::FAT32_FINAL
                    || (self.0 >= Self::FAT32_RESERVED_RANGE_START
                        && self.0 <= Self::FAT32_RESERVED_RANGE_END)
            }
        }
    }
}

pub struct Sector(u32);

impl Into<BlockIdx> for Sector {
    fn into(self) -> BlockIdx {
        BlockIdx(self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cluster(u32);

impl Cluster {
    pub fn new(cluster_number: u32) -> Self {
        Self(cluster_number)
    }

    pub fn sectors(&self, bpb: &BiosParameterBlock) -> BlockIter {
        let start = BlockIdx(self.0);
        start.range(bpb.sectors_per_cluster())
    }

    pub fn find_next<BD>(
        &self,
        fat_number: u32,
        volume: &mut FatVolume<BD>,
    ) -> Result<Option<Cluster>, BD::Error>
    where
        BD: BlockDevice,
    {
        let my_entry = volume.get_fat_entry(fat_number, self.0)?;

        if my_entry.is_final(volume.fat_type()) || my_entry.is_free(volume.fat_type()) {
            Ok(None)
        } else {
            Ok(Some(Cluster(my_entry.0)))
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ShortName {
    main_name: [u8; 8],
    extension: [u8; 3],
}

impl ShortName {
    pub fn main_name(&self) -> &str {
        unsafe { core::str::from_utf8_unchecked(&self.main_name) }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DirEntryError {
    Fat16FistClusHiNotZero,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    name: ShortName,
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
            name: ShortName {
                main_name: name[0..8].try_into().expect("Infallible"),
                extension: name[8..11].try_into().expect("Infallible"),
            },
            attributes,
            file_size,
            first_cluster,
        })
    }

    pub fn name(&self) -> &ShortName {
        &self.name
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
    define_field!(fst_clus_lo, u16, 2);
    define_field!(file_size, u32, 28);
}

pub enum FileError<E> {
    DeviceError(E),
}

pub struct File<BD>
where
    BD: BlockDevice,
{
    fat_number: u32,
    first_cluster: Cluster,
    current_cluster: Cluster,
    current_sector: BlockIter,
    read_cache: BlockByteCache,
    _block_device: PhantomData<BD>,
}

impl<BD> core::fmt::Debug for File<BD>
where
    BD: BlockDevice,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File")
            .field("first_cluster", &self.first_cluster)
            .field("current_cluster", &self.current_cluster)
            .field("current_sector", &self.current_sector)
            .field("read_cache", &self.read_cache)
            .finish()
    }
}

impl<BD> File<BD>
where
    BD: BlockDevice,
{
    pub fn new(
        bios_param_block: &BiosParameterBlock,
        fat_number: u32,
        first_cluster: Cluster,
        file_len: usize,
    ) -> Self {
        Self {
            fat_number,
            first_cluster,
            current_cluster: first_cluster,
            current_sector: first_cluster.sectors(bios_param_block),
            read_cache: BlockByteCache::new(file_len),
            _block_device: PhantomData {},
        }
    }

    pub fn reset(&mut self, bios_param_block: &BiosParameterBlock) {
        self.current_cluster = self.first_cluster;
        self.current_sector = self.first_cluster.sectors(bios_param_block);
        self.read_cache.clear();
    }

    pub fn increment_current_cluster(
        &mut self,
        fat_volume: &mut FatVolume<BD>,
    ) -> Result<(), BD::Error> {
        let next_cluster = self
            .current_cluster
            .find_next(self.fat_number, fat_volume)?;
        if let Some(next_cluster) = next_cluster {
            self.current_cluster = next_cluster;
            Ok(())
        } else {
            Ok(())
        }
    }

    pub fn read_all(
        &mut self,
        fat_volume: &mut FatVolume<BD>,
        data: &mut [u8],
    ) -> Result<usize, BD::Error> {
        let mut data = data;
        let mut read_bytes_total = 0;

        while data.len() > 0 {
            if self.read_cache.all_cached_bytes_read() {
                let next_sector = if let Some(next_sector) = self.current_sector.next() {
                    Some(next_sector)
                } else {
                    self.increment_current_cluster(fat_volume)?;
                    self.current_sector.next()
                };

                if let Some(next_sector) = next_sector {
                    let block = fat_volume.block_device.read_block(next_sector)?;
                    self.read_cache.feed(block);
                }
            }

            let read_bytes = self.read_cache.read(data);
            data = &mut data[read_bytes..];
            read_bytes_total += read_bytes;
            if read_bytes == 0 {
                break;
            }
        }

        Ok(read_bytes_total)
    }
}
