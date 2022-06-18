#![allow(missing_docs)]

mod boot_param_block;
mod fat16;
pub use boot_param_block::{BootParameterBlock, BpbError, FatType as BpbFatType};
use core::ops::RangeInclusive;

use fat16::FilesystemInfo as Fat16Info;

mod fat32;
use fat32::FilesystemInfo as Fat32Info;

use crate::{Block, BlockIdx};

pub struct File;

pub struct Sector;

pub struct Cluster(u32);

pub struct VolumeName([u8; 11]);

pub struct Volume {
    volume_name: VolumeName,
    info: FileSystem,
    blocks_per_cluster: u8,
    blocks: RangeInclusive<BlockIdx>,
    reserved_sectors: RangeInclusive<BlockIdx>,
    fat_region: RangeInclusive<BlockIdx>,
    root_directory_region: Option<RangeInclusive<BlockIdx>>,
    data_region_start: BlockIdx,
    next_free_cluster: Option<Cluster>,
    cluster_count: u32,
    free_cluster_count: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct FilenameError;

enum FileSystem {
    Fat16(Fat16Info),
    Fat32(Fat32Info),
}

impl FileSystem {
    const FOOTER_VALUE: u16 = 0xAA55;

    pub fn from_block(block: Block) -> Result<Self, ()> {
        todo!()
    }
}

struct DirEntry<'a> {
    data: &'a [u8],
}

impl<'a> DirEntry<'a> {
    pub(crate) const LEN: usize = 32;
    pub(crate) const LEN_U32: u32 = Self::LEN as u32;
}
