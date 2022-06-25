use crate::BlockDevice;

use super::{
    block_byte_cache::BlockByteCache, cluster::ClusterSectorIterator, directory::DirEntry,
    FatVolume,
};

#[derive(Debug, Copy, Clone)]
pub enum OpenMode {
    ReadWrite,
    ReadOnly,
}

pub enum FileError<E> {
    DeviceError(E),
}

pub struct File<BD>
where
    BD: BlockDevice,
{
    read_cache: BlockByteCache<BD, ClusterSectorIterator>,
    mode: OpenMode,
    dir_entry: DirEntry<BD>,
}

impl<BD> File<BD>
where
    BD: BlockDevice,
{
    pub const EMPTY: Option<Self> = None;

    pub(crate) fn from_dir_entry(
        dir_entry: DirEntry<BD>,
        volume: &mut FatVolume<BD>,
        mode: OpenMode,
    ) -> Option<Self> {
        if !dir_entry.is_dir() {
            let file_size = dir_entry.file_size() as usize;
            let sectors = volume.all_sectors(*dir_entry.first_cluster());
            Some(Self {
                dir_entry,
                read_cache: BlockByteCache::new(Some(file_size), sectors),
                mode,
            })
        } else {
            None
        }
    }

    pub fn close(self, volume: &mut FatVolume<BD>) -> DirEntry<BD> {
        volume.close_dir_entry(&self.dir_entry);
        self.dir_entry
    }

    pub fn reset(&mut self, volume: &mut FatVolume<BD>) {
        self.read_cache
            .reset(volume.all_sectors(*self.dir_entry.first_cluster()));
    }

    pub fn dir_entry(&self) -> &DirEntry<BD> {
        &self.dir_entry
    }

    pub fn activate<'file, 'volume>(
        &'file mut self,
        volume: &'volume mut FatVolume<BD>,
    ) -> Option<ActiveFile<'file, 'volume, BD>> {
        if volume.is_open(&self.dir_entry) {
            Some(ActiveFile { file: self, volume })
        } else {
            None
        }
    }
}

#[cfg(feature = "improper_deref_impl")]
impl<BD> core::ops::Deref for File<BD>
where
    BD: BlockDevice,
{
    type Target = DirEntry<BD>;

    fn deref(&self) -> &Self::Target {
        &self.dir_entry
    }
}

pub enum WriteError<E> {
    DeviceError(E),
    FileNotWriteable,
}

pub struct ActiveFile<'file, 'volume, BD>
where
    BD: BlockDevice,
{
    file: &'file mut File<BD>,
    volume: &'volume mut FatVolume<BD>,
}

impl<'file, 'volume, BD> ActiveFile<'file, 'volume, BD>
where
    BD: BlockDevice,
{
    pub fn file(&self) -> &File<BD> {
        self.file
    }

    pub fn reset(&mut self) {
        self.file.reset(self.volume)
    }

    pub fn read(&mut self, data: &mut [u8]) -> Result<usize, BD::Error> {
        let mut data = data;
        let mut read_bytes_total = 0;

        while !data.is_empty() {
            if !self.file.read_cache.more_data(self.volume) {
                break;
            }

            let (read_bytes, _) = self.file.read_cache.read(data);
            data = &mut data[read_bytes..];
            read_bytes_total += read_bytes;
            if read_bytes == 0 {
                break;
            }
        }

        Ok(read_bytes_total)
    }

    pub fn write(&mut self, _data: &[u8]) -> Result<usize, WriteError<BD::Error>> {
        match self.file.mode {
            OpenMode::ReadWrite => {}
            OpenMode::ReadOnly => return Err(WriteError::FileNotWriteable),
        }
        todo!()
    }
}

#[cfg(feature = "improper_deref_impl")]
impl<BD> core::ops::Deref for ActiveFile<'_, '_, BD>
where
    BD: BlockDevice,
{
    type Target = File<BD>;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}
