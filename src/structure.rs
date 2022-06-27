//! embedded-sdmmc-rs - Useful macros for parsing SD/MMC structures.

macro_rules! access_field {
    ($self:expr, $offset:expr, $start_bit:expr, 1) => {
        ($self.data()[$offset] & (1 << $start_bit)) != 0
    };
    ($self:expr, $offset:expr, $start:expr, $num_bits:expr) => {
        ($self.data()[$offset] >> $start) & (((1u16 << $num_bits) - 1) as u8)
    };
}

macro_rules! define_field {
    ($name:ident, bool, $offset:expr, $bit:expr) => {
        /// Get the value from the $name field
        pub fn $name(&self) -> bool {
            access_field!(self, $offset, $bit, 1)
        }
    };
    ($name:ident, u8, $offset:expr, $start_bit:expr, $num_bits:expr) => {
        /// Get the value from the $name field
        pub fn $name(&self) -> u8 {
            access_field!(self, $offset, $start_bit, $num_bits)
        }
    };
    ($name:ident, $type:ty, [ $( ( $offset:expr, $start_bit:expr, $num_bits:expr ) ),+ ]) => {
        /// Gets the value from the $name field
        pub fn $name(&self) -> $type {
            let mut result = 0;
            $(
                    result <<= $num_bits;
                    let part = access_field!(self, $offset, $start_bit, $num_bits) as $type;
                    result |=  part;
            )+
            result
        }
    };

    ($name:ident, $set_name:ident, u8, $offset:expr) => {
        doc_comment::doc_comment! {
            concat!("Get the value of the ", stringify!($name), " field"),
            pub fn $name(&self) -> u8 {
                self.data()[$offset]
            }
        }

        doc_comment::doc_comment! {
            concat!("Set the value of the ", stringify!($name), " field"),
            pub fn $set_name(&mut self, value: u8) {
                self.data_mut()[$offset] = value;
            }
        }

    };

    ($name:ident, $set_name:ident, u16, $offset:expr) => {
        doc_comment::doc_comment! {
            concat!("Get the value of the ", stringify!($name), " field"),
            pub fn $name(&self) -> u16 {
                use core::convert::TryInto;
                u16::from_le_bytes(self.data()[$offset..$offset + 2].try_into().expect("Infallible"))
            }
        }

        doc_comment::doc_comment! {
            concat!("Set the value of the ", stringify!($name), " field"),
            pub fn $set_name(&mut self, value: u16) {
                self.data_mut()[$offset..$offset+2].copy_from_slice(&value.to_le_bytes());
            }
        }
    };

    ($name:ident, $set_name:ident, u32, $offset:expr) => {
        doc_comment::doc_comment! {
            concat!("Get the value of the ", stringify!($name), " field"),
            pub fn $name(&self) -> u32 {
                use core::convert::TryInto;
                u32::from_le_bytes(self.data()[$offset..$offset + 4].try_into().expect("Infallible"))
            }
        }

        doc_comment::doc_comment! {
            concat!("Set the value of the ", stringify!($name), " field"),
            pub fn $set_name(&self) -> u32 {
                use core::convert::TryInto;
                u32::from_le_bytes(self.data()[$offset..$offset + 4].try_into().expect("Infallible"))
            }
        }
    };
}

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
