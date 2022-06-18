use crate::Block;

use super::DirEntry;

pub enum FatType {
    Fat16,
    Fat32,
}

pub enum BpbError {
    InvalidFooterValue(u16),
    Fat12Unsupported,
}

pub struct BootParameterBlock<'a> {
    block: &'a Block,
    fat_type: FatType,
    cluster_count: u32,
}

impl<'a> BootParameterBlock<'a> {
    const FOOTER_VALUE: u16 = 0xAA55;

    pub fn from_block(block: &'a Block) -> Result<Self, BpbError> {
        let mut bpb = Self {
            block,
            fat_type: FatType::Fat16,
            cluster_count: 0,
        };

        if bpb.footer() != Self::FOOTER_VALUE {
            return Err(BpbError::InvalidFooterValue(bpb.footer()));
        }
        // FirstDataSector = BPB_ResvdSecCnt + (BPB_NumFATs * FATSz) + RootDirSectors;
        let root_dir_blocks = ((bpb.root_entries_count() as u32 * DirEntry::LEN_U32)
            + (Block::LEN_U32 - 1))
            / Block::LEN_U32;

        let fats_len = bpb.num_fats() as u32 * bpb.fat_size();
        let data_blocks =
            bpb.total_blocks() - (bpb.reserved_block_count() as u32 + fats_len + root_dir_blocks);

        let cluster_count = data_blocks / bpb.blocks_per_cluster() as u32;
        let fat_type = if cluster_count < 4085 {
            return Err(BpbError::Fat12Unsupported);
        } else if cluster_count < 65525 {
            FatType::Fat16
        } else {
            FatType::Fat32
        };

        bpb.fat_type = fat_type;
        bpb.cluster_count = cluster_count;

        Ok(bpb)
    }

    pub fn data(&self) -> &[u8; 512] {
        &self.block.contents
    }

    fn fat_size(&self) -> u32 {
        let result = self.fat_size16() as u32;
        if result == 0 {
            self.fat_size32()
        } else {
            result
        }
    }

    fn total_blocks(&self) -> u32 {
        let result = self.total_blocks16() as u32;
        if result == 0 {
            self.total_blocks32()
        } else {
            result
        }
    }

    // FAT16/FAT32
    define_field!(bytes_per_block, u16, 11);
    define_field!(blocks_per_cluster, u8, 13);
    define_field!(reserved_block_count, u16, 14);
    define_field!(num_fats, u8, 16);
    define_field!(root_entries_count, u16, 17);
    define_field!(total_blocks16, u16, 19);
    define_field!(media, u8, 21);
    define_field!(fat_size16, u16, 22);
    define_field!(blocks_per_track, u16, 24);
    define_field!(num_heads, u16, 26);
    define_field!(hidden_blocks, u32, 28);
    define_field!(total_blocks32, u32, 32);
    define_field!(footer, u16, 510);

    // FAT32 only
    define_field!(fat_size32, u32, 36);
    define_field!(fs_ver, u16, 42);
    define_field!(first_root_dir_cluster, u32, 44);
    define_field!(fs_info, u16, 48);
    define_field!(backup_boot_block, u16, 50);
}
