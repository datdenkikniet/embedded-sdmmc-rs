use crate::{blockdevice::BlockIter, BlockCount, BlockDevice, BlockIdx};

use super::{cluster::ClusterIterator, Cluster, FatVolume, SectorIter};

#[derive(Debug, Clone, Copy)]
pub enum RootDirectorySectors {
    Cluster(Cluster),
    Region {
        start_block: BlockIdx,
        len: BlockCount,
    },
}

impl RootDirectorySectors {
    pub fn iter(&self, fat_number: u32, sectors_per_cluster: BlockCount) -> RootDirIter {
        RootDirIter::new(fat_number, sectors_per_cluster, self.clone())
    }
}

#[derive(Debug)]
enum RootDirIterInner {
    Cluster(ClusterIterator),
    Region(BlockIter),
}

#[derive(Debug)]
pub struct RootDirIter {
    inner: RootDirIterInner,
}

impl RootDirIter {
    pub fn new(
        fat_number: u32,
        sectors_per_cluster: BlockCount,
        start: RootDirectorySectors,
    ) -> Self {
        let inner = match start {
            RootDirectorySectors::Cluster(cluster) => {
                RootDirIterInner::Cluster(cluster.all_sectors(fat_number, sectors_per_cluster))
            }
            RootDirectorySectors::Region { start_block, len } => {
                RootDirIterInner::Region(start_block.range(len))
            }
        };

        Self { inner }
    }
}

impl SectorIter for RootDirIter {
    fn next<BD>(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx>
    where
        BD: BlockDevice,
    {
        match &mut self.inner {
            RootDirIterInner::Cluster(cluster) => cluster.next(volume),
            RootDirIterInner::Region(sectors) => sectors.next(),
        }
    }
}
