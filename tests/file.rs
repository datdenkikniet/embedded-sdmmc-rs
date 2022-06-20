use embedded_sdmmc::{
    fat::{FatVolume, SectorIter},
    mbr::{Mbr, PartitionInfo, PartitionNumber, PartitionType},
    BlockCount, MemoryBlockDevice,
};

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

    let mut fat_volume = FatVolume::new(first_partition).unwrap();

    let mut iter = fat_volume.root_directory_iter();

    for dir_entry in &mut iter {
        println!("{:X?}", dir_entry);
        println!("{}", dir_entry.name().main_name());
    }
    println!("{}", iter.total_entries_read());
}
