use crate::{
    fat::{cluster::Cluster, Attributes, FatType, FatVolume, File, SectorIter},
    mbr::{Mbr, PartitionInfo, PartitionNumber, PartitionType},
    BlockCount, BlockDevice, BlockIdx, MemoryBlockDevice,
};

macro_rules! test_dir_entry {
    ($entry:expr, $name:literal, $extension:literal, $attributes:expr, $file_size:literal, $first_cluster:literal) => {{
        unsafe {
            println!(
                "File: {}.{}. Size: {} bytes",
                $entry.name().main_name_str(),
                $entry.name().extension_str(),
                $entry.file_size()
            );
            assert_eq!($entry.name().main_name_str(), $name);
            assert_eq!($entry.name().extension_str(), $extension);
        }
        assert_eq!($entry.attributes(), &$attributes);
        assert_eq!($entry.file_size(), $file_size);
        assert_eq!($entry.first_cluster(), &Cluster($first_cluster));
    }};
}

fn test_first_partition<BD>(partition: BD)
where
    BD: BlockDevice,
{
    let mut fat16_volume = FatVolume::new(partition).unwrap();
    assert_eq!(fat16_volume.fat_type(), FatType::Fat16);

    let mut f16_iter = fat16_volume.root_directory_iter();

    let mut idx = 0;
    let mut readme_dir_entry = None;
    for dir_entry in &mut f16_iter {
        match idx {
            0 => {
                test_dir_entry!(dir_entry, "README", "TXT", Attributes::ARCHIVE, 258, 17473);
                readme_dir_entry = Some(dir_entry);
            }
            1 => test_dir_entry!(dir_entry, "EMPTY", "DAT", Attributes::ARCHIVE, 0, 21584),
            2 => test_dir_entry!(dir_entry, "TEST", "", Attributes::DIRECTORY, 0, 21587),
            3 => test_dir_entry!(
                dir_entry,
                "64MB",
                "DAT",
                Attributes::ARCHIVE,
                67108864,
                16973
            ),
            _ => unreachable!(),
        };

        idx += 1;
    }
}

fn test_second_partition<BD>(partition: BD)
where
    BD: BlockDevice,
{
    let mut fat32_volume = FatVolume::new(partition).unwrap();
    assert_eq!(fat32_volume.fat_type(), FatType::Fat32);

    let mut f32_iter = fat32_volume.root_directory_iter();

    let mut idx = 0;
    for dir_entry in &mut f32_iter {
        match idx {
            0 => test_dir_entry!(
                dir_entry,
                "64MB",
                "DAT",
                Attributes::ARCHIVE,
                67108864,
                16973
            ),
            1 => test_dir_entry!(dir_entry, "EMPTY", "DAT", Attributes::ARCHIVE, 0, 21584),
            2 => test_dir_entry!(dir_entry, "README", "TXT", Attributes::ARCHIVE, 258, 17473),
            3 => test_dir_entry!(dir_entry, "TEST", "", Attributes::DIRECTORY, 0, 21587),
            _ => unreachable!(),
        };

        idx += 1;
    }
}

#[test]
fn read_disk_file() {
    let mut data = std::fs::read("disk.img").unwrap();

    let block_device = MemoryBlockDevice::new(&mut data);

    let mut mbr = Mbr::new(block_device).unwrap();

    assert_eq!(
        mbr.get_partition_info(PartitionNumber::One),
        Some(PartitionInfo {
            ty: PartitionType::Fat16Lba,
            lba_start: BlockCount(2048),
            block_count: BlockCount(262144),
        })
    );

    assert_eq!(
        mbr.get_partition_info(PartitionNumber::Two),
        Some(PartitionInfo {
            ty: PartitionType::Fat32Lba,
            lba_start: BlockCount(264192),
            block_count: BlockCount(784384),
        })
    );

    let first_partition = mbr.open_partition(PartitionNumber::One).unwrap();
    // test_first_partition(first_partition);

    let second_partition = mbr.open_partition(PartitionNumber::Two).unwrap();
    // test_second_partition(second_partition);

    let data1 = &mut std::fs::read("disk.img1").unwrap();
    let mut data1 = MemoryBlockDevice::new(data1);

    for idx in BlockIdx(0).range(BlockCount(0xFFFF)) {
        let block = data1.read_block(idx).unwrap();
        if block.contents[0] != 0 {
            println!("{:?}: {:?}", idx, block);
        }
    }

    let mut fat_volume = FatVolume::new(data1).unwrap();

    // Dir_entry first_cluster.sectors().first() should be BlockIdx(552)
    let dir_entry = fat_volume
        .root_directory_iter()
        .find(|f| unsafe { f.name().main_name_str() == "README" })
        .unwrap();

    panic!("{:?}", dir_entry);

    panic!(
        "{:?}",
        fat_volume
            .all_sectors(dir_entry.first_cluster().clone())
            .next(&mut fat_volume)
    );

    let mut file = File::new(dir_entry);

    let data = &mut [0u8; 300];
    let data_len = file.read_all(&mut fat_volume, data).unwrap();
    panic!("{:?}", &data[..data_len]);
}
