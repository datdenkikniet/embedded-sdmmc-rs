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

#[derive(Debug)]
pub struct File<'parent, BD>
where
    BD: BlockDevice,
{
    pub(crate) fd: usize,
    read_cache: BlockByteCache<BD, ClusterSectorIterator>,
    mode: OpenMode,
    dir_entry: DirEntry<'parent, BD>,
}

impl<'parent, BD> File<'parent, BD>
where
    BD: BlockDevice,
{
    pub const EMPTY: Option<Self> = None;

    pub(crate) fn from_dir_entry(
        dir_entry: DirEntry<'parent, BD>,
        volume: &mut FatVolume<BD>,
        mode: OpenMode,
        fd: usize,
    ) -> Option<Self> {
        if !dir_entry.info().is_dir() {
            let file_size = dir_entry.info().file_size() as usize;
            let sectors = volume.all_sectors(*dir_entry.info().first_cluster());
            Some(Self {
                dir_entry,
                read_cache: BlockByteCache::new(Some(file_size), sectors),
                mode,
                fd,
            })
        } else {
            None
        }
    }

    pub fn close(self, volume: &mut FatVolume<BD>) -> DirEntry<'parent, BD> {
        volume.close_dir_entry(&self.dir_entry);
        self.dir_entry
    }

    pub fn delete(self, volume: &mut FatVolume<BD>) -> Result<(), BD::Error> {
        volume.deallocate_dir_entry(self.dir_entry)
    }

    pub fn reset(&mut self, volume: &mut FatVolume<BD>) {
        self.read_cache
            .reset(volume.all_sectors(*self.dir_entry.info().first_cluster()));
    }

    pub fn dir_entry(&self) -> &DirEntry<BD> {
        &self.dir_entry
    }

    pub fn activate<'file, 'volume>(
        &'file mut self,
        volume: &'volume mut FatVolume<BD>,
    ) -> Option<ActiveFile<'file, 'volume, 'parent, BD>> {
        if volume.file_is_open(&self) {
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
    type Target = DirEntryData<BD>;

    fn deref(&self) -> &Self::Target {
        &self.dir_entry
    }
}

#[derive(Debug, Clone)]
pub enum WriteError<E> {
    DeviceError(E),
    FileClosed,
    FileNotWriteable,
}

#[derive(Debug, Clone)]
pub enum ReadError<E> {
    DeviceError(E),
    FileClosed,
}

pub struct ActiveFile<'file, 'volume, 'parent, BD>
where
    BD: BlockDevice,
{
    file: &'file mut File<'parent, BD>,
    volume: &'volume mut FatVolume<BD>,
}

impl<'file, 'volume, 'parent, BD> ActiveFile<'file, 'volume, 'parent, BD>
where
    BD: BlockDevice,
{
    pub fn release(self) -> &'file mut File<'parent, BD> {
        self.file
    }

    pub fn file(&self) -> &File<BD> {
        self.file
    }

    pub fn reset(&mut self) {
        self.file.reset(self.volume)
    }

    pub fn read(&mut self, data: &mut [u8]) -> Result<usize, ReadError<BD::Error>> {
        if !self.volume.file_is_open(&self.file) {
            return Err(ReadError::FileClosed);
        }

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
        if !self.volume.file_is_open(&self.file) {
            return Err(WriteError::FileClosed);
        }

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
