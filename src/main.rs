#![no_std]
#![no_main]

use cortex_m::asm;
use cortex_m_rt::entry;
use panic_halt as _;
use stm32f1xx_hal::{
    pac,
    prelude::*,
};

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();

    let mut rcc = dp.RCC.constrain();
    let mut gpiob = dp.GPIOB.split(&mut rcc);
    let mut gpioc = dp.GPIOC.split(&mut rcc);

    // Blue Pill PC13 LED is active-low: set low => LED on, high => LED off.
    let mut led = gpioc.pc13.into_push_pull_output(&mut gpioc.crh);
    // Internal pull-up enabled on PB12.
    let handset = gpiob.pb12.into_pull_up_input(&mut gpiob.crh);
    let _ = led.set_high();

    loop {
        // Blue Pill LED is active-low, so:
        // HIGH => LED on, LOW => LED off.
        if handset.is_high() {
            let _ = led.set_low();
        } else {
            let _ = led.set_high();
        }
        asm::nop();
    }
}
