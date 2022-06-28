use crate::{
    fat::{
        directory::{Attributes, ShortNameRaw},
        file::OpenMode,
        FatType, FatVolume,
    },
    mbr::{Mbr, PartitionInfo, PartitionNumber, PartitionType},
    BlockCount, BlockDevice, MemoryBlockDevice,
};

macro_rules! test_dir_entry {
    ($iter:expr, $name:literal, $attributes:expr, $file_size:literal, $first_cluster:literal) => {{
        let entry = $iter.next().expect("Expected another file to exist");
        let name_data = &mut ShortNameRaw::STR.clone();
        let name = entry.short_name().to_str(name_data);
        assert_eq!(name, Some($name));
        assert_eq!(entry.attributes(), &$attributes);
        assert_eq!(entry.file_size(), $file_size);
        assert_eq!(entry.first_cluster().entry().value(), $first_cluster);
    }};
}

fn test_subdir<BD>(volume: &mut FatVolume<BD>)
where
    BD: BlockDevice + std::fmt::Debug,
{
    let test_dir = volume
        .root_dir_iter()
        .find(|f| f.short_name().main_name_str() == Some("TEST"))
        .unwrap();

    let name_data = &mut ShortNameRaw::STR.clone();

    let mut subdir_iter = test_dir.iter_subdir(volume).unwrap();
    assert_eq!(
        subdir_iter.next().unwrap().short_name().to_str(name_data),
        Some(".")
    );
    assert_eq!(
        subdir_iter.next().unwrap().short_name().to_str(name_data),
        Some("..")
    );

    let test = subdir_iter.next().unwrap();

    assert_eq!(test.short_name().to_str(name_data), Some("TEST.DAT"));

    let long_name_entry = subdir_iter.next().unwrap();
    let long_name = long_name_entry.long_name();

    let str_data = &mut [0u8; 256];
    let long_name = long_name.to_str(str_data);

    assert_eq!(Some("a file with a much longer name"), long_name);

    assert!(subdir_iter.next().is_none());

    let mut opened_test_dat_file = volume.open_file(test, OpenMode::ReadOnly).unwrap();
    let mut opened_test_dat = opened_test_dat_file.activate(volume).unwrap();
    let data = &mut [0u8; 4096];
    let bytes = opened_test_dat.read(data).unwrap();
    assert_eq!(bytes, 3500);
    opened_test_dat.release();
    opened_test_dat_file.delete(volume).unwrap();

    let iter = test_dir.iter_subdir(volume).unwrap();
    assert_eq!(iter.count(), 3);
}

fn test_64mb<BD>(volume: &mut FatVolume<BD>)
where
    BD: BlockDevice + core::fmt::Debug,
{
    let dir_entry = volume
        .root_dir_iter()
        .find(|f| f.short_name().main_name_str() == Some("64MB"))
        .unwrap();

    let mut data = vec![0; dir_entry.file_size() as usize + 1024];

    let mut file = volume.open_file(dir_entry, OpenMode::ReadOnly).unwrap();

    let mut active_file = file.activate(volume).unwrap();

    let read_bytes = active_file.read(&mut data).unwrap();

    assert_eq!(
        read_bytes,
        active_file.file().dir_entry().file_size() as usize
    );

    volume.close_dir_entry(file.dir_entry());
    assert!(file.activate(volume).is_none());
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
    let mut fat16_volume = FatVolume::new(first_partition).unwrap();
    assert_eq!(fat16_volume.fat_type(), FatType::Fat16);

    let mut f16_iter = fat16_volume.root_dir_iter();
    test_dir_entry!(f16_iter, "README.TXT", Attributes::ARCHIVE, 258, 32778);
    test_dir_entry!(f16_iter, "EMPTY.DAT", Attributes::ARCHIVE, 0, 0);
    test_dir_entry!(f16_iter, "TEST", Attributes::DIRECTORY, 0, 5);
    test_dir_entry!(f16_iter, "64MB.DAT", Attributes::ARCHIVE, 67108864, 6);
    assert!(f16_iter.next().is_none());

    test_64mb(&mut fat16_volume);
    test_subdir(&mut fat16_volume);

    let second_partition = mbr.open_partition(PartitionNumber::Two).unwrap();
    let mut fat32_volume = FatVolume::new(second_partition).unwrap();
    assert_eq!(fat32_volume.fat_type(), FatType::Fat32);

    let mut iter = fat32_volume.root_dir_iter();
    test_dir_entry!(iter, "64MB.DAT", Attributes::ARCHIVE, 67108864, 3);
    test_dir_entry!(iter, "EMPTY.DAT", Attributes::ARCHIVE, 0, 0);
    test_dir_entry!(iter, "README.TXT", Attributes::ARCHIVE, 258, 16387);
    test_dir_entry!(iter, "TEST", Attributes::DIRECTORY, 0, 16388);
    assert!(iter.next().is_none());

    test_subdir(&mut fat32_volume);
    test_64mb(&mut fat32_volume);
}
