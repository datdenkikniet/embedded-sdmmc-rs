//! # embedded-sdmmc
//!
//! > An SD/MMC Library written in Embedded Rust
//!
//! This crate is intended to allow you to read/write files on a FAT formatted SD
//! card on your Rust Embedded device, as easily as using the `SdFat` Arduino
//! library. It is written in pure-Rust, is `#![no_std]` and does not use `alloc`
//! or `collections` to keep the memory footprint low. In the first instance it is
//! designed for readability and simplicity over performance.
//!
//! ## Using the crate
//!
//! You will need something that implements the `BlockDevice` trait, which can read and write the 512-byte blocks (or sectors) from your card. If you were to implement this over USB Mass Storage, there's no reason this crate couldn't work with a USB Thumb Drive, but we only supply a `BlockDevice` suitable for reading SD and SDHC cards over SPI.
//!
//! ```rust,ignore
//! # struct DummySpi;
//! # struct DummyCsPin;
//! # struct DummyUart;
//! # struct DummyTimeSource;
//! # impl embedded_hal::blocking::spi::Transfer<u8> for  DummySpi {
//! #   type Error = ();
//! #   fn transfer<'w>(&mut self, data: &'w mut [u8]) -> Result<&'w [u8], ()> { Ok(&[0]) }
//! # }
//! # impl embedded_hal::digital::v2::OutputPin for DummyCsPin {
//! #   type Error = ();
//! #   fn set_low(&mut self) -> Result<(), ()> { Ok(()) }
//! #   fn set_high(&mut self) -> Result<(), ()> { Ok(()) }
//! # }
//! # impl embedded_sdmmc::TimeSource for DummyTimeSource {
//! #   fn get_timestamp(&self) -> embedded_sdmmc::Timestamp { embedded_sdmmc::Timestamp::from_fat(0, 0) }
//! # }
//! # impl std::fmt::Write for DummyUart { fn write_str(&mut self, s: &str) -> std::fmt::Result { Ok(()) } }
//! # use std::fmt::Write;
//! # let mut uart = DummyUart;
//! # let mut sdmmc_spi = DummySpi;
//! # let mut sdmmc_cs = DummyCsPin;
//! # let time_source = DummyTimeSource;
//! let mut spi_dev = embedded_sdmmc::SdMmcSpi::new(sdmmc_spi, sdmmc_cs);
//! write!(uart, "Init SD card...").unwrap();
//! match spi_dev.acquire() {
//!     Ok(block) => {
//!         let mut cont = embedded_sdmmc::Controller::new(block, time_source);
//!         write!(uart, "OK!\nCard size...").unwrap();
//!         match cont.device().card_size_bytes() {
//!             Ok(size) => writeln!(uart, "{}", size).unwrap(),
//!             Err(e) => writeln!(uart, "Err: {:?}", e).unwrap(),
//!         }
//!         write!(uart, "Volume 0...").unwrap();
//!         match cont.get_volume(embedded_sdmmc::VolumeIdx(0)) {
//!             Ok(v) => writeln!(uart, "{:?}", v).unwrap(),
//!             Err(e) => writeln!(uart, "Err: {:?}", e).unwrap(),
//!         }
//!     }
//!     Err(e) => writeln!(uart, "{:?}!", e).unwrap(),
//! };
//! ```
//!
//! ## Features
//!
//! * `defmt-log`: By turning off the default features and enabling the `defmt-log` feature you can
//! configure this crate to log messages over defmt instead.
//!
//! Make sure that either the `log` feature or the `defmt-log` feature is enabled.

#![cfg_attr(not(test), no_std)]
// #![deny(missing_docs)]

// ****************************************************************************
//
// Imports
//
// ****************************************************************************

#[cfg(test)]
mod test;

#[cfg(feature = "defmt-log")]
use defmt::debug;

#[macro_use]
mod structure;

pub mod block_device;
pub mod mbr;
pub mod sdmmc;

pub mod fat;

pub use crate::block_device::{Block, BlockCount, BlockDevice, BlockIdx, MemoryBlockDevice};
pub use crate::sdmmc::Error as SdMmcError;
pub use crate::sdmmc::SdMmcSpi;

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
