use crate::gpio::*;
use crate::rcc::{self, Rcc};
use crate::stm32::{self as pac, spi1};
use crate::time::Hertz;
use core::convert::Infallible;
use core::ptr;
use embedded_hal::delay::DelayNs;
use hal::digital;
use hal::digital::OutputPin;
pub use hal::spi::{
    self, ErrorKind, ErrorType, Mode, Phase, Polarity, MODE_0, MODE_1, MODE_2, MODE_3,
};
use nb::block;

/// SPI error
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Overrun occurred
    Overrun,
    /// Mode fault occurred
    ModeFault,
    /// CRC error
    Crc,
    /// Chip Select Fault
    ChipSelectFault,
}

impl hal::spi::Error for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::Overrun => ErrorKind::Overrun,
            Error::ModeFault => ErrorKind::ModeFault,
            Error::ChipSelectFault => ErrorKind::ChipSelectFault,
            Error::Crc => ErrorKind::Other,
        }
    }
}

pub trait Instance:
    crate::Sealed + core::ops::Deref<Target = spi1::RegisterBlock> + rcc::Enable + rcc::Reset
{
}

/// A filler type for when the delay is unnecessary
pub struct NoDelay;

impl DelayNs for NoDelay {
    fn delay_ns(&mut self, _: u32) {}
}

/// A filler type for when the CS pin is unnecessary
pub struct NoCS;

impl digital::ErrorType for NoCS {
    type Error = Infallible;
}

impl digital::OutputPin for NoCS {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// A filler type for when the SCK pin is unnecessary
pub struct NoSck;
/// A filler type for when the Miso pin is unnecessary
pub struct NoMiso;
/// A filler type for when the Mosi pin is unnecessary
pub struct NoMosi;

pub trait Pins<SPI> {
    fn setup(&self);
    fn release(self) -> Self;
}

pub trait PinSck<SPI> {
    fn setup(&self);
    fn release(self) -> Self;
}

pub trait PinMiso<SPI> {
    fn setup(&self);
    fn release(self) -> Self;
}

pub trait PinMosi<SPI> {
    fn setup(&self);
    fn release(self) -> Self;
}

impl<SPI, SCK, MISO, MOSI> Pins<SPI> for (SCK, MISO, MOSI)
where
    SCK: PinSck<SPI>,
    MISO: PinMiso<SPI>,
    MOSI: PinMosi<SPI>,
{
    fn setup(&self) {
        self.0.setup();
        self.1.setup();
        self.2.setup();
    }

    fn release(self) -> Self {
        (self.0.release(), self.1.release(), self.2.release())
    }
}

#[derive(Debug)]
pub struct SpiBus<SPI, PINS> {
    spi: SPI,
    pins: PINS,
}

#[derive(Debug)]
pub struct SpiDevice<BUS, CS, DELAY> {
    bus: BUS,
    cs: CS,
    delay: DELAY,
}

pub trait SpiExt: Sized {
    fn spi<PINS>(self, pins: PINS, mode: Mode, freq: Hertz, rcc: &mut Rcc) -> SpiBus<Self, PINS>
    where
        PINS: Pins<Self>;
}

macro_rules! spi {
    ($SPIX:ty,
        sck: [ $(($SCK:ty, $SCK_AF:expr),)+ ],
        miso: [ $(($MISO:ty, $MISO_AF:expr),)+ ],
        mosi: [ $(($MOSI:ty, $MOSI_AF:expr),)+ ],
    ) => {
        impl Instance for $SPIX {}

        impl PinSck<$SPIX> for NoSck {
            fn setup(&self) {}

            fn release(self) -> Self {
                self
            }
        }

        impl PinMiso<$SPIX> for NoMiso {
            fn setup(&self) {}

            fn release(self) -> Self {
                self
            }
        }

        impl PinMosi<$SPIX> for NoMosi {
            fn setup(&self) {}

            fn release(self) -> Self {
                self
            }
        }

        $(
            impl PinSck<$SPIX> for $SCK {
                fn setup(&self) {
                    self.set_alt_mode($SCK_AF);
                }

                fn release(self) -> Self {
                    self.into_analog()
                }
            }
        )*
        $(
            impl PinMiso<$SPIX> for $MISO {
                fn setup(&self) {
                    self.set_alt_mode($MISO_AF);
                }

                fn release(self) -> Self {
                    self.into_analog()
                }
            }
        )*
        $(
            impl PinMosi<$SPIX> for $MOSI {
                fn setup(&self) {
                    self.set_alt_mode($MOSI_AF);
                }

                fn release(self) -> Self {
                    self.into_analog()
                }
            }
        )*
    }
}

impl<SPI: Instance, PINS: Pins<SPI>> SpiBus<SPI, PINS> {
    pub fn new(spi: SPI, pins: PINS, mode: Mode, speed: Hertz, rcc: &mut Rcc) -> Self {
        SPI::enable(rcc);
        SPI::reset(rcc);

        // disable SS output
        spi.cr2().write(|w| w.ssoe().clear_bit());

        let br = match rcc.clocks.apb_clk / speed {
            0 => unreachable!(),
            1..=2 => 0b000,
            3..=5 => 0b001,
            6..=11 => 0b010,
            12..=23 => 0b011,
            24..=47 => 0b100,
            48..=95 => 0b101,
            96..=191 => 0b110,
            _ => 0b111,
        };

        spi.cr2()
            .write(|w| unsafe { w.frxth().set_bit().ds().bits(0b111).ssoe().clear_bit() });

        // Enable pins
        pins.setup();

        spi.cr1().write(|w| {
            w.cpha().bit(mode.phase == Phase::CaptureOnSecondTransition);
            w.cpol().bit(mode.polarity == Polarity::IdleHigh);
            w.mstr().set_bit();
            w.br().set(br);
            w.lsbfirst().clear_bit();
            w.ssm().set_bit();
            w.ssi().set_bit();
            w.rxonly().clear_bit();
            w.crcl().clear_bit();
            w.bidimode().clear_bit();
            w.spe().set_bit()
        });

        SpiBus { spi, pins }
    }

    pub fn exclusive<CS: OutputPin, DELAY: DelayNs>(
        self,
        cs: CS,
        delay: DELAY,
    ) -> SpiDevice<SpiBus<SPI, PINS>, CS, DELAY> {
        SpiDevice {
            bus: self,
            cs,
            delay,
        }
    }

    pub fn data_size(&mut self, nr_bits: u8) {
        self.spi
            .cr2()
            .modify(|_, w| unsafe { w.ds().bits(nr_bits - 1) });
    }

    pub fn half_duplex_enable(&mut self, enable: bool) {
        self.spi.cr1().modify(|_, w| w.bidimode().bit(enable));
    }

    pub fn half_duplex_output_enable(&mut self, enable: bool) {
        self.spi.cr1().modify(|_, w| w.bidioe().bit(enable));
    }

    pub fn release(self) -> (SPI, PINS) {
        (self.spi, self.pins.release())
    }
}

impl<SPI: Instance, PINS, CS: OutputPin, DELAY> ErrorType
    for SpiDevice<SpiBus<SPI, PINS>, CS, DELAY>
{
    type Error = Error;
}
impl<SPI: Instance, PINS, CS: OutputPin, DELAY: DelayNs> spi::SpiDevice
    for SpiDevice<SpiBus<SPI, PINS>, CS, DELAY>
{
    fn transaction(&mut self, operations: &mut [hal::spi::Operation<'_, u8>]) -> Result<(), Error> {
        use crate::hal::spi::SpiBus;
        self.cs.set_low().map_err(|_| Error::ChipSelectFault)?;
        for op in operations {
            match op {
                spi::Operation::Read(read) => {
                    self.bus.read(read)?;
                }
                spi::Operation::Write(write) => {
                    self.bus.write(write)?;
                }
                spi::Operation::Transfer(write, read) => {
                    self.bus.transfer(write, read)?;
                }
                spi::Operation::TransferInPlace(data) => {
                    self.bus.transfer_in_place(data)?;
                }
                spi::Operation::DelayNs(ns) => self.delay.delay_ns(*ns),
            }
        }
        self.cs.set_high().map_err(|_| Error::ChipSelectFault)?;
        Ok(())
    }
}

impl<SPI: Instance> SpiExt for SPI {
    fn spi<PINS>(self, pins: PINS, mode: Mode, freq: Hertz, rcc: &mut Rcc) -> SpiBus<SPI, PINS>
    where
        PINS: Pins<SPI>,
    {
        SpiBus::new(self, pins, mode, freq, rcc)
    }
}

impl<SPI: Instance, PINS> SpiBus<SPI, PINS> {
    fn receive_byte(&mut self) -> nb::Result<u8, Error> {
        let sr = self.spi.sr().read();

        Err(if sr.ovr().bit_is_set() {
            nb::Error::Other(Error::Overrun)
        } else if sr.modf().bit_is_set() {
            nb::Error::Other(Error::ModeFault)
        } else if sr.crcerr().bit_is_set() {
            nb::Error::Other(Error::Crc)
        } else if sr.rxne().bit_is_set() {
            // NOTE(read_volatile) read only 1 byte (the svd2rust API only allows
            // reading a half-word)
            return Ok(unsafe { ptr::read_volatile(&self.spi.dr() as *const _ as *const u8) });
        } else {
            nb::Error::WouldBlock
        })
    }

    fn send_byte(&mut self, byte: u8) -> nb::Result<(), Error> {
        let sr = self.spi.sr().read();
        Err(if sr.ovr().bit_is_set() {
            nb::Error::Other(Error::Overrun)
        } else if sr.modf().bit_is_set() {
            nb::Error::Other(Error::ModeFault)
        } else if sr.crcerr().bit_is_set() {
            nb::Error::Other(Error::Crc)
        } else if sr.txe().bit_is_set() {
            self.spi.dr().write(|w| w.dr().set(byte.into()));
            return Ok(());
        } else {
            nb::Error::WouldBlock
        })
    }
}

impl<SPI: Instance, PINS> ErrorType for SpiBus<SPI, PINS> {
    type Error = Error;
}

impl<SPI: Instance, PINS> spi::SpiBus for SpiBus<SPI, PINS> {
    fn read(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
        for byte in bytes.iter_mut() {
            block!(self.send_byte(0))?;
            *byte = block!(self.receive_byte())?;
        }
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        for byte in bytes.iter() {
            block!(self.send_byte(*byte))?;
            block!(self.receive_byte())?;
        }
        Ok(())
    }

    fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        let mut iter_r = read.iter_mut();
        let mut iter_w = write.iter().cloned();
        loop {
            match (iter_r.next(), iter_w.next()) {
                (Some(r), Some(w)) => {
                    block!(self.send_byte(w))?;
                    *r = block!(self.receive_byte())?;
                }
                (Some(r), None) => {
                    block!(self.send_byte(0))?;
                    *r = block!(self.receive_byte())?;
                }
                (None, Some(w)) => {
                    block!(self.send_byte(w))?;
                    let _ = block!(self.receive_byte())?;
                }
                (None, None) => return Ok(()),
            }
        }
    }

    fn transfer_in_place(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
        for byte in bytes.iter_mut() {
            block!(self.send_byte(*byte))?;
            *byte = block!(self.receive_byte())?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

spi!(
    pac::SPI1,
    sck: [
        (PA1<DefaultMode>, AltFunction::AF0),
        (PA5<DefaultMode>, AltFunction::AF0),
        (PB3<DefaultMode>, AltFunction::AF0),
        (PD8<DefaultMode>, AltFunction::AF1),
    ],
    miso: [
        (PA6<DefaultMode>, AltFunction::AF0),
        (PA11<DefaultMode>, AltFunction::AF0),
        (PB4<DefaultMode>, AltFunction::AF0),
        (PD5<DefaultMode>, AltFunction::AF1),
    ],
    mosi: [
        (PA2<DefaultMode>, AltFunction::AF0),
        (PA7<DefaultMode>, AltFunction::AF0),
        (PA12<DefaultMode>, AltFunction::AF0),
        (PB5<DefaultMode>, AltFunction::AF0),
        (PD6<DefaultMode>, AltFunction::AF1),
    ],
);

spi!(
    pac::SPI2,
    sck: [
        (PA0<DefaultMode>, AltFunction::AF0),
        (PB8<DefaultMode>, AltFunction::AF1),
        (PB10<DefaultMode>, AltFunction::AF5),
        (PB13<DefaultMode>, AltFunction::AF0),
        (PD1<DefaultMode>, AltFunction::AF1),
    ],
    miso: [
        (PA3<DefaultMode>, AltFunction::AF0),
        (PA9<DefaultMode>, AltFunction::AF4),
        (PB2<DefaultMode>, AltFunction::AF1),
        (PB6<DefaultMode>, AltFunction::AF4),
        (PB14<DefaultMode>, AltFunction::AF0),
        (PC2<DefaultMode>, AltFunction::AF1),
        (PD3<DefaultMode>, AltFunction::AF1),
    ],
    mosi: [
        (PA4<DefaultMode>, AltFunction::AF1),
        (PA10<DefaultMode>, AltFunction::AF0),
        (PB7<DefaultMode>, AltFunction::AF1),
        (PB11<DefaultMode>, AltFunction::AF0),
        (PB15<DefaultMode>, AltFunction::AF0),
        (PC3<DefaultMode>, AltFunction::AF1),
        (PD4<DefaultMode>, AltFunction::AF1),
    ],
);
