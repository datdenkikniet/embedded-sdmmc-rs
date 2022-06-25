use core::marker::PhantomData;

use crate::BlockDevice;

use super::{
    block_byte_cache::BlockByteCache, cluster::ClusterSectorIterator, directory::DirEntry,
    FatVolume,
};

#[derive(Debug, Copy, Clone)]
pub(crate) enum OpenMode {
    ReadWrite,
    ReadOnly,
}

pub struct FileHandle<BD>
where
    BD: BlockDevice,
{
    id: u32,
    mode: OpenMode,
    _block_device: PhantomData<FatVolume<BD>>,
}

impl<BD> FileHandle<BD>
where
    BD: BlockDevice,
{
    pub(crate) fn new(id: u32, mode: OpenMode) -> Self {
        Self {
            id,
            mode,
            _block_device: Default::default(),
        }
    }

    pub(crate) fn copy(&self) -> Self {
        Self {
            id: self.id,
            mode: self.mode,
            _block_device: Default::default(),
        }
    }
}

pub enum FileError<E> {
    DeviceError(E),
}

pub struct File<BD>
where
    BD: BlockDevice,
{
    read_cache: BlockByteCache<BD, ClusterSectorIterator>,
    dir_entry: DirEntry<BD>,
}

impl<BD> core::fmt::Debug for File<BD>
where
    BD: BlockDevice,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File").finish()
    }
}

impl<BD> File<BD>
where
    BD: BlockDevice,
{
    pub const EMPTY: Option<Self> = None;

    pub fn new(dir_entry: DirEntry<BD>, volume: &mut FatVolume<BD>) -> Option<Self> {
        if !dir_entry.is_dir() {
            let file_size = dir_entry.file_size() as usize;
            let sectors = volume.all_sectors(dir_entry.first_cluster().clone());
            Some(Self {
                dir_entry,
                read_cache: BlockByteCache::new(Some(file_size), sectors),
            })
        } else {
            None
        }
    }

    pub(crate) fn reset(&mut self, volume: &mut FatVolume<BD>) {
        self.read_cache
            .reset(volume.all_sectors(self.dir_entry.first_cluster().clone()));
    }

    pub(crate) fn read(
        &mut self,
        fat_volume: &mut FatVolume<BD>,
        data: &mut [u8],
    ) -> Result<usize, BD::Error> {
        let mut data = data;
        let mut read_bytes_total = 0;

        while data.len() > 0 {
            if !self.read_cache.more_data(fat_volume) {
                break;
            }

            let read_bytes = self.read_cache.read(data);
            data = &mut data[read_bytes..];
            read_bytes_total += read_bytes;
            if read_bytes == 0 {
                break;
            }
        }

        Ok(read_bytes_total)
    }

    pub fn dir_entry(&self) -> &DirEntry<BD> {
        &self.dir_entry
    }
}
