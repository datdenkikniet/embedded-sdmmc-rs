//! embedded-sdmmc-rs - SDMMC Protocol
//!
//! Implements the SD/MMC protocol on some generic SPI interface.
//!
//! This is currently optimised for readability and debugability, not
//! performance.

mod busy;
use busy::SdMmcSpiBusy;

use core::fmt::Debug;

use super::sdmmc_proto::*;
use super::{Block, BlockCount, BlockDevice, BlockIdx};

use embedded_hal::blocking::spi::Transfer;
use embedded_hal::digital::v2::OutputPin;
#[cfg(feature = "log")]
use log::{debug, trace, warn};

#[cfg(feature = "defmt-log")]
use defmt::{debug, trace, warn};

const DEFAULT_DELAY_COUNT: u32 = 32_000;

/// Represents an inactive SD Card interface.
/// Built from an SPI peripheral and a Chip
/// Select pin. We need Chip Select to be separate so we can clock out some
/// bytes without Chip Select asserted (which puts the card into SPI mode).
pub struct SdMmcSpi<SPI, CS, State>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    card_type: CardType,
    spi: SPI,
    cs: CS,
    state: State,
}

/// The possible errors `SdMmcSpi` can generate.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, Copy, Clone)]
pub enum Error {
    /// We got an error from the SPI peripheral
    Transport,
    /// We failed to enable CRC checking on the SD card
    CantEnableCRC,
    /// We didn't get a response when reading data from the card
    TimeoutReadBuffer,
    /// We didn't get a response when waiting for the card to not be busy
    TimeoutWaitNotBusy,
    /// We didn't get a response when executing this command
    TimeoutCommand(u8),
    /// We didn't get a response when executing this application-specific command
    TimeoutACommand(u8),
    /// We got a bad response from Command 58
    Cmd58Error,
    /// We failed to read the Card Specific Data register
    RegisterReadError,
    /// We got a CRC mismatch (card gave us, we calculated)
    CrcError(u16, u16),
    /// Error reading from the card
    ReadError,
    /// Error writing to the card
    WriteError,
    /// Can't perform this operation with the card in this state
    BadState,
    /// Couldn't find the card
    CardNotFound,
    /// Couldn't set a GPIO pin
    GpioError,
}

/// The state of an SdMmcSpi if it is not initialized
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
pub struct NotInit;

/// The state of an SdMmcSpi if it is initialized
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
pub struct Initialized;

/// The different types of card we support.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug, Copy, Clone, PartialEq)]
enum CardType {
    SD1,
    SD2,
    SDHC,
}

/// A terrible hack for busy-waiting the CPU while we wait for the card to
/// sort itself out.
///
/// @TODO replace this!
struct Delay(u32);

impl Delay {
    fn new() -> Delay {
        Delay(DEFAULT_DELAY_COUNT)
    }

    fn delay(&mut self, err: Error) -> Result<(), Error> {
        if self.0 == 0 {
            Err(err)
        } else {
            let dummy_var: u32 = 0;
            for _ in 0..100 {
                unsafe { core::ptr::read_volatile(&dummy_var) };
            }
            self.0 -= 1;
            Ok(())
        }
    }
}

/// Options for acquiring the card.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
#[derive(Debug)]
pub struct AcquireOpts {
    /// Some cards don't support CRC mode. At least a 512MiB Transcend one.
    pub require_crc: bool,
}

impl Default for AcquireOpts {
    fn default() -> Self {
        AcquireOpts { require_crc: true }
    }
}

impl<SPI, CS> SdMmcSpi<SPI, CS, NotInit>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    /// Create a new SD/MMC controller using a raw SPI interface.
    pub fn new(spi: SPI, cs: CS) -> Self {
        SdMmcSpi {
            card_type: CardType::SD1,
            spi,
            cs,
            state: NotInit {},
        }
    }

    /// Initializes the card into a known state
    pub fn acquire(self) -> Result<SdMmcSpi<SPI, CS, Initialized>, (Error, Self)> {
        self.acquire_with_opts(Default::default())
    }

    fn discard_byte(&mut self) -> Result<u8, Error> {
        self.spi
            .transfer(&mut [0xFF])
            .map(|b| b[0])
            .map_err(|_e| Error::Transport)
    }

    /// Initializes the card into a known state
    pub fn acquire_with_opts(
        mut self,
        options: AcquireOpts,
    ) -> Result<SdMmcSpi<SPI, CS, Initialized>, (Error, Self)> {
        debug!("acquiring card with opts: {:?}", options);
        let f = |s: &mut Self| {
            trace!("Reset card..");

            // Supply minimum of 74 clock cycles without CS asserted.
            s.cs.set_high().map_err(|_| Error::GpioError)?;
            for _ in 0..10 {
                s.discard_byte()?;
            }

            let mut busy = SdMmcSpiBusy::new(&mut s.spi, &mut s.cs)?;

            // Enter SPI mode
            let mut delay = Delay::new();
            let mut attempts = 32;
            while attempts > 0 {
                trace!("Enter SPI mode, attempt: {}..", 32i32 - attempts);
                match busy.card_command(CMD0, 0) {
                    Err(Error::TimeoutCommand(0)) => {
                        // Try again?
                        warn!("Timed out, trying again..");
                        attempts -= 1;
                    }
                    Err(e) => {
                        return Err(e);
                    }
                    Ok(R1_IDLE_STATE) => {
                        break;
                    }
                    Ok(r) => {
                        // Try again
                        warn!("Got response: {:x}, trying again..", r);
                    }
                }

                delay.delay(Error::TimeoutCommand(CMD0))?;
            }
            if attempts == 0 {
                return Err(Error::CardNotFound);
            }
            // Enable CRC
            debug!("Enable CRC: {}", options.require_crc);
            if busy.card_command(CMD59, 1)? != R1_IDLE_STATE && options.require_crc {
                return Err(Error::CantEnableCRC);
            }
            // Check card version
            let mut delay = Delay::new();
            loop {
                if busy.card_command(CMD8, 0x1AA)? == (R1_ILLEGAL_COMMAND | R1_IDLE_STATE) {
                    s.card_type = CardType::SD1;
                    break;
                }
                busy.receive()?;
                busy.receive()?;
                busy.receive()?;
                let status = busy.receive()?;
                if status == 0xAA {
                    s.card_type = CardType::SD2;
                    break;
                }
                delay.delay(Error::TimeoutCommand(CMD8))?;
            }
            debug!("Card version: {:?}", s.card_type);

            let arg = match s.card_type {
                CardType::SD1 => 0,
                CardType::SD2 | CardType::SDHC => 0x4000_0000,
            };

            let mut delay = Delay::new();
            while busy.card_acmd(ACMD41, arg)? != R1_READY_STATE {
                delay.delay(Error::TimeoutACommand(ACMD41))?;
            }

            if s.card_type == CardType::SD2 {
                if busy.card_command(CMD58, 0)? != 0 {
                    return Err(Error::Cmd58Error);
                }
                if (busy.receive()? & 0xC0) == 0xC0 {
                    s.card_type = CardType::SDHC;
                }
                // Discard other three bytes
                busy.receive()?;
                busy.receive()?;
                busy.receive()?;
            }
            Ok(())
        };
        let result = f(&mut self);
        let _ = self.discard_byte();

        match result {
            Ok(_) => Ok(SdMmcSpi {
                card_type: self.card_type,
                spi: self.spi,
                cs: self.cs,
                state: Initialized {},
            }),
            Err(e) => Err((e, self)),
        }
    }
}

impl<SPI, CS> SdMmcSpi<SPI, CS, Initialized>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    /// Mark the card as unused.
    /// This should be kept infallible, because Drop is unable to fail.
    /// See https://github.com/rust-lang/rfcs/issues/814
    // If there is any need to flush data, it should be implemented here.
    pub fn deinit(self) -> SdMmcSpi<SPI, CS, NotInit> {
        SdMmcSpi {
            card_type: self.card_type,
            spi: self.spi,
            cs: self.cs,
            state: NotInit {},
        }
    }

    /// Run a command with chip select asserted.
    ///
    /// Chip select is always deasserted, even if an error occured in `f`
    fn with_chip_select<F, R>(&mut self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut SdMmcSpiBusy<SPI, CS>) -> Result<R, Error>,
    {
        let mut busy = SdMmcSpiBusy::new(&mut self.spi, &mut self.cs)?;
        let result = f(&mut busy);
        result
    }

    /// Read the 'card specific data' block.
    pub fn read_csd(&mut self) -> Result<Csd, Error> {
        let card_type = self.card_type;
        self.with_chip_select(|spi| match card_type {
            CardType::SD1 => {
                let mut csd = CsdV1::new();
                if spi.card_command(CMD9, 0)? != 0 {
                    return Err(Error::RegisterReadError);
                }
                spi.read_data(&mut csd.data)?;
                Ok(Csd::V1(csd))
            }
            CardType::SD2 | CardType::SDHC => {
                let mut csd = CsdV2::new();
                if spi.card_command(CMD9, 0)? != 0 {
                    return Err(Error::RegisterReadError);
                }
                spi.read_data(&mut csd.data)?;
                Ok(Csd::V2(csd))
            }
        })
    }

    /// Return the usable size of this SD card in bytes.
    pub fn card_size_bytes(&mut self) -> Result<u64, Error> {
        let csd = self.read_csd()?;
        match csd {
            Csd::V1(contents) => Ok(contents.card_capacity_bytes()),
            Csd::V2(contents) => Ok(contents.card_capacity_bytes()),
        }
    }

    /// Erase some blocks on the card.
    pub fn erase(&mut self, _first_block: BlockIdx, _last_block: BlockIdx) -> Result<(), Error> {
        unimplemented!();
    }

    /// Can this card erase single blocks?
    pub fn erase_single_block_enabled(&mut self) -> Result<bool, Error> {
        let csd = self.read_csd()?;
        match csd {
            Csd::V1(contents) => Ok(contents.erase_single_block_enabled()),
            Csd::V2(contents) => Ok(contents.erase_single_block_enabled()),
        }
    }
}

impl<SPI, CS> BlockDevice for SdMmcSpi<SPI, CS, Initialized>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    type Error = Error;

    /// Read one or more blocks, starting at the given block index.
    fn read(
        &mut self,
        blocks: &mut [Block],
        start_block_idx: BlockIdx,
        _reason: &str,
    ) -> Result<(), Self::Error> {
        let start_idx = match self.card_type {
            CardType::SD1 | CardType::SD2 => start_block_idx.0 * 512,
            CardType::SDHC => start_block_idx.0,
        };
        self.with_chip_select(|s| {
            if blocks.len() == 1 {
                // Start a single-block read
                s.card_command(CMD17, start_idx)?;
                s.read_data(&mut blocks[0].contents)?;
            } else {
                // Start a multi-block read
                s.card_command(CMD18, start_idx)?;
                for block in blocks.iter_mut() {
                    s.read_data(&mut block.contents)?;
                }
                // Stop the read
                s.card_command(CMD12, 0)?;
            }
            Ok(())
        })
    }

    /// Write one or more blocks, starting at the given block index.
    fn write(&mut self, blocks: &[Block], start_block_idx: BlockIdx) -> Result<(), Self::Error> {
        let start_idx = match self.card_type {
            CardType::SD1 | CardType::SD2 => start_block_idx.0 * 512,
            CardType::SDHC => start_block_idx.0,
        };
        self.with_chip_select(|s| {
            if blocks.len() == 1 {
                // Start a single-block write
                s.card_command(CMD24, start_idx)?;
                s.write_data(DATA_START_BLOCK, &blocks[0].contents)?;
                s.wait_not_busy()?;
                if s.card_command(CMD13, 0)? != 0x00 {
                    return Err(Error::WriteError);
                }
                if s.receive()? != 0x00 {
                    return Err(Error::WriteError);
                }
            } else {
                // Start a multi-block write
                s.card_command(CMD25, start_idx)?;
                for block in blocks.iter() {
                    s.wait_not_busy()?;
                    s.write_data(WRITE_MULTIPLE_TOKEN, &block.contents)?;
                }
                // Stop the write
                s.wait_not_busy()?;
                s.send(STOP_TRAN_TOKEN)?;
            }
            Ok(())
        })
    }

    /// Determine how many blocks this device can hold.
    fn num_blocks(&mut self) -> Result<BlockCount, Self::Error> {
        let num_bytes = self.card_size_bytes()?;
        let num_blocks = (num_bytes / 512) as u32;
        Ok(BlockCount(num_blocks))
    }
}

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
