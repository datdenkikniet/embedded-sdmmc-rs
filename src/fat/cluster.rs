use crate::{blockdevice::BlockIter, BlockCount, BlockDevice, BlockIdx};

use super::{FatVolume, SectorIter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cluster(pub(crate) u32);

impl Cluster {
    pub fn new(cluster_number: u32) -> Self {
        Self(cluster_number)
    }

    pub fn sectors(&self, sectors_per_cluster: BlockCount) -> BlockIter {
        let start = BlockIdx(self.0);
        start.range(sectors_per_cluster)
    }

    pub fn all_sectors(&self, fat_number: u32, sectors_per_cluster: BlockCount) -> ClusterIterator {
        ClusterIterator::new(fat_number, self.clone(), sectors_per_cluster)
    }
}

#[derive(Debug, Clone)]
pub struct ClusterIterator {
    fat_number: u32,
    current_cluster: Cluster,
    sectors_per_cluster: BlockCount,
    cluster_sectors: BlockIter,
}

impl ClusterIterator {
    pub fn new(fat_number: u32, start: Cluster, sectors_per_cluster: BlockCount) -> Self {
        Self {
            fat_number,
            sectors_per_cluster,
            current_cluster: start,
            cluster_sectors: start.sectors(sectors_per_cluster),
        }
    }
}

impl SectorIter for ClusterIterator {
    fn next<BD>(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx>
    where
        BD: BlockDevice,
    {
        if let Some(next_sector) = self.cluster_sectors.next() {
            return Some(next_sector);
        } else {
            let next_cluster = volume
                .find_next_cluster(self.fat_number, &self.current_cluster)
                .ok()??;
            self.cluster_sectors = next_cluster.sectors(self.sectors_per_cluster);
            self.cluster_sectors.next()
        }
    }
}
