use core::{convert::TryInto, num::NonZeroU8};

use crate::{blockdevice::BlockIter, Block, BlockCount, BlockDevice, BlockIdx};

use self::bios_param_block::{BiosParameterBlock, BpbError};

pub mod bios_param_block;

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

    pub fn get_fat_entry(&mut self, fat_number: u32, cluster: Cluster) -> Result<Entry, BD::Error> {
        let fat_offset = match self.bpb.fat_type() {
            FatType::Fat16 => cluster.0 * 2,
            FatType::Fat32 => cluster.0 * 4,
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

    pub fn cluster_sectors(&self, cluster: &Cluster) -> BlockIter {
        self.bpb
            .data_start()
            .range(BlockCount(cluster.0 - 2) * self.bpb.sectors_per_cluster())
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

pub struct Cluster(pub u32);

pub struct ShortName {
    main_name: [u8; 8],
    extension: [u8; 3],
}

#[derive(Debug, Clone, Copy)]
pub enum DirEntryError {}

pub struct DirEntry<'a> {
    raw: DirEntryRaw<'a>,
    name: ShortName,
    attributes: Attributes,
}

impl<'a> DirEntry<'a> {
    pub fn new(raw: DirEntryRaw) -> Result<Self, DirEntryError> {
        let name = raw.name();
        let attr = Attributes::from_bits_truncate(raw.attr());

        let create_time_tenth = raw.crt_time_tenth();
        let create_time = raw.crt_time();
        let create_date = raw.crt_date();

        todo!()
    }
}

pub struct DirEntryRaw<'a> {
    block: &'a Block,
}

impl<'a> DirEntryRaw<'a> {
    fn data(&self) -> &[u8] {
        &self.block.contents
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
