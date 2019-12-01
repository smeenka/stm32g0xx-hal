//! Timers
use cortex_m::peripheral::syst::SystClkSource;
use cortex_m::peripheral::SYST;
use hal::timer::{CountDown, Periodic};
use nb;
use void::Void;

use crate::rcc::Rcc;
use crate::stm32::*;
use crate::time::{Hertz, MicroSecond};

pub mod opm;
pub mod pwm;
pub mod qei;
pub mod stopwatch;
pub mod pins;

/// Hardware timers
pub struct Timer<TIM> {
    clk: Hertz,
    tim: TIM,
}

pub struct Channel1;
pub struct Channel2;
pub struct Channel3;
pub struct Channel4;

/// System timer
impl Timer<SYST> {
    /// Configures the SYST clock as a periodic count down timer
    pub fn syst(mut syst: SYST, rcc: &mut Rcc) -> Self {
        syst.set_clock_source(SystClkSource::Core);
        Timer {
            tim: syst,
            clk: rcc.clocks.apb_tim_clk,
        }
    }

    /// Starts listening
    pub fn listen(&mut self) {
        self.tim.enable_interrupt()
    }

    /// Stops listening
    pub fn unlisten(&mut self) {
        self.tim.disable_interrupt()
    }
}

impl CountDown for Timer<SYST> {
    type Time = MicroSecond;

    fn start<T>(&mut self, timeout: T)
    where
        T: Into<MicroSecond>,
    {
        let cycles = timeout.into().cycles(self.clk);
        assert!(cycles < 0x00ff_ffff);
        self.tim.set_reload(cycles);
        self.tim.clear_current();
        self.tim.enable_counter();
    }

    fn wait(&mut self) -> nb::Result<(), Void> {
        if self.tim.has_wrapped() {
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}

pub trait TimerExt<TIM> {
    fn timer(self, rcc: &mut Rcc) -> Timer<TIM>;
}

impl TimerExt<SYST> for SYST {
    fn timer(self, rcc: &mut Rcc) -> Timer<SYST> {
        Timer::syst(self, rcc)
    }
}

impl Periodic for Timer<SYST> {}

macro_rules! timers {
    ($($TIM:ident: ($tim:ident, $timXen:ident, $timXrst:ident, $apbenr:ident, $apbrstr:ident, $cnt:ident $(,$cnt_h:ident)*),)+) => {
        $(
            impl Timer<$TIM> {
                /// Configures a TIM peripheral as a periodic count down timer
                pub fn $tim<T>(tim: $TIM, rcc: &mut Rcc) -> Self {
                    rcc.rb.$apbenr.modify(|_, w| w.$timXen().set_bit());
                    rcc.rb.$apbrstr.modify(|_, w| w.$timXrst().set_bit());
                    rcc.rb.$apbrstr.modify(|_, w| w.$timXrst().clear_bit());

                    Timer {
                        tim,
                        clk: rcc.clocks.apb_tim_clk,
                    }
                }

                /// Pauses timer
                pub fn pause(&mut self) {
                    self.tim.cr1.modify(|_, w| w.cen().clear_bit());
                }

                /// Resumes timer
                pub fn resume(&mut self) {
                    self.tim.cr1.modify(|_, w| w.cen().set_bit());
                }

                /// Starts listening
                pub fn listen(&mut self) {
                    self.tim.dier.write(|w| w.uie().set_bit());
                }

                /// Stops listening
                pub fn unlisten(&mut self) {
                    self.tim.dier.write(|w| w.uie().clear_bit());
                }

                /// Clears interrupt flag
                pub fn clear_irq(&mut self) {
                    self.tim.sr.modify(|_, w| w.uif().clear_bit());
                }

                /// Resets counter value
                pub fn reset(&mut self) {
                    self.tim.cnt.reset();
                }

                /// Gets timer counter current value
                pub fn counter(&self) -> u32 {
                    let _high = 0;
                    $(
                        let _high = self.tim.cnt.read().$cnt_h().bits() as u32;
                    )*
                    let low = self.tim.cnt.read().$cnt().bits() as u32;
                    low | (_high << 16)
                }

                /// Releases the TIM peripheral
                pub fn release(self) -> $TIM {
                    self.tim
                }
            }

            impl TimerExt<$TIM> for $TIM {
                fn timer(self, rcc: &mut Rcc) -> Timer<$TIM> {
                    Timer::$tim::<$TIM>(self, rcc)
                }
            }

            impl CountDown for Timer<$TIM> {
                type Time = MicroSecond;

                fn start<T>(&mut self, timeout: T)
                where
                    T: Into<MicroSecond>,
                {
                    // pause
                    self.tim.cr1.modify(|_, w| w.cen().clear_bit());
                    // reset counter
                    self.tim.cnt.reset();

                    // Calculate counter configuration
                    let cycles = timeout.into().cycles(self.clk);
                    let psc = cycles / 0xffff;
                    let arr = cycles / (psc + 1);

                    self.tim.psc.write(|w| unsafe { w.psc().bits(psc as u16) });
                    self.tim.arr.write(|w| unsafe { w.bits(arr) });
                    self.tim.cr1.modify(|_, w| w.cen().set_bit().urs().set_bit());
                }

                fn wait(&mut self) -> nb::Result<(), Void> {
                    if self.tim.sr.read().uif().bit_is_clear() {
                        Err(nb::Error::WouldBlock)
                    } else {
                        self.tim.sr.modify(|_, w| w.uif().clear_bit());
                        Ok(())
                    }
                }
            }

            impl Periodic for Timer<$TIM> {}
        )+
    }
}

timers! {
    TIM1: (tim1, tim1en, tim1rst, apbenr2, apbrstr2, cnt),
    TIM2: (tim2, tim2en, tim2rst, apbenr1, apbrstr1, cnt_l, cnt_h),
    TIM3: (tim3, tim3en, tim3rst, apbenr1, apbrstr1, cnt_l, cnt_h),
    TIM14: (tim14, tim14en, tim14rst, apbenr2, apbrstr2, cnt),
    TIM16: (tim16, tim16en, tim16rst, apbenr2, apbrstr2, cnt),
    TIM17: (tim17, tim17en, tim17rst, apbenr2, apbrstr2, cnt),
}

#[cfg(any(feature = "stm32g07x", feature = "stm32g081"))]
timers! {
    TIM6: (tim6, tim6en, tim6rst, apbenr1, apbrstr1, cnt),
    TIM7: (tim7, tim7en, tim7rst, apbenr1, apbrstr1, cnt),
    TIM15: (tim15, tim15en, tim15rst, apbenr2, apbrstr2, cnt),
}