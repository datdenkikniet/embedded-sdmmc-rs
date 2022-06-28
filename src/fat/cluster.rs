use crate::{block_device::BlockIter, BlockCount, BlockDevice, BlockIdx};

use super::{Entry, FatVolume, SectorIter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cluster(Entry);

impl Cluster {
    pub fn new(entry: Entry) -> Self {
        Self(entry)
    }

    pub fn sectors(&self, data_start: BlockIdx, sectors_per_cluster: BlockCount) -> BlockIter {
        let start = data_start + (BlockIdx(self.0.value.saturating_sub(2)) * sectors_per_cluster);
        start.range(sectors_per_cluster)
    }

    pub fn entry(&self) -> &Entry {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct ClusterSectorIterator {
    fat_number: u32,
    current_cluster: Cluster,
    data_start: BlockIdx,
    sectors_per_cluster: BlockCount,
    current_cluster_sectors: BlockIter,
}

impl ClusterSectorIterator {
    pub(crate) fn new(
        fat_number: u32,
        data_start: BlockIdx,
        start: Cluster,
        sectors_per_cluster: BlockCount,
    ) -> Self {
        Self {
            fat_number,
            sectors_per_cluster,
            data_start,
            current_cluster: start,
            current_cluster_sectors: start.sectors(data_start, sectors_per_cluster),
        }
    }
}

impl<BD> SectorIter<BD> for ClusterSectorIterator
where
    BD: BlockDevice,
{
    fn next(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx> {
        if let Some(next_sector) = self.current_cluster_sectors.next() {
            Some(next_sector)
        } else {
            self.current_cluster = volume
                .find_next_cluster(self.fat_number, &self.current_cluster)
                .ok()??;
            self.current_cluster_sectors = self
                .current_cluster
                .sectors(self.data_start, self.sectors_per_cluster);
            self.current_cluster_sectors.next()
        }
    }
}
