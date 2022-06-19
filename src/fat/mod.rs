use core::convert::TryInto;

use crate::{Block, BlockDevice, BlockIdx};

use self::bios_param_block::BiosParameterBlock;

pub mod bios_param_block;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FatType {
    // Fat12
    Fat16,
    Fat32,
}

pub struct FatVolume<BD>
where
    BD: BlockDevice,
{
    bpb: BiosParameterBlock,
    block_device: BD,
}

impl<BD> FatVolume<BD>
where
    BD: BlockDevice,
{
    fn read_block(&mut self, idx: BlockIdx) -> Result<Block, BD::Error> {
        let mut blocks = [Block::new()];
        self.block_device.read(&mut blocks, idx, "")?;
        let [block] = blocks;
        Ok(block)
    }

    pub fn get_fat_entry(&mut self, fat_number: u32, cluster: Cluster) -> Result<Entry, BD::Error> {
        let fat_offset = match self.bpb.fat_type() {
            FatType::Fat16 => cluster.0 * 2,
            FatType::Fat32 => cluster.0 * 4,
        };

        let sec_num = self.bpb.reserved_sector_count().get() as u32
            + (fat_offset / self.bpb.bytes_per_sector().get() as u32);
        let entry_sector = Sector(sec_num);

        let entry_offset = (fat_offset % self.bpb.bytes_per_sector().get() as u32) as usize;

        let sector = if fat_number == 1 {
            entry_sector
        } else {
            Sector((fat_number * self.bpb.fat_size()) + entry_sector.0)
        };

        let sector = self.read_block(sector.into())?;

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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry(u32);

impl Entry {
    const ALLOC_MIN: u32 = 2;

    pub const FREE: Self = Self(0);

    pub const FAT32_BAD: Self = Self(0xFFFFFF7);
    pub const FAT32_FINAL: Self = Self(0xFFFFFFFF);
    const F32_RESERVED_MAX: u32 = 0xFFFFFF6;
    const F32_RESERVED_RANGE_START: u32 = 0xFFFFFF8;
    const F32_RESERVED_RANGE_END: u32 = 0xFFFFFFE;

    pub const FAT16_BAD: Self = Self(0xFFF7);
    pub const FAT16_FINAL: Self = Self(0xFFFF);
    const F16_RESERVED_MAX: u32 = 0xFFF6;
    const F16_RESERVED_RANGE_START: u32 = 0xFFF8;
    const F16_RESERVED_RANGE_END: u32 = 0xFFFE;

    pub fn new(value: u32, _fat_type: FatType) -> Self {
        Self(value)
    }

    pub fn is_free(&self, fat_type: FatType) -> bool {
        match fat_type {
            FatType::Fat16 => self == &Self::FREE,
            FatType::Fat32 => Entry(self.0 & 0xFFFFFFF) == Self::FREE,
        }
    }
}

pub struct Sector(u32);

impl Into<BlockIdx> for Sector {
    fn into(self) -> BlockIdx {
        BlockIdx(self.0)
    }
}

pub struct Cluster(u32);
