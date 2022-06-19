use core::num::{NonZeroU16, NonZeroU32, NonZeroU8};

use crate::Block;

use super::FatType;

#[derive(Debug, Clone)]
pub struct BiosParameterBlock {
    fat_type: FatType,
    fat_size: u32,
    reserved_sector_count: NonZeroU16,
    bytes_per_sector: NonZeroU16,
    media: NonZeroU8,
    raw: BiosParameterBlockRaw,
    root_entry_count: u16,
    cluster_count: u32,
}

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

pub enum Fat32BpbError {
    Count16NotZero,
    FatSize16NotZero,
    RootEntryCountNotZero,
    FsVerNotZero,
    RootClusterLessThanTwo,
    InvalidBackupBootSector(u16),
}

/// The BPB_Reserved and BS_* fields are not verified.
impl BiosParameterBlock {
    const SIGNATURE: [u8; 2] = [0x55, 0xAA];

    pub fn new(block: Block) -> Result<Self, BpbError> {
        let raw = BiosParameterBlockRaw { block };

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

        let root_entry_count = raw.root_entr_cnt();

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

        let mut me = Self {
            // Assume we have FAT16, to be overwritten later
            fat_type: FatType::Fat16,
            fat_size,
            reserved_sector_count,
            bytes_per_sector,
            media,
            raw,
            root_entry_count,
            cluster_count,
        };

        me.fat_type = me.compute_fat_type()?;

        let verification_error = me
            .verify_signature()
            .or(me.verify_root_entry_count())
            .or(me.verify_total_sector_count())
            .or(me.verify_fat_size());

        if let Some(err) = verification_error {
            Err(err)
        } else {
            Ok(me)
        }
    }

    pub fn reserved_sector_count(&self) -> NonZeroU16 {
        self.reserved_sector_count
    }

    pub fn bytes_per_sector(&self) -> NonZeroU16 {
        self.bytes_per_sector
    }

    pub fn fat_size(&self) -> u32 {
        self.fat_size
    }

    pub fn fat_type(&self) -> FatType {
        self.fat_type
    }

    pub fn media(&self) -> NonZeroU8 {
        self.media
    }

    pub fn compute_root_dir_sectors(root_entry_count: u32, bytes_per_sector: u32) -> u32 {
        let root_dir_sectors = (root_entry_count + (bytes_per_sector - 1)) / bytes_per_sector;
        root_dir_sectors
    }

    pub fn cluster_count(&self) -> u32 {
        self.cluster_count
    }

    pub fn maximum_valid_cluster(&self) -> u32 {
        self.cluster_count + 1
    }

    pub fn total_cluster_count(&self) -> u32 {
        self.cluster_count + 2
    }

    fn compute_data_sectors(
        fat_size: u32,
        total_sector_count: u32,
        reserved_sectors: u32,
        num_fats: u32,
        root_dir_sectors: u32,
    ) -> u32 {
        let data_sectors =
            total_sector_count - (reserved_sectors + (num_fats * fat_size) + root_dir_sectors);

        data_sectors
    }
    fn compute_cluster_count(data_sectors: u32, sectors_per_cluster: NonZeroU32) -> u32 {
        let data_cluster_count = data_sectors / sectors_per_cluster.get();
        data_cluster_count
    }

    fn compute_fat_type(&self) -> Result<FatType, BpbError> {
        if self.cluster_count < 4085 {
            return Err(BpbError::Fat12NotSupported);
        } else if self.cluster_count < 65525 {
            Ok(FatType::Fat16)
        } else {
            Ok(FatType::Fat32)
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
            _ => return Err(BpbError::InvalidBytesPerSector(bytes_per_sec)),
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

    fn verify_total_sector_count(&self) -> Option<BpbError> {
        let _16 = self.raw.tot_sec_16();
        let _32 = self.raw.tot_sec_32();

        if _16 == 0 && _32 == 0 {
            return Some(BpbError::BothSectorCountsZero);
        }

        match self.fat_type {
            FatType::Fat16 => None,
            FatType::Fat32 => {
                if _16 == 0 {
                    None
                } else {
                    Some(BpbError::Fat32(Fat32BpbError::Count16NotZero))
                }
            }
        }
    }

    fn verify_root_entry_count(&self) -> Option<BpbError> {
        let value = self.root_entry_count;
        match self.fat_type {
            FatType::Fat16 => {
                let bytes_per_sec = self.bytes_per_sector().get() as u32;
                let multiplied = value as u32 * 32;
                if multiplied % bytes_per_sec == 0 {
                    None
                } else {
                    Some(BpbError::RootEntryCountSize)
                }
            }
            FatType::Fat32 => {
                if value != 0 {
                    Some(BpbError::Fat32(Fat32BpbError::RootEntryCountNotZero))
                } else {
                    None
                }
            }
        }
    }

    fn verify_fat_size(&self) -> Option<BpbError> {
        let _16 = self.raw.fat_sz_16();
        let _32 = self.raw.fat_sz_32();

        match self.fat_type {
            FatType::Fat16 => None,
            FatType::Fat32 => {
                if _16 == 0 {
                    None
                } else {
                    Some(BpbError::Fat32(Fat32BpbError::FatSize16NotZero))
                }
            }
        }
    }

    // This procedure is the same for all FATs
    fn verify_signature(&self) -> Option<BpbError> {
        let signature: [u8; 2] = self.raw.signature_word();
        if signature == &Self::SIGNATURE[..] {
            None
        } else {
            Some(BpbError::InvalidSignature(signature))
        }
    }

    // All following functions are fat32 only
    fn fat32_only(&self, field_name: &'static str) -> Result<(), BpbError> {
        if self.fat_type == FatType::Fat32 {
            Ok(())
        } else {
            Err(BpbError::Fat32Field(field_name))
        }
    }

    pub fn ext_flags(&self) -> Result<u16, BpbError> {
        self.fat32_only("ext_flags")?;
        let value = self.raw.ext_flags();
        Ok(value)
    }

    pub fn fs_version(&self) -> Result<u16, BpbError> {
        self.fat32_only("fs_version")?;
        let value = self.raw.fs_ver();
        if value == 0 {
            Ok(value)
        } else {
            Err(BpbError::Fat32(Fat32BpbError::FsVerNotZero))
        }
    }

    pub fn root_cluster(&self) -> Result<u32, BpbError> {
        self.fat32_only("root_cluster")?;
        let value = self.raw.root_clus();
        if value >= 2 {
            Ok(value)
        } else {
            Err(BpbError::Fat32(Fat32BpbError::RootClusterLessThanTwo))
        }
    }

    pub fn fs_info(&self) -> Result<u16, BpbError> {
        self.fat32_only("fs_info")?;
        let value = self.raw.fs_info();
        Ok(value)
    }

    pub fn bk_boot_sector(&self) -> Result<u16, BpbError> {
        self.fat32_only("bk_boot_sector")?;
        let value = self.raw.bk_boot_sec();
        if value == 0 || value == 6 {
            Ok(value)
        } else {
            Err(BpbError::Fat32(Fat32BpbError::InvalidBackupBootSector(
                value,
            )))
        }
    }
}

#[derive(Debug, Clone)]
pub struct BiosParameterBlockRaw {
    block: Block,
}

impl BiosParameterBlockRaw {
    fn data(&self) -> &[u8] {
        &self.block.contents
    }

    define_field!(bytes_per_sec, u16, 11);
    define_field!(sec_per_clu, u8, 13);
    define_field!(rsvd_sec_cnt, u16, 14);
    define_field!(num_fats, u8, 16);
    define_field!(root_entr_cnt, u16, 17);
    define_field!(tot_sec_16, u16, 19);
    define_field!(media, u8, 21);
    define_field!(fat_sz_16, u16, 22);
    define_field!(sectors_per_track, u16, 24);
    define_field!(number_of_heads, u16, 26);
    define_field!(hidden_sectors, u32, 28);
    define_field!(tot_sec_32, u32, 32);

    // FAT32 specific structure
    define_field!(fat_sz_32, u32, 36);
    define_field!(ext_flags, u16, 40);
    define_field!(fs_ver, u16, 42);
    define_field!(root_clus, u32, 44);
    define_field!(fs_info, u16, 48);
    define_field!(bk_boot_sec, u16, 50);

    fn signature_word(&self) -> [u8; 2] {
        let d = self.data();
        [d[510], d[511]]
    }
}
