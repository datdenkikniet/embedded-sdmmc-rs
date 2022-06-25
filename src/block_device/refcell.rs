use crate::{Block, BlockDevice, BlockIdx};

use super::BlockCount;

impl<T> BlockDevice for core::cell::RefCell<T>
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
        let mut underlying = self.borrow_mut();
        underlying.read(blocks, start_block_idx, reason)
    }

    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let mut underlying = self.borrow_mut();
        underlying.write(blocks, start_block_idx)
    }

    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error> {
        let mut underlying = self.borrow_mut();
        underlying.num_blocks()
    }
}

impl<T> BlockDevice for &core::cell::RefCell<T>
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
        let mut underlying = self.borrow_mut();
        underlying.read(blocks, start_block_idx, reason)
    }

    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let mut underlying = self.borrow_mut();
        underlying.write(blocks, start_block_idx)
    }

    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error> {
        let mut underlying = self.borrow_mut();
        underlying.num_blocks()
    }
}
