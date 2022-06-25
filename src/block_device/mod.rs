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

    fn read_block(&mut self, block_idx: BlockIdx) -> Result<Block, Self::Error> {
        let mut blocks = [Block::new()];
        self.read(&mut blocks, block_idx, "")?;
        let [block] = blocks;
        Ok(block)
    }
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

#[derive(Debug)]
pub struct MemoryBlockDevice<'a> {
    memory: &'a mut [u8],
}

impl<'a> MemoryBlockDevice<'a> {
    pub fn new(memory: &'a mut [u8]) -> Self {
        Self { memory }
    }

    fn block_start(block_idx: usize) -> usize {
        block_idx * Block::LEN
    }

    fn block_end(block_idx: usize) -> usize {
        (block_idx * Block::LEN) + 512
    }
}

impl<'a> BlockDevice for MemoryBlockDevice<'a> {
    type Error = ();

    fn read(
        &mut self,
        blocks: &mut [Block],
        start_block_idx: BlockIdx,
        _reason: &str,
    ) -> Result<(), Self::Error> {
        for (idx, block) in blocks.iter_mut().enumerate() {
            let blk_start = Self::block_start(start_block_idx.0 as usize + idx);
            let blk_end = Self::block_end(start_block_idx.0 as usize + idx);
            block
                .contents
                .copy_from_slice(&self.memory[blk_start..blk_end])
        }

        Ok(())
    }

    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        for (idx, block) in blocks.iter().enumerate() {
            let blk_start = Self::block_start(start_block_idx.0 as usize + idx);
            let blk_end = Self::block_end(start_block_idx.0 as usize + idx);
            self.memory[blk_start..blk_end].copy_from_slice(&block.contents);
        }
        Ok(())
    }

    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error> {
        Ok(BlockCount((self.memory.len() / Block::LEN) as u32))
    }
}
