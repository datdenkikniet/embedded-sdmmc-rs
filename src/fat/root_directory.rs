use core::marker::PhantomData;

use crate::{blockdevice::BlockIter, BlockCount, BlockDevice, BlockIdx};

use super::{cluster::ClusterIterator, Cluster, FatVolume, SectorIter};

#[derive(Debug, Clone, Copy)]
pub enum RootDirectorySectors {
    Cluster(Cluster),
    Region {
        start_sector: BlockIdx,
        len: BlockCount,
    },
}

impl RootDirectorySectors {
    pub fn iter<BD>(&self, fat_volume: &mut FatVolume<BD>) -> RootDirIter<BD>
    where
        BD: BlockDevice,
    {
        RootDirIter::new(fat_volume, self.clone())
    }
}

#[derive(Debug)]
enum RootDirIterInner {
    Cluster(ClusterIterator),
    Region(BlockIter),
}

#[derive(Debug)]
pub struct RootDirIter<BD>
where
    BD: BlockDevice,
{
    inner: RootDirIterInner,
    _volume: PhantomData<BD>,
}

impl<BD> RootDirIter<BD>
where
    BD: BlockDevice,
{
    pub fn new(fat_volume: &mut FatVolume<BD>, start: RootDirectorySectors) -> Self {
        let inner = match start {
            RootDirectorySectors::Cluster(cluster) => {
                RootDirIterInner::Cluster(fat_volume.all_sectors(cluster))
            }
            RootDirectorySectors::Region { start_sector, len } => {
                RootDirIterInner::Region(start_sector.range(len))
            }
        };

        Self {
            inner,
            _volume: Default::default(),
        }
    }
}

impl<BD> SectorIter<BD> for RootDirIter<BD>
where
    BD: BlockDevice,
{
    fn next(&mut self, volume: &mut FatVolume<BD>) -> Option<BlockIdx> {
        match &mut self.inner {
            RootDirIterInner::Cluster(cluster) => cluster.next(volume),
            RootDirIterInner::Region(sectors) => sectors.next(),
        }
    }
}
