use crate::{
    fat::{cluster::Cluster, directory::Attributes, FatType, FatVolume},
    mbr::{Mbr, PartitionInfo, PartitionNumber, PartitionType},
    BlockCount, BlockDevice, MemoryBlockDevice,
};

macro_rules! test_dir_entry {
    ($iter:expr, $name:literal, $extension:literal, $attributes:expr, $file_size:literal, $first_cluster:literal) => {{
        let entry = $iter.next().expect("Expected another file to exist");
        unsafe {
            assert_eq!(entry.short_name().main_name_str(), $name);
            assert_eq!(entry.short_name().extension_str(), $extension);
        }
        assert_eq!(entry.attributes(), &$attributes);
        assert_eq!(entry.file_size(), $file_size);
        assert_eq!(entry.first_cluster(), &Cluster($first_cluster));
    }};
}

fn test_subdir<BD>(volume: &mut FatVolume<BD>)
where
    BD: BlockDevice + std::fmt::Debug,
{
    let test_dir = volume
        .root_directory_iter()
        .find(|f| unsafe { f.short_name().main_name_str() } == "TEST")
        .unwrap();

    let mut subdir_iter = test_dir.iter_subdir(volume).unwrap();
    assert_eq!(
        unsafe { subdir_iter.next().unwrap().short_name().main_name_str() },
        "."
    );
    assert_eq!(
        unsafe { subdir_iter.next().unwrap().short_name().main_name_str() },
        ".."
    );
    assert_eq!(
        unsafe { subdir_iter.next().unwrap().short_name().main_name_str() },
        "TEST"
    );

    let long_name_entry = subdir_iter.next().unwrap();
    let long_name = long_name_entry.long_name();
    let name_len = long_name.len();

    assert_eq!(
        &[
            'a', ' ', 'f', 'i', 'l', 'e', ' ', 'w', 'i', 't', 'h', ' ', 'a', ' ', 'm', 'u', 'c',
            'h', ' ', 'l', 'o', 'n', 'g', 'e', 'r', ' ', 'n', 'a', 'm', 'e'
        ],
        &long_name.chars()[..name_len]
    );

    assert!(subdir_iter.next().is_none());
}

fn test_64mb<BD>(volume: &mut FatVolume<BD>)
where
    BD: BlockDevice,
{
    let file = volume
        .root_directory_iter()
        .find(|f| unsafe { f.short_name().main_name_str() } == "64MB")
        .unwrap();

    let mut data = vec![0; file.file_size() as usize + 1024];

    let mut file = volume.open_file(file).unwrap();
    let mut file = file.activate(volume);

    let read_bytes = file.read(&mut data).unwrap();

    assert_eq!(read_bytes, file.file().dir_entry().file_size() as usize);
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

    let mut f16_iter = fat16_volume.root_directory_iter();
    test_dir_entry!(f16_iter, "README", "TXT", Attributes::ARCHIVE, 258, 32778);
    test_dir_entry!(f16_iter, "EMPTY", "DAT", Attributes::ARCHIVE, 0, 0);
    test_dir_entry!(f16_iter, "TEST", "", Attributes::DIRECTORY, 0, 5);
    test_dir_entry!(f16_iter, "64MB", "DAT", Attributes::ARCHIVE, 67108864, 6);
    assert!(f16_iter.next().is_none());

    test_subdir(&mut fat16_volume);
    test_64mb(&mut fat16_volume);

    let second_partition = mbr.open_partition(PartitionNumber::Two).unwrap();
    let mut fat32_volume = FatVolume::new(second_partition).unwrap();
    assert_eq!(fat32_volume.fat_type(), FatType::Fat32);

    let mut iter = fat32_volume.root_directory_iter();
    test_dir_entry!(iter, "64MB", "DAT", Attributes::ARCHIVE, 67108864, 3);
    test_dir_entry!(iter, "EMPTY", "DAT", Attributes::ARCHIVE, 0, 0);
    test_dir_entry!(iter, "README", "TXT", Attributes::ARCHIVE, 258, 16387);
    test_dir_entry!(iter, "TEST", "", Attributes::DIRECTORY, 0, 16388);
    assert!(iter.next().is_none());

    test_subdir(&mut fat32_volume);
    test_64mb(&mut fat32_volume);
}
