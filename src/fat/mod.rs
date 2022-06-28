use core::convert::TryInto;

use crate::{BlockDevice, BlockIdx};

use self::{
    bios_param_block::{BiosParameterBlock, BpbError},
    cluster::{Cluster, ClusterSectorIterator},
    directory::{DirEntry, DirEntryInfo, DirEntryParent, DirEntryRaw, DirIter},
    file::{File, OpenMode},
    root_directory::RootDirIter,
};

pub mod bios_param_block;
pub mod block_byte_cache;
pub mod cluster;
pub mod directory;
pub mod file;
pub mod root_directory;

#[cfg(test)]
mod test;

#[derive(Debug, Clone, Copy)]
pub struct PhysicalLocation {
    /// The sector number in which this entry is stored
    sector: BlockIdx,
    /// The offset of the data described by this location within
    /// the sector
    byte_offset: usize,
}

impl PhysicalLocation {
    pub const fn zero() -> Self {
        Self {
            sector: BlockIdx(0),
            byte_offset: 0,
        }
    }
}

pub trait SectorIter<BD>
where
    BD: BlockDevice,
{
    fn next(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FatType {
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
    fd: usize,
    open_entries: [Option<(usize, DirEntryInfo)>; MAX_FILES],
}

impl<BD> FatVolume<BD>
where
    BD: BlockDevice,
{
    const EMPTY_FILE_HANDLE: Option<(usize, DirEntryInfo)> = None;

    pub fn new(mut block_device: BD) -> Result<Self, FatError<BD::Error>> {
        let bpb_block = block_device
            .read_block(BlockIdx(0))
            .map_err(FatError::DeviceError)?;

        let bpb = BiosParameterBlock::new(bpb_block)?;

        Ok(Self {
            bpb,
            block_device,
            open_entries: [Self::EMPTY_FILE_HANDLE; MAX_FILES],
            fd: 0,
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

    pub fn dir_entry_is_open(&self, dir_entry: &DirEntry<BD>) -> bool {
        self.open_entries.iter().any(|open_file_entry| {
            if let Some((_, open_entry)) = open_file_entry {
                open_entry == dir_entry.info()
            } else {
                false
            }
        })
    }

    pub fn file_is_open(&self, file: &File<BD>) -> bool {
        self.open_entries.iter().any(|open_file_entry| {
            if let Some((fd, _)) = open_file_entry {
                *fd == file.fd
            } else {
                false
            }
        })
    }

    pub fn open_file<'parent>(
        &mut self,
        dir_entry: DirEntry<'parent, BD>,
        mode: OpenMode,
    ) -> Result<File<'parent, BD>, DirEntry<'parent, BD>> {
        if self.dir_entry_is_open(&dir_entry) {
            return Err(dir_entry);
        }

        let fd = self.next_fd();
        let file = if let Some(file) = File::from_dir_entry(dir_entry.copy(), self, mode, fd) {
            file
        } else {
            return Err(dir_entry);
        };

        let empty_entry = self.open_entries.iter_mut().find(|f| f.is_none());

        if let Some(entry) = empty_entry {
            *entry = Some((fd, dir_entry.info().copy()));
            Ok(file)
        } else {
            Err(dir_entry)
        }
    }

    pub fn root_dir_iter<'me, 'func_duration>(
        &'me mut self,
    ) -> DirIter<'me, 'func_duration, BD, RootDirIter<BD>> {
        let root_sectors = self.bpb().root_sectors();
        let iter = root_sectors.iter(self);
        DirIter::new(self, DirEntryParent::RootDir, iter)
    }

    pub(crate) fn get_entry_location(&mut self, fat_number: u32, entry: u32) -> PhysicalLocation {
        let fat_offset = match self.fat_type() {
            FatType::Fat16 => entry * 2,
            FatType::Fat32 => entry * 4,
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

        PhysicalLocation {
            sector,
            byte_offset: entry_offset,
        }
    }

    pub(crate) fn read_fat_entry(
        &mut self,
        location: PhysicalLocation,
    ) -> Result<Entry, BD::Error> {
        let PhysicalLocation {
            sector,
            byte_offset: entry_offset,
        } = location;
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

        Ok(Entry::new_unchecked(entry_value, location))
    }

    pub(crate) fn find_next_entry(
        &mut self,
        fat_number: u32,
        current_entry: &Entry,
    ) -> Result<Option<Entry>, BD::Error> {
        if current_entry.is_final(self.fat_type()) || current_entry.is_free(self.fat_type()) {
            Ok(None)
        } else {
            let next_entry_location = self.get_entry_location(fat_number, current_entry.value);
            self.read_fat_entry(next_entry_location).map(|e| Some(e))
        }
    }

    pub(crate) fn find_next_cluster(
        &mut self,
        fat_number: u32,
        current_cluster: &Cluster,
    ) -> Result<Option<Cluster>, BD::Error> {
        let my_entry = current_cluster.entry();
        let next_entry = self.find_next_entry(fat_number, &my_entry)?;
        Ok(next_entry.map(|e| Cluster::new(e)))
    }

    pub(crate) fn all_sectors(&mut self, cluster: Cluster) -> ClusterSectorIterator {
        ClusterSectorIterator::new(
            1,
            self.bpb.data_start(),
            cluster,
            self.bpb.sectors_per_cluster(),
        )
    }

    pub fn close_dir_entry(&mut self, dir_entry: &DirEntry<BD>) {
        for f in self.open_entries.iter_mut() {
            if let Some((_, entry)) = f {
                if entry == dir_entry.info() {
                    f.take();
                }
            }
        }
    }

    pub(crate) fn deallocate_dir_entry(
        &mut self,
        dir_entry: DirEntry<BD>,
    ) -> Result<(), BD::Error> {
        self.close_dir_entry(&dir_entry);

        let fat_type = self.fat_type();
        let mut current_entry = dir_entry.info().first_cluster().entry().clone();

        loop {
            let mut block = self
                .block_device
                .read_block(current_entry.location.sector)?;

            let offset = current_entry.location.byte_offset;
            match fat_type {
                FatType::Fat16 => {
                    block.contents[offset..offset + 2]
                        .copy_from_slice(&(Entry::FREE as u16).to_le_bytes());
                }
                FatType::Fat32 => {
                    block.contents[offset..offset + 4]
                        .copy_from_slice(&(Entry::FREE as u32).to_le_bytes());
                }
            }

            self.block_device
                .write(&[block], current_entry.location.sector)?;

            current_entry =
                if let Some(val) = self.find_next_entry(dir_entry.fat_number(), &current_entry)? {
                    val
                } else {
                    break;
                }
        }

        let entry_location = dir_entry.location();

        let mut block = self.block_device.read_block(entry_location.sector)?;
        extern crate std;
        std::println!("{:X?}", block);
        let mut raw = DirEntryRaw::new(
            &mut block.contents[entry_location.byte_offset..entry_location.byte_offset + 32],
        );
        raw.clear_name();
        std::println!("{:X?}", block);
        self.block_device.write(&[block], entry_location.sector)?;
        Ok(())
    }

    fn next_fd(&mut self) -> usize {
        let value = self.fd;
        self.fd = self.fd.wrapping_add(1);
        value
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    value: u32,
    location: PhysicalLocation,
}

impl PartialEq<Entry> for Entry {
    fn eq(&self, other: &Entry) -> bool {
        self.value == other.value
    }
}

impl Entry {
    const ALLOC_MIN: u32 = 2;

    pub const FREE: u32 = 0;

    pub const FAT32_BAD: u32 = 0xFFFFFF7;
    pub const FAT32_FINAL: u32 = 0xFFFFFFFF;
    const FAT32_RESERVED_RANGE_START: u32 = 0xFFFFFF8;
    const FAT32_RESERVED_RANGE_END: u32 = 0xFFFFFFE;

    pub const FAT16_BAD: u32 = 0xFFF7;
    pub const FAT16_FINAL: u32 = 0xFFFF;
    const FAT16_RESERVED_RANGE_START: u32 = 0xFFF8;
    const FAT16_RESERVED_RANGE_END: u32 = 0xFFFE;

    const fn new_unchecked(value: u32, location: PhysicalLocation) -> Self {
        Self { value, location }
    }

    pub const fn new(value: u32, location: PhysicalLocation) -> Option<Self> {
        if value > Self::ALLOC_MIN {
            Some(Self { value, location })
        } else {
            None
        }
    }

    pub fn is_free(&self, fat_type: FatType) -> bool {
        match fat_type {
            FatType::Fat16 => self.value == Self::FREE,
            FatType::Fat32 => self.value & 0xFFFFFFF == Self::FREE,
        }
    }

    pub fn is_final(&self, fat_type: FatType) -> bool {
        match fat_type {
            FatType::Fat16 => {
                self.value == Self::FAT16_FINAL
                    || (self.value >= Self::FAT16_RESERVED_RANGE_START
                        && self.value <= Self::FAT16_RESERVED_RANGE_END)
            }
            FatType::Fat32 => {
                self.value & 0xFFFFFFF == Self::FAT32_FINAL
                    || (self.value >= Self::FAT32_RESERVED_RANGE_START
                        && self.value <= Self::FAT32_RESERVED_RANGE_END)
            }
        }
    }

    pub fn value(&self) -> u32 {
        self.value
    }
}
