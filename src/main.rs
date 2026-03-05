#![no_std]
#![no_main]

use cortex_m::asm;
use cortex_m_rt::entry;
use panic_halt as _;
use rtt_target::{rprintln, rtt_init_print};
use stm32f1xx_hal::{
    gpio::{
        gpioa::{PA0, PA1, PA2, PA3, PA4, PA5, PA6, PA7},
        Input, Output, PullUp, PushPull,
    },
    pac,
    prelude::*,
};

fn key_label(key: Option<char>) -> &'static str {
    match key {
        Some('1') => "1",
        Some('2') => "2",
        Some('3') => "3",
        Some('A') => "A",
        Some('4') => "4",
        Some('5') => "5",
        Some('6') => "6",
        Some('B') => "B",
        Some('7') => "7",
        Some('8') => "8",
        Some('9') => "9",
        Some('C') => "C",
        Some('*') => "*",
        Some('0') => "0",
        Some('#') => "#",
        Some('D') => "D",
        None => "none",
        _ => "unknown",
    }
}

fn remap_key(raw: Option<char>) -> Option<char> {
    match raw {
        Some('5') => Some('1'),
        Some('4') => Some('2'),
        Some('B') => Some('3'),
        Some('6') => Some('A'),
        Some('2') => Some('4'),
        Some('1') => Some('5'),
        Some('A') => Some('6'),
        Some('3') => Some('B'),
        Some('8') => Some('7'),
        Some('7') => Some('8'),
        Some('C') => Some('9'),
        Some('9') => Some('C'),
        Some('0') => Some('*'),
        Some('*') => Some('0'),
        Some('D') => Some('#'),
        Some('#') => Some('D'),
        _ => raw,
    }
}

struct Keypad4x4 {
    r0: PA7<Output<PushPull>>,
    r1: PA6<Output<PushPull>>,
    r2: PA1<Output<PushPull>>,
    r3: PA0<Output<PushPull>>,
    c0: PA5<Input<PullUp>>,
    c1: PA4<Input<PullUp>>,
    c2: PA3<Input<PullUp>>,
    c3: PA2<Input<PullUp>>,
}

impl Keypad4x4 {
    fn set_all_rows_high(&mut self) {
        let _ = self.r0.set_high();
        let _ = self.r1.set_high();
        let _ = self.r2.set_high();
        let _ = self.r3.set_high();
    }

    fn scan_key(&mut self) -> Option<char> {
        self.set_all_rows_high();

        let _ = self.r0.set_low();
        if self.c0.is_low() {
            return Some('1');
        }
        if self.c1.is_low() {
            return Some('2');
        }
        if self.c2.is_low() {
            return Some('3');
        }
        if self.c3.is_low() {
            return Some('A');
        }
        let _ = self.r0.set_high();

        let _ = self.r1.set_low();
        if self.c0.is_low() {
            return Some('4');
        }
        if self.c1.is_low() {
            return Some('5');
        }
        if self.c2.is_low() {
            return Some('6');
        }
        if self.c3.is_low() {
            return Some('B');
        }
        let _ = self.r1.set_high();

        let _ = self.r2.set_low();
        if self.c0.is_low() {
            return Some('7');
        }
        if self.c1.is_low() {
            return Some('8');
        }
        if self.c2.is_low() {
            return Some('9');
        }
        if self.c3.is_low() {
            return Some('C');
        }
        let _ = self.r2.set_high();

        let _ = self.r3.set_low();
        if self.c0.is_low() {
            return Some('*');
        }
        if self.c1.is_low() {
            return Some('0');
        }
        if self.c2.is_low() {
            return Some('#');
        }
        if self.c3.is_low() {
            return Some('D');
        }
        let _ = self.r3.set_high();

        None
    }
}

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();

    let mut rcc = dp.RCC.constrain();
    let mut gpioa = dp.GPIOA.split(&mut rcc);
    let mut gpiob = dp.GPIOB.split(&mut rcc);
    let mut gpioc = dp.GPIOC.split(&mut rcc);

    // Blue Pill PC13 LED is active-low: set low => LED on, high => LED off.
    let mut led = gpioc.pc13.into_push_pull_output(&mut gpioc.crh);
    // Internal pull-up enabled on PB12.
    let handset = gpiob.pb12.into_pull_up_input(&mut gpiob.crh);
    let _ = led.set_high();

    let mut keypad = Keypad4x4 {
        r0: gpioa.pa7.into_push_pull_output(&mut gpioa.crl),
        r1: gpioa.pa6.into_push_pull_output(&mut gpioa.crl),
        r2: gpioa.pa1.into_push_pull_output(&mut gpioa.crl),
        r3: gpioa.pa0.into_push_pull_output(&mut gpioa.crl),
        c0: gpioa.pa5.into_pull_up_input(&mut gpioa.crl),
        c1: gpioa.pa4.into_pull_up_input(&mut gpioa.crl),
        c2: gpioa.pa3.into_pull_up_input(&mut gpioa.crl),
        c3: gpioa.pa2.into_pull_up_input(&mut gpioa.crl),
    };
    keypad.set_all_rows_high();
    let mut last_key: Option<char> = None;
    let mut last_handset_up: bool = false;
    let mut heartbeat: u32 = 0;
    rtt_init_print!();
    rprintln!("boot");

    loop {
        let pressed_key = remap_key(keypad.scan_key());
        let handset_up = handset.is_high();

        // Blue Pill LED is active-low, so:
        // HIGH => LED on, LOW => LED off.
        if handset_up {
            let _ = led.set_low();
        } else {
            let _ = led.set_high();
        }

        if pressed_key != last_key || handset_up != last_handset_up {
            rprintln!(
                "key={} handset_up={}",
                key_label(pressed_key),
                handset_up
            );
            last_key = pressed_key;
            last_handset_up = handset_up;
        }

        heartbeat = heartbeat.wrapping_add(1);
        if heartbeat % 200_000 == 0 {
            rprintln!(
                "alive key={} handset_up={}",
                key_label(pressed_key),
                handset_up
            );
        }
        asm::nop();
    }
}
