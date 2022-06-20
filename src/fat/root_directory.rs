use crate::{BlockCount, BlockDevice, BlockIdx};

use super::{Cluster, FatVolume, SectorIter};

#[derive(Debug, Clone, Copy)]
pub enum RootDirectorySectors {
    Cluster(Cluster),
    Region {
        start_block: BlockIdx,
        len: BlockCount,
    },
}

impl RootDirectorySectors {
    pub fn iter(&self, fat_number: u32) -> RootDirIter {
        RootDirIter::new(fat_number, self.clone())
    }
}

#[derive(Debug)]
pub struct RootDirIter {
    current: RootDirectorySectors,
    fat_number: u32,
    iterated_cluster_sectors: u32,
}

impl RootDirIter {
    pub fn new(fat_number: u32, start: RootDirectorySectors) -> Self {
        Self {
            current: start,
            fat_number,
            iterated_cluster_sectors: 0,
        }
    }
}

impl SectorIter for RootDirIter {
    fn next<BD>(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx>
    where
        BD: BlockDevice,
    {
        let Self {
            current,
            fat_number,
            iterated_cluster_sectors,
        } = self;

        match current {
            RootDirectorySectors::Cluster(cluster) => {
                if *iterated_cluster_sectors == volume.bpb.sectors_per_cluster().0 {
                    *cluster = volume.find_next_cluster(*fat_number, &cluster).ok()??;
                    *iterated_cluster_sectors = 0;
                }
                cluster
                    .sectors(volume.bpb.sectors_per_cluster())
                    .skip(*iterated_cluster_sectors as usize)
                    .next()
            }
            RootDirectorySectors::Region { start_block, len } => {
                let res = *start_block;
                let still_there = len != &BlockCount(0);
                if still_there {
                    *len -= BlockCount(1);
                    *start_block += BlockCount(1);
                    Some(res)
                } else {
                    None
                }
            }
        }
    }
}
