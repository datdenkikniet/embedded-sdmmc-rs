#![allow(missing_docs)]

use crate::{Block, BlockCount, BlockDevice, BlockIdx};
use core::{convert::TryInto, fmt::Debug};

#[derive(Debug)]
pub enum Error<BlockDeviceError>
where
    BlockDeviceError: Debug,
{
    DeviceError(BlockDeviceError),
    InvalidMbrSignature,
    InvalidPartitionStatus,
    UnsupportedPartitionType(u8),
    InfoTooShort,
}

impl<BDE> From<BDE> for Error<BDE>
where
    BDE: Debug,
{
    fn from(e: BDE) -> Self {
        Self::DeviceError(e)
    }
}

#[derive(Debug, PartialEq)]
pub enum VolumeType {
    Fat,
}

pub enum PartitionNumber {
    One,
    Two,
    Three,
    Four,
}

impl PartitionNumber {
    pub fn from_number(number: usize) -> Option<Self> {
        let partition = match number {
            1 => Self::One,
            2 => Self::One,
            3 => Self::Two,
            4 => Self::Three,
            _ => return None,
        };
        Some(partition)
    }
}

pub enum PartitionType {
    Fat32ChsLba,
    Fat32Lba,
    Fat16Lba,
    Fat16,
    Unknown(u8),
}

impl PartitionType {
    /// Marker for a FAT32 partition. What Macosx disk utility (and also SD-Card formatter?)
    /// use.
    const FAT32_CHS_LBA: u8 = 0x0B;
    /// Marker for a FAT32 partition. Sometimes also use for FAT16 formatted
    /// partitions.
    const FAT32_LBA: u8 = 0x0C;
    /// Marker for a FAT16 partition with LBA. Seen on a Raspberry Pi SD card.
    const FAT16_LBA: u8 = 0x0E;
    /// Marker for a FAT16 partition. Seen on a card formatted with the official
    /// SD-Card formatter.
    const FAT16: u8 = 0x06;

    pub fn from_u8(value: u8) -> Self {
        match value {
            Self::FAT32_CHS_LBA => Self::Fat32ChsLba,
            Self::FAT32_LBA => Self::Fat32Lba,
            Self::FAT16_LBA => Self::Fat16Lba,
            Self::FAT16 => Self::Fat16,
            _ => Self::Unknown(value),
        }
    }
}

#[derive(Debug)]
pub enum PartitionError<E>
where
    E: Debug,
{
    DeviceError(E),
    OutOfRange { partition_block_count: BlockCount },
}

pub struct PartitionBlockDevice<'bd, 'part, BD>
where
    BD: BlockDevice,
{
    block_device: &'bd mut BD,
    partition: &'part Partition,
}

impl<'bd, 'part, BD> PartitionBlockDevice<'bd, 'part, BD>
where
    BD: BlockDevice,
{
    fn range_check<E>(&self, start: u32, len: u32) -> Result<(), PartitionError<E>>
    where
        E: Debug,
    {
        let last_block = start + len;

        if last_block > self.partition.block_count.0 {
            return Err(PartitionError::OutOfRange {
                partition_block_count: self.partition.block_count,
            });
        } else {
            Ok(())
        }
    }
}

impl<'bd, 'part, BD> BlockDevice for PartitionBlockDevice<'bd, 'part, BD>
where
    BD: BlockDevice,
{
    type Error = PartitionError<BD::Error>;

    fn read(
        &mut self,
        blocks: &mut [Block],
        start_block_idx: BlockIdx,
        reason: &str,
    ) -> Result<(), Self::Error> {
        let blocks_to_read = blocks.len() as u32;
        self.range_check(start_block_idx.0, blocks_to_read)?;

        let part_start_block_idx = start_block_idx + self.partition.lba_start;

        self.block_device
            .read(blocks, part_start_block_idx, reason)
            .map_err(|e| PartitionError::DeviceError(e))
    }

    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let blocks_to_write = blocks.len() as u32;
        self.range_check(start_block_idx.0, blocks_to_write)?;

        let part_start_block_idx = start_block_idx + self.partition.lba_start;

        self.block_device
            .write(blocks, part_start_block_idx)
            .map_err(|e| PartitionError::DeviceError(e))
    }

    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error> {
        Ok(self.partition.block_count)
    }
}

pub struct Partition {
    pub ty: PartitionType,
    pub lba_start: BlockCount,
    pub block_count: BlockCount,
}

impl Partition {
    const STATUS_IDX: usize = 0;
    const TYPE_IDX: usize = 4;
    const LBA_START_IDX: usize = 8;
    const NUM_BLOCKS_IDX: usize = 12;
    pub(crate) const PARTITION_INFO_LENGTH: usize = 16;

    pub fn from_info<E>(info: &[u8]) -> Result<Self, Error<E>>
    where
        E: Debug,
    {
        if info.len() != Self::PARTITION_INFO_LENGTH {
            return Err(Error::InfoTooShort);
        }

        let pstatus = info[Self::STATUS_IDX];
        if pstatus != 0x80 && pstatus != 0x00 {
            return Err(Error::InvalidPartitionStatus);
        }

        let lba_start = u32::from_le_bytes(
            info[Self::LBA_START_IDX..Self::LBA_START_IDX + 4]
                .try_into()
                .expect("Infallible"),
        );

        let num_blocks = u32::from_le_bytes(
            info[Self::NUM_BLOCKS_IDX..Self::NUM_BLOCKS_IDX + 4]
                .try_into()
                .expect("Infallible"),
        );

        let partition_type = PartitionType::from_u8(info[Self::TYPE_IDX]);

        Ok(Partition {
            ty: partition_type,
            lba_start: BlockCount(lba_start),
            block_count: BlockCount(num_blocks),
        })
    }

    pub fn with_block_device<'bd, 'part, BD>(
        &'part self,
        block_device: &'bd mut BD,
    ) -> PartitionBlockDevice<'bd, 'part, BD>
    where
        BD: BlockDevice,
    {
        PartitionBlockDevice {
            block_device,
            partition: self,
        }
    }
}

pub struct Mbr;

impl Mbr {
    const FOOTER_START: usize = 510;
    const FOOTER_VALUE: u16 = 0xAA55;
    const PARTITION1_START: usize = 446;
    const PARTITION2_START: usize = Self::PARTITION1_START + Partition::PARTITION_INFO_LENGTH;
    const PARTITION3_START: usize = Self::PARTITION2_START + Partition::PARTITION_INFO_LENGTH;
    const PARTITION4_START: usize = Self::PARTITION3_START + Partition::PARTITION_INFO_LENGTH;

    pub fn read_partition<BlockDev>(
        block_dev: &mut BlockDev,
        partition_num: PartitionNumber,
    ) -> Result<Partition, Error<BlockDev::Error>>
    where
        BlockDev: BlockDevice,
    {
        let mut blocks = [Block::new()];
        block_dev.read(&mut blocks, BlockIdx(0), "read_mbr")?;
        let [first_block] = blocks;

        let footer = u16::from_le_bytes(
            first_block[Self::FOOTER_START..Self::FOOTER_START + 2]
                .try_into()
                .expect("Infallible"),
        );

        if footer != Self::FOOTER_VALUE {
            return Err(Error::InvalidMbrSignature);
        }

        let pinfo_start = match partition_num {
            PartitionNumber::One => Self::PARTITION1_START,
            PartitionNumber::Two => Self::PARTITION2_START,
            PartitionNumber::Three => Self::PARTITION3_START,
            PartitionNumber::Four => Self::PARTITION4_START,
        };

        let pinfo_data = &first_block[pinfo_start..pinfo_start + Partition::PARTITION_INFO_LENGTH];
        Partition::from_info(pinfo_data)
    }
}
