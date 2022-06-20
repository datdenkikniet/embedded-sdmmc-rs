use crate::Block;

#[derive(Debug)]
pub struct BlockByteCache {
    byte_index: usize,
    current_cache: Option<Block>,
    total_read: usize,
    total_len: Option<usize>,
}

impl BlockByteCache {
    pub fn new(total_len_bytes: Option<usize>) -> Self {
        Self {
            byte_index: 0,
            current_cache: None,
            total_read: 0,
            total_len: total_len_bytes,
        }
    }

    // Only call this if `all_cached_bytes_read`
    pub fn feed(&mut self, block: Block) {
        self.current_cache = Some(block);
        self.byte_index = 0;
    }

    pub fn all_cached_bytes_read(&self) -> bool {
        let read_all_bytes = if let Some(total_len) = self.total_len {
            (total_len - self.total_read) == 0
        } else {
            false
        };

        self.current_cache.is_none() || read_all_bytes
    }

    pub fn clear(&mut self) {
        self.byte_index = 0;
        self.total_read = 0;
        self.current_cache.take();
    }

    pub fn read(&mut self, data: &mut [u8]) -> usize {
        if let Some(cache) = &self.current_cache {
            let total_left = if let Some(total_len) = self.total_len {
                total_len - self.total_read
            } else {
                Block::LEN
            };

            let data_to_read = data.len().min(Block::LEN - self.byte_index).min(total_left);

            data[..data_to_read]
                .copy_from_slice(&cache.contents[self.byte_index..self.byte_index + data_to_read]);
            self.byte_index += data_to_read;
            self.total_read += data_to_read;

            if self.byte_index == Block::LEN {
                self.current_cache.take();
            }

            data_to_read
        } else {
            0
        }
    }
}
