use core::convert::TryInto;

use crate::{BlockDevice, BlockIdx};

use self::{
    bios_param_block::{BiosParameterBlock, BpbError},
    cluster::{Cluster, ClusterSectorIterator},
    directory::{DirEntry, DirIter},
    file::{File, OpenMode},
    root_directory::RootDirIter,
};

pub mod bios_param_block;
pub mod block_byte_cache;
pub mod cluster;
pub mod directory;
pub mod file;
pub mod root_directory;

pub trait SectorIter<BD>
where
    BD: BlockDevice,
{
    fn next(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx>;
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

const MAX_FILES: usize = 8;

pub struct FatVolume<BD>
where
    BD: BlockDevice,
{
    bpb: BiosParameterBlock,
    block_device: BD,
    open_entries: [Option<DirEntry<BD>>; MAX_FILES],
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
    const EMPTY_FILE_HANDLE: Option<DirEntry<BD>> = None;

    pub fn new(mut block_device: BD) -> Result<Self, FatError<BD::Error>> {
        let bpb_block = block_device
            .read_block(BlockIdx(0))
            .map_err(FatError::DeviceError)?;

        let bpb = BiosParameterBlock::new(bpb_block)?;

        Ok(Self {
            bpb,
            block_device,
            open_entries: [Self::EMPTY_FILE_HANDLE; MAX_FILES],
        })
    }

    pub fn release(self) -> BD {
        self.block_device
    }

    pub fn bpb(&self) -> &BiosParameterBlock {
        &self.bpb
    }

    pub fn fat_type(&self) -> FatType {
        self.bpb.fat_type()
    }

    pub fn open_file(&mut self, dir_entry: DirEntry<BD>) -> Result<File<BD>, ()> {
        let entry_is_open = self.open_entries.iter().any(|open_file_entry| {
            if let Some(open_entry) = open_file_entry {
                open_entry == &dir_entry
            } else {
                false
            }
        });

        if entry_is_open {
            return Err(());
        }

        let file =
            if let Some(file) = File::open_dir_entry(dir_entry.clone(), self, OpenMode::ReadOnly) {
                file
            } else {
                return Err(());
            };

        let empty_entry = self.open_entries.iter_mut().find(|f| f.is_none());

        if let Some(entry) = empty_entry {
            *entry = Some(dir_entry);
            Ok(file)
        } else {
            Err(())
        }
    }

    pub fn root_directory_iter(&mut self) -> DirIter<BD, RootDirIter<BD>> {
        let root_sectors = self.bpb().root_sectors();
        let iter = root_sectors.iter(self);
        DirIter::new(self, iter)
    }

    pub(crate) fn get_fat_entry(
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

        let sector = self.block_device.read_block(sector)?;

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

    pub(crate) fn find_next_cluster(
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

    pub(crate) fn all_sectors(&mut self, cluster: Cluster) -> ClusterSectorIterator {
        ClusterSectorIterator::new(
            1,
            self.bpb.data_start(),
            cluster,
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
