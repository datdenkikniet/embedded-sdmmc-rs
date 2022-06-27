use crate::{Block, BlockDevice, BlockIdx, MemoryBlockDevice};

use super::bios_param_block::{BiosParameterBlock, BiosParameterBlockRaw};

extern crate std;

// Sets up a fake FAT16 file system that only contains a valid BPB, FAT and Root Directory,
// but no actual data
fn setup_fat16<BD>(bd: &mut BD)
where
    BD: BlockDevice,
{
    let mut bpb_block = Block::new();
    let mut raw = BiosParameterBlockRaw::new(&mut bpb_block);
    raw.set_bytes_per_sec(512);
    raw.set_sec_per_clu(1);
    raw.set_rsvd_sec_cnt(1);
    raw.set_num_fats(1);
    raw.set_root_ent_cnt(512 / 32);
    raw.set_tot_sec_16((bd.num_blocks().unwrap().0) as u16);
    raw.set_fat_sz_16(2);
    raw.set_media(0xFA);
    raw.set_signature(BiosParameterBlock::SIGNATURE);
    bd.write(&[bpb_block], BlockIdx(0)).unwrap();
}

#[test]
fn deallocate_file() {
    let mut memory = vec![0u8; 4096 * 512];
    let mut bd = MemoryBlockDevice::new(&mut memory);
    setup_fat16(&mut bd);

    let bpb = BiosParameterBlock::new(bd.read_block(BlockIdx(0)).unwrap());
    panic!("{:#?}", bpb);
}
