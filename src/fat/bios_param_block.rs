use core::num::{NonZeroU16, NonZeroU32, NonZeroU8};

use crate::{Block, BlockCount, BlockIdx};

use super::{root_directory::RootDirectorySectors, Cluster, FatType};

#[derive(Debug, Clone, Copy, PartialEq)]
enum FatInfo {
    Fat16,
    Fat32(Fat32Info),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Fat32Info {
    root_cluster: Cluster,
}

impl From<FatInfo> for FatType {
    fn from(other: FatInfo) -> FatType {
        match other {
            FatInfo::Fat16 => FatType::Fat16,
            FatInfo::Fat32(_) => FatType::Fat32,
        }
    }
}

#[derive(Clone)]
pub struct BiosParameterBlock {
    fat_info: FatInfo,
    fat_size: u32,
    reserved_sector_count: NonZeroU16,
    bytes_per_sector: NonZeroU16,
    media: NonZeroU8,
    root_entry_count: u16,
    cluster_count: u32,
    num_fats: u8,
    sectors_per_cluster: NonZeroU8,
}

#[derive(Debug, Clone, Copy)]
pub enum BpbError {
    Fat12NotSupported,
    Fat32Field(&'static str),
    InvalidMedia(u8),
    BothSectorCountsZero,
    BothSectorCountsNotZero,
    RootEntryCountSize,
    Fat32(Fat32BpbError),
    InvalidBytesPerSector(u16),
    InvalidSectorsPerCluster(u8),
    ReservedSectorCountZero,
    InvalidSignature([u8; 2]),
}

#[derive(Debug, Clone, Copy)]
pub enum Fat32BpbError {
    Count16NotZero,
    FatSize16NotZero,
    RootEntryCountNotZero,
    FsVerNotZero,
    RootClusterLessThanTwo,
    InvalidBackupBootSector(u16),
}

impl core::fmt::Debug for BiosParameterBlock {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BiosParameterBlock")
            .field("fat_info", &self.fat_info)
            .field("fat_size", &self.fat_size)
            .field("reserved_sector_count", &self.reserved_sector_count)
            .field("bytes_per_sector", &self.bytes_per_sector)
            .field("media", &self.media)
            .field("root_entry_count", &self.root_entry_count)
            .field("cluster_count", &self.cluster_count)
            .field("num_fats", &self.num_fats)
            .field("sectors_per_cluster", &self.sectors_per_cluster)
            .finish()
    }
}

impl PartialEq<BiosParameterBlock> for BiosParameterBlock {
    fn eq(&self, other: &BiosParameterBlock) -> bool {
        self.fat_info == other.fat_info
            && self.fat_size == other.fat_size
            && self.reserved_sector_count == other.reserved_sector_count
            && self.bytes_per_sector == other.bytes_per_sector
            && self.media == other.media
            && self.root_entry_count == other.root_entry_count
            && self.cluster_count == other.cluster_count
            && self.num_fats == other.num_fats
            && self.sectors_per_cluster == other.sectors_per_cluster
    }
}

/// The BPB_Reserved and BS_* fields are not verified.
impl BiosParameterBlock {
    pub const SIGNATURE: [u8; 2] = [0x55, 0xAA];

    pub fn new(mut block: Block) -> Result<Self, BpbError> {
        let raw = BiosParameterBlockRaw::new(&mut block);

        let reserved_sector_count =
            NonZeroU16::new(raw.rsvd_sec_cnt()).ok_or(BpbError::ReservedSectorCountZero)?;

        let bytes_per_sector = Self::bytes_per_sector_checked(raw.bytes_per_sec())?;

        let size_16 = raw.fat_sz_16();
        let size_32 = raw.fat_sz_32();

        let fat_size = if size_16 == 0 {
            size_32
        } else {
            size_16 as u32
        };

        let sectors_per_cluster = Self::sectors_per_cluster_checked(raw.sec_per_clu())?;

        let media = Self::media_checked(raw.media())?;

        let root_entry_count = raw.root_ent_cnt();

        let total_sector_count = Self::total_sector_count_checked(&raw)?;

        let root_dir_sectors =
            Self::compute_root_dir_sectors(root_entry_count as u32, bytes_per_sector.get() as u32);
        let data_sectors = Self::compute_data_sectors(
            fat_size,
            total_sector_count.get(),
            reserved_sector_count.get() as u32,
            raw.num_fats() as u32,
            root_dir_sectors,
        );
        let cluster_count = Self::compute_cluster_count(data_sectors, sectors_per_cluster.into());

        let num_fats = raw.num_fats();

        let fat_info = Self::compute_fat_type(cluster_count, &raw)?;

        let me = Self {
            fat_info,
            fat_size,
            reserved_sector_count,
            bytes_per_sector,
            media,
            root_entry_count,
            cluster_count,
            num_fats,
            sectors_per_cluster,
        };

        let verification_error = Self::verify_signature(raw.signature_word())
            .or_else(|| me.verify_root_entry_count())
            .or_else(|| me.verify_total_sector_count(&raw))
            .or_else(|| me.verify_fat_size(&raw));

        if let Some(err) = verification_error {
            Err(err)
        } else {
            Ok(me)
        }
    }

    pub fn reserved_sector_count(&self) -> BlockCount {
        BlockCount(self.reserved_sector_count.get() as u32)
    }

    pub fn bytes_per_sector(&self) -> NonZeroU16 {
        self.bytes_per_sector
    }

    pub fn fat_size(&self) -> BlockCount {
        BlockCount(self.fat_size)
    }

    pub fn fat_type(&self) -> FatType {
        self.fat_info.into()
    }

    pub fn media(&self) -> NonZeroU8 {
        self.media
    }

    pub fn compute_root_dir_sectors(root_entry_count: u32, bytes_per_sector: u32) -> u32 {
        ((root_entry_count * 32) + (bytes_per_sector - 1)) / bytes_per_sector
    }

    pub fn cluster_count(&self) -> u32 {
        self.cluster_count
    }

    pub fn maximum_valid_cluster(&self) -> u32 {
        self.cluster_count + 1
    }

    pub fn sectors_per_cluster(&self) -> BlockCount {
        BlockCount(self.sectors_per_cluster.get() as u32)
    }

    pub fn total_cluster_count(&self) -> BlockCount {
        BlockCount(self.cluster_count + 2)
    }

    pub fn fat_start(&self) -> BlockIdx {
        BlockIdx(self.reserved_sector_count.get() as u32)
    }

    pub fn fat_len(&self) -> BlockCount {
        BlockCount(self.num_fats as u32) * self.fat_size()
    }

    pub fn data_start(&self) -> BlockIdx {
        self.root_start() + self.root_len()
    }

    pub fn root_sectors(&self) -> RootDirectorySectors {
        match self.fat_info {
            FatInfo::Fat16 => RootDirectorySectors::Region {
                start_sector: self.root_start(),
                len: self.root_len(),
            },
            FatInfo::Fat32(fat32_info) => RootDirectorySectors::Cluster(fat32_info.root_cluster),
        }
    }

    fn root_start(&self) -> BlockIdx {
        self.fat_start() + self.fat_len()
    }

    fn root_len(&self) -> BlockCount {
        match self.fat_info {
            FatInfo::Fat16 => BlockCount(Self::compute_root_dir_sectors(
                self.root_entry_count as u32,
                self.bytes_per_sector.get() as u32,
            )),
            FatInfo::Fat32(_) => BlockCount(0),
        }
    }

    fn compute_data_sectors(
        fat_size: u32,
        total_sector_count: u32,
        reserved_sectors: u32,
        num_fats: u32,
        root_dir_sectors: u32,
    ) -> u32 {
        total_sector_count - (reserved_sectors + (num_fats * fat_size) + root_dir_sectors)
    }
    fn compute_cluster_count(data_sectors: u32, sectors_per_cluster: NonZeroU32) -> u32 {
        data_sectors / sectors_per_cluster.get()
    }

    fn compute_fat_type(
        cluster_count: u32,
        raw: &BiosParameterBlockRaw,
    ) -> Result<FatInfo, BpbError> {
        if cluster_count < 4085 {
            Err(BpbError::Fat12NotSupported)
        } else if cluster_count < 65525 {
            Ok(FatInfo::Fat16)
        } else {
            let root_cluster = Self::root_cluster_checked(raw.root_clus())?;
            Ok(FatInfo::Fat32(Fat32Info {
                root_cluster: Cluster(root_cluster),
            }))
        }
    }

    fn sectors_per_cluster_checked(sectors_per_cluster: u8) -> Result<NonZeroU8, BpbError> {
        match sectors_per_cluster {
            1 | 2 | 4 | 8 | 16 | 32 | 64 | 128 => {
                Ok(
                    // SAFETY: sectors_per_cluster is not 0
                    unsafe { NonZeroU8::new_unchecked(sectors_per_cluster) },
                )
            }
            _ => Err(BpbError::InvalidSectorsPerCluster(sectors_per_cluster)),
        }
    }

    fn total_sector_count_checked(raw: &BiosParameterBlockRaw) -> Result<NonZeroU32, BpbError> {
        let sec_16 = raw.tot_sec_16();
        let sec_32 = raw.tot_sec_32();
        if sec_16 == 0 && sec_32 != 0 {
            Ok(unsafe { NonZeroU32::new_unchecked(sec_32) })
        } else if sec_32 == 0 && sec_16 != 0 {
            Ok(unsafe { NonZeroU32::new_unchecked(sec_16 as u32) })
        } else if sec_32 != 0 && sec_16 != 0 {
            Err(BpbError::BothSectorCountsNotZero)
        } else {
            Err(BpbError::BothSectorCountsZero)
        }
    }

    fn bytes_per_sector_checked(bytes_per_sec: u16) -> Result<NonZeroU16, BpbError> {
        match bytes_per_sec {
            // SAFTEY: bytes_per_sector is not 0
            512 | 1024 | 2048 | 4096 => Ok(unsafe { NonZeroU16::new_unchecked(bytes_per_sec) }),
            _ => Err(BpbError::InvalidBytesPerSector(bytes_per_sec)),
        }
    }

    fn media_checked(media: u8) -> Result<NonZeroU8, BpbError> {
        match media {
            0xF0 | 0xF8 | 0xF9 | 0xFA | 0xFB | 0xFC | 0xFD | 0xFE | 0xFF => {
                Ok(
                    // SAFETY: media is not 0
                    unsafe { NonZeroU8::new_unchecked(media) },
                )
            }
            _ => Err(BpbError::InvalidMedia(media)),
        }
    }

    fn verify_total_sector_count(&self, raw: &BiosParameterBlockRaw) -> Option<BpbError> {
        let fs_16 = raw.tot_sec_16();
        let fs_32 = raw.tot_sec_32();

        if fs_16 == 0 && fs_32 == 0 {
            return Some(BpbError::BothSectorCountsZero);
        }

        match self.fat_info {
            FatInfo::Fat16 => None,
            FatInfo::Fat32(_) => {
                if fs_16 == 0 {
                    None
                } else {
                    Some(BpbError::Fat32(Fat32BpbError::Count16NotZero))
                }
            }
        }
    }

    fn verify_root_entry_count(&self) -> Option<BpbError> {
        let value = self.root_entry_count;
        match self.fat_info {
            FatInfo::Fat16 => {
                let bytes_per_sec = self.bytes_per_sector().get() as u32;
                let multiplied = value as u32 * 32;
                if multiplied % bytes_per_sec == 0 {
                    None
                } else {
                    Some(BpbError::RootEntryCountSize)
                }
            }
            FatInfo::Fat32(_) => {
                if value != 0 {
                    Some(BpbError::Fat32(Fat32BpbError::RootEntryCountNotZero))
                } else {
                    None
                }
            }
        }
    }

    fn verify_fat_size(&self, raw: &BiosParameterBlockRaw) -> Option<BpbError> {
        let fs_16 = raw.fat_sz_16();
        let _fs_32 = raw.fat_sz_32();

        match self.fat_info {
            FatInfo::Fat16 => None,
            FatInfo::Fat32(_) => {
                if fs_16 == 0 {
                    None
                } else {
                    Some(BpbError::Fat32(Fat32BpbError::FatSize16NotZero))
                }
            }
        }
    }

    // This procedure is the same for all FATs
    fn verify_signature(signature: [u8; 2]) -> Option<BpbError> {
        if signature == Self::SIGNATURE[..] {
            None
        } else {
            Some(BpbError::InvalidSignature(signature))
        }
    }

    // All following functions are fat32 only

    #[allow(dead_code)]
    fn fs_version_checked(fs_version: u16) -> Result<u16, BpbError> {
        if fs_version == 0 {
            Ok(fs_version)
        } else {
            Err(BpbError::Fat32(Fat32BpbError::FsVerNotZero))
        }
    }

    #[allow(dead_code)]
    fn root_cluster_checked(root_cluster: u32) -> Result<u32, BpbError> {
        if root_cluster >= 2 {
            Ok(root_cluster)
        } else {
            Err(BpbError::Fat32(Fat32BpbError::RootClusterLessThanTwo))
        }
    }

    #[allow(dead_code)]
    fn bk_boot_sector_checked(bk_boot_sector: u16) -> Result<u16, BpbError> {
        if bk_boot_sector == 0 || bk_boot_sector == 6 {
            Ok(bk_boot_sector)
        } else {
            Err(BpbError::Fat32(Fat32BpbError::InvalidBackupBootSector(
                bk_boot_sector,
            )))
        }
    }
}

#[derive(Debug)]
pub struct BiosParameterBlockRaw<'a> {
    block: &'a mut Block,
}

impl<'a> BiosParameterBlockRaw<'a> {
    pub(crate) fn new(block: &'a mut Block) -> Self {
        Self { block }
    }

    fn data(&self) -> &[u8] {
        &self.block.contents
    }

    fn data_mut(&mut self) -> &mut [u8] {
        &mut self.block.contents
    }

    define_field!(bytes_per_sec, set_bytes_per_sec, u16, 11);
    define_field!(sec_per_clu, set_sec_per_clu, u8, 13);
    define_field!(rsvd_sec_cnt, set_rsvd_sec_cnt, u16, 14);
    define_field!(num_fats, set_num_fats, u8, 16);
    define_field!(root_ent_cnt, set_root_ent_cnt, u16, 17);
    define_field!(tot_sec_16, set_tot_sec_16, u16, 19);
    define_field!(media, set_media, u8, 21);
    define_field!(fat_sz_16, set_fat_sz_16, u16, 22);
    define_field!(sectors_per_track, set_sectors_per_track, u16, 24);
    define_field!(number_of_heads, set_number_of_heads, u16, 26);
    define_field!(hidden_sectors, set_hidden_sectors, u32, 28);
    define_field!(tot_sec_32, set_tot_sec_32, u32, 32);

    // FAT32 specific structure
    define_field!(fat_sz_32, set_fat_sz_32, u32, 36);
    define_field!(ext_flags, set_ext_flags, u16, 40);
    define_field!(fs_ver, set_fs_ver, u16, 42);
    define_field!(root_clus, set_root_clus, u32, 44);
    define_field!(fs_info, set_fs_info, u16, 48);
    define_field!(bk_boot_sec, set_bk_boot_sec, u16, 50);

    pub fn signature_word(&self) -> [u8; 2] {
        let d = self.data();
        [d[510], d[511]]
    }

    pub fn set_signature(&mut self, bytes: [u8; 2]) {
        self.data_mut()[510..512].copy_from_slice(&bytes)
    }
}
