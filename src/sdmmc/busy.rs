use embedded_hal::{blocking::spi::Transfer, digital::v2::OutputPin};

use crate::sdmmc_proto::*;

use super::{Delay, Error};

/// A struct used to ensure that communication only occurs
/// when CS is low.
///
/// This struct is responsible for ensuring that all SPI, CRC, and
/// other communication-layer functionalities are performed correctly.
#[cfg_attr(feature = "defmt-log", derive(defmt::Format))]
pub struct SdMmcSpiBusy<'spi, 'cs, SPI, CS>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    spi: &'spi mut SPI,
    cs: &'cs mut CS,
}

impl<'spi, 'cs, SPI, CS> Drop for SdMmcSpiBusy<'spi, 'cs, SPI, CS>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    fn drop(&mut self) {
        self.cs_high().ok();
    }
}

impl<'spi, 'cs, SPI, CS> SdMmcSpiBusy<'spi, 'cs, SPI, CS>
where
    SPI: Transfer<u8>,
    CS: OutputPin,
{
    pub fn new(spi: &'spi mut SPI, cs: &'cs mut CS) -> Result<Self, Error> {
        let mut me = Self { spi, cs };
        me.cs_low()?;
        Ok(me)
    }

    fn cs_high(&mut self) -> Result<(), Error> {
        self.cs.set_high().map_err(|_| Error::GpioError)
    }

    fn cs_low(&mut self) -> Result<(), Error> {
        self.cs.set_low().map_err(|_| Error::GpioError)
    }

    /// Send one byte and receive one byte.
    fn transfer(&mut self, out: u8) -> Result<u8, Error> {
        self.spi
            .transfer(&mut [out])
            .map(|b| b[0])
            .map_err(|_e| Error::Transport)
    }

    /// Receive a byte from the SD card by clocking in an 0xFF byte.
    pub fn receive(&mut self) -> Result<u8, Error> {
        self.transfer(0xFF)
    }

    /// Send a byte from the SD card.
    pub fn send(&mut self, out: u8) -> Result<(), Error> {
        let _ = self.transfer(out)?;
        Ok(())
    }

    /// Spin until the card returns 0xFF, or we spin too many times and
    /// timeout.
    pub fn wait_not_busy(&mut self) -> Result<(), Error> {
        let mut delay = Delay::new();
        loop {
            let s = self.receive()?;
            if s == 0xFF {
                break;
            }
            delay.delay(Error::TimeoutWaitNotBusy)?;
        }
        Ok(())
    }

    /// Perform a command.
    pub fn card_command(&mut self, command: u8, arg: u32) -> Result<u8, Error> {
        self.wait_not_busy()?;
        let mut buf = [
            0x40 | command,
            (arg >> 24) as u8,
            (arg >> 16) as u8,
            (arg >> 8) as u8,
            arg as u8,
            0,
        ];
        buf[5] = crc7(&buf[0..5]);

        for b in buf.iter() {
            self.send(*b)?;
        }

        // skip stuff byte for stop read
        if command == CMD12 {
            let _result = self.receive()?;
        }

        for _ in 0..512 {
            let result = self.receive()?;
            if (result & 0x80) == ERROR_OK {
                return Ok(result);
            }
        }

        Err(Error::TimeoutCommand(command))
    }

    /// Perform an application-specific command.
    pub fn card_acmd(&mut self, command: u8, arg: u32) -> Result<u8, Error> {
        self.card_command(CMD55, 0)?;
        self.card_command(command, arg)
    }

    /// Read an arbitrary number of bytes from the card. Always fills the
    /// given buffer, so make sure it's the right size.
    pub fn read_data(&mut self, buffer: &mut [u8]) -> Result<(), Error> {
        // Get first non-FF byte.
        let mut delay = Delay::new();
        let status = loop {
            let s = self.receive()?;
            if s != 0xFF {
                break s;
            }
            delay.delay(Error::TimeoutReadBuffer)?;
        };
        if status != DATA_START_BLOCK {
            return Err(Error::ReadError);
        }

        for b in buffer.iter_mut() {
            *b = self.receive()?;
        }

        let mut crc = u16::from(self.receive()?);
        crc <<= 8;
        crc |= u16::from(self.receive()?);

        let calc_crc = crc16(buffer);
        if crc != calc_crc {
            return Err(Error::CrcError(crc, calc_crc));
        }

        Ok(())
    }

    /// Write an arbitrary number of bytes to the card.
    pub fn write_data(&mut self, token: u8, buffer: &[u8]) -> Result<(), Error> {
        let calc_crc = crc16(buffer);
        self.send(token)?;
        for &b in buffer.iter() {
            self.send(b)?;
        }
        self.send((calc_crc >> 8) as u8)?;
        self.send(calc_crc as u8)?;
        let status = self.receive()?;
        if (status & DATA_RES_MASK) != DATA_RES_ACCEPTED {
            Err(Error::WriteError)
        } else {
            Ok(())
        }
    }
}
