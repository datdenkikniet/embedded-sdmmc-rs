use core::{convert::TryInto, marker::PhantomData, num::NonZeroU8};

use crate::{BlockDevice, BlockIdx};

use self::{
    bios_param_block::{BiosParameterBlock, BpbError},
    block_byte_cache::BlockByteCache,
    cluster::{Cluster, ClusterIterator},
    directory::{DirEntry, DirIter},
    root_directory::RootDirIter,
};

pub mod bios_param_block;
pub mod block_byte_cache;
pub mod cluster;
pub mod directory;
pub mod root_directory;

pub trait SectorIter<BD>
where
    BD: BlockDevice,
{
    fn next(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx>;
}

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

    pub fn get_fat_entry(
        &mut self,
        fat_number: u32,
        cluster: &Cluster,
    ) -> Result<Entry, BD::Error> {
        let fat_offset = match self.bpb.fat_type() {
            FatType::Fat16 => cluster.0 * 2,
            FatType::Fat32 => cluster.0 * 4,
        };

        let sec_num = self.bpb.reserved_sector_count().0
            + (fat_offset / self.bpb.bytes_per_sector().get() as u32);
        let entry_sector = BlockIdx(sec_num);

        let entry_offset = (fat_offset % self.bpb.bytes_per_sector().get() as u32) as usize;

        let sector = if fat_number == 1 {
            entry_sector
        } else {
            BlockIdx((fat_number * self.bpb.fat_size().0) + entry_sector.0)
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

    pub fn find_next_cluster(
        &mut self,
        fat_number: u32,
        current_cluster: &Cluster,
    ) -> Result<Option<Cluster>, BD::Error> {
        let my_entry = self.get_fat_entry(fat_number, current_cluster)?;

        if my_entry.is_final(self.fat_type()) || my_entry.is_free(self.fat_type()) {
            Ok(None)
        } else {
            Ok(Some(Cluster(my_entry.0)))
        }
    }

    pub fn root_directory_iter<'a>(&'a mut self) -> DirIter<'a, BD, RootDirIter<BD>> {
        let root_sectors = self.bpb().root_sectors();
        let iter = root_sectors.iter(self);
        DirIter::new(self, iter)
    }

    pub fn bpb(&self) -> &BiosParameterBlock {
        &self.bpb
    }

    pub fn all_sectors(&mut self, cluster: Cluster) -> ClusterIterator {
        ClusterIterator::new(
            1,
            self.bpb.data_start(),
            cluster.clone(),
            self.bpb.sectors_per_cluster(),
        )
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

pub enum FileError<E> {
    DeviceError(E),
}

pub struct File<BD>
where
    BD: BlockDevice,
{
    dir_entry: DirEntry,
    _block_device: PhantomData<BD>,
}

impl<BD> core::fmt::Debug for File<BD>
where
    BD: BlockDevice,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File").finish()
    }
}

impl<BD> File<BD>
where
    BD: BlockDevice,
{
    pub fn new(dir_entry: DirEntry) -> Self {
        Self {
            dir_entry,
            _block_device: PhantomData {},
        }
    }

    pub fn read_all(
        &mut self,
        fat_volume: &mut FatVolume<BD>,
        data: &mut [u8],
    ) -> Result<usize, BD::Error> {
        let mut data = data;
        let mut read_bytes_total = 0;
        let mut sectors = fat_volume.all_sectors(self.dir_entry.first_cluster().clone());
        let mut read_cache = BlockByteCache::new(Some(self.dir_entry.file_size() as usize));

        while data.len() > 0 {
            if read_cache.all_cached_bytes_read() {
                if let Some(next_sector) = sectors.next(fat_volume) {
                    let block = fat_volume.block_device.read_block(next_sector).ok();
                    if let Some(block) = block {
                        read_cache.feed(block);
                    }
                } else {
                    break;
                }
            }

            let read_bytes = read_cache.read(data);
            data = &mut data[read_bytes..];
            read_bytes_total += read_bytes;
            if read_bytes == 0 {
                break;
            }
        }

        Ok(read_bytes_total)
    }
}
