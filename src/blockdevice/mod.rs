//! embedded-sdmmc-rs - Block Device support
//!
//! Generic code for handling block devices.

#[cfg(feature = "refcell-blockdevice")]
mod refcell;

mod block;
pub use block::*;

/// Represents a block device - a device which can read and write blocks (or
/// sectors). Only supports devices which are <= 2 TiB in size.
pub trait BlockDevice {
    /// The errors that the `BlockDevice` can return. Must be debug formattable.
    type Error: core::fmt::Debug;
    /// Read one or more blocks, starting at the given block index.
    fn read(
        &mut self,
        blocks: &mut [Block],
        start_block_idx: BlockIdx,
        reason: &str,
    ) -> Result<(), Self::Error>;
    /// Write one or more blocks, starting at the given block index.
    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error>;
    /// Determine how many blocks this device can hold.
    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error>;
}

impl<T> BlockDevice for &mut T
where
    T: BlockDevice,
{
    type Error = T::Error;

    fn read(
        &mut self,
        blocks: &mut [Block],
        start_block_idx: BlockIdx,
        reason: &str,
    ) -> Result<(), Self::Error> {
        (*self).read(blocks, start_block_idx, reason)
    }

    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        (*self).write(blocks, start_block_idx)
    }

    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error> {
        (*self).num_blocks()
    }
}
