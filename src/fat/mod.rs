#![allow(missing_docs)]

mod fat16;
use core::ops::RangeInclusive;

use fat16::FilesystemInfo as Fat16Info;

mod fat32;
use fat32::FilesystemInfo as Fat32Info;

pub struct File;
pub struct Sector;
pub struct Cluster;
pub struct Partition;
pub struct Volume {
    reserved_sectors: RangeInclusive<usize>,
    fat_region: RangeInclusive<usize>,
    root_directory_region: Option<RangeInclusive<usize>>,
    data_region: RangeInclusive<usize>,
}

#[derive(Debug, Clone)]
pub struct FilenameError;

enum FileSystem {
    Fat16(Fat16Info),
    Fat32(Fat32Info),
}
