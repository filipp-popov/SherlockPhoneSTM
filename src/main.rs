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
    serial::{self},
    timer::{Channel, SysCounterHz, Tim1NoRemap, Timer},
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

fn dtmf_frequencies(key: char) -> Option<(u32, u32)> {
    match key {
        '1' => Some((697, 1209)),
        '2' => Some((697, 1336)),
        '3' => Some((697, 1477)),
        'A' => Some((697, 1633)),
        '4' => Some((770, 1209)),
        '5' => Some((770, 1336)),
        '6' => Some((770, 1477)),
        'B' => Some((770, 1633)),
        '7' => Some((852, 1209)),
        '8' => Some((852, 1336)),
        '9' => Some((852, 1477)),
        'C' => Some((852, 1633)),
        '*' => Some((941, 1209)),
        '0' => Some((941, 1336)),
        '#' => Some((941, 1477)),
        'D' => Some((941, 1633)),
        _ => None,
    }
}

// Test mode: output a single clean tone per key (no dual-tone multiplexing).
const SINGLE_TONE_KEYS: bool = true;
const RING_ON_MS: u32 = 1000;
const RING_OFF_MS: u32 = 3000;
const DIAL_TIMEOUT_MS: u32 = 3000;
const OFFHOOK_IDLE_TIMEOUT_MS: u32 = 10000;
const KEY_EVENT_GUARD_MS: u32 = 120;
const BUSY_STAGE_SWITCH_MS: u32 = 12000;
const BUSY_LONG_ON_MS: u32 = 500;
const BUSY_LONG_OFF_MS: u32 = 500;
const BUSY_SHORT_ON_MS: u32 = 200;
const BUSY_SHORT_OFF_MS: u32 = 200;

struct Route {
    digits: &'static [u8],
    file_index: u16,
    ring_total_ms: u32,
}

const ROUTES: &[Route] = &[
    Route {
        digits: b"5664",
        file_index: 1,
        ring_total_ms: 4000,
    },
    Route {
        digits: b"88522222",
        file_index: 2,
        ring_total_ms: 7000,
    },
        Route {
        digits: b"112",
        file_index: 3,
        ring_total_ms: 7500,
    },
];

fn uart1_write_byte(tx: &mut stm32f1xx_hal::serial::Tx1, b: u8) {
    loop {
        if tx.write_u8(b).is_ok() {
            break;
        }
    }
}

fn dfplayer_send_command(tx: &mut stm32f1xx_hal::serial::Tx1, cmd: u8, param: u16) {
    let version: u8 = 0xFF;
    let len: u8 = 0x06;
    let ack: u8 = 0x00;
    let param_h = (param >> 8) as u8;
    let param_l = (param & 0xFF) as u8;

    let sum = version as u16 + len as u16 + cmd as u16 + ack as u16 + param_h as u16 + param_l as u16;
    let checksum = 0u16.wrapping_sub(sum);
    let c_h = (checksum >> 8) as u8;
    let c_l = (checksum & 0xFF) as u8;

    let frame = [0x7E, version, len, cmd, ack, param_h, param_l, c_h, c_l, 0xEF];
    for b in frame {
        uart1_write_byte(tx, b);
    }
}

fn dfplayer_play_root_index(tx: &mut stm32f1xx_hal::serial::Tx1, index: u16) {
    // Command 0x03: play track by index from root (FAT order dependent).
    dfplayer_send_command(tx, 0x03, index);
}

fn dfplayer_stop(tx: &mut stm32f1xx_hal::serial::Tx1) {
    // Command 0x16: stop playback.
    dfplayer_send_command(tx, 0x16, 0);
}

fn delay_ms(timer: &mut SysCounterHz, ms: u32) {
    for _ in 0..ms {
        loop {
            if timer.wait().is_ok() {
                break;
            }
        }
    }
}

struct DfFrameParser {
    buf: [u8; 10],
    idx: usize,
    in_frame: bool,
}

impl DfFrameParser {
    fn new() -> Self {
        Self {
            buf: [0; 10],
            idx: 0,
            in_frame: false,
        }
    }

    fn push(&mut self, b: u8) -> Option<(u8, u16)> {
        if !self.in_frame {
            if b == 0x7E {
                self.in_frame = true;
                self.idx = 0;
                self.buf[self.idx] = b;
                self.idx += 1;
            }
            return None;
        }

        if self.idx < self.buf.len() {
            self.buf[self.idx] = b;
            self.idx += 1;
        } else {
            self.in_frame = false;
            self.idx = 0;
            return None;
        }

        if self.idx == 10 {
            self.in_frame = false;
            self.idx = 0;
            if self.buf[0] == 0x7E && self.buf[9] == 0xEF {
                let cmd = self.buf[3];
                let param = ((self.buf[5] as u16) << 8) | self.buf[6] as u16;
                return Some((cmd, param));
            }
        }

        None
    }
}

fn is_digit_key(key: char) -> bool {
    matches!(key, '0'..='9')
}

fn is_valid_prefix(buf: &[u8]) -> bool {
    ROUTES.iter().any(|r| r.digits.starts_with(buf))
}

fn find_exact_route(buf: &[u8]) -> Option<&'static Route> {
    ROUTES.iter().find(|r| r.digits == buf)
}

fn busy_tone_on(now_ms: u32, busy_start_ms: u32) -> bool {
    let elapsed = now_ms.wrapping_sub(busy_start_ms);
    if elapsed < BUSY_STAGE_SWITCH_MS {
        let period = BUSY_LONG_ON_MS + BUSY_LONG_OFF_MS;
        (elapsed % period) < BUSY_LONG_ON_MS
    } else {
        let period = BUSY_SHORT_ON_MS + BUSY_SHORT_OFF_MS;
        (elapsed % period) < BUSY_SHORT_ON_MS
    }
}

fn ring_tone_on_for_total(elapsed_ms: u32, total_ms: u32, on_ms: u32, off_ms: u32) -> bool {
    if elapsed_ms >= total_ms {
        return false;
    }
    let period = on_ms + off_ms;
    (elapsed_ms % period) < on_ms
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
    let cp = cortex_m::Peripherals::take().unwrap();
    let dp = pac::Peripherals::take().unwrap();

    let mut rcc = dp.RCC.constrain();
    let mut afio = dp.AFIO.constrain(&mut rcc);
    let mut gpioa = dp.GPIOA.split(&mut rcc);
    let mut gpiob = dp.GPIOB.split(&mut rcc);
    let mut gpioc = dp.GPIOC.split(&mut rcc);
    let mut systick = Timer::syst(cp.SYST, &rcc.clocks).counter_hz();
    let _ = systick.start(1000.Hz());

    // Blue Pill PC13 LED is active-low: set low => LED on, high => LED off.
    let mut led = gpioc.pc13.into_push_pull_output(&mut gpioc.crh);
    // Internal pull-up enabled on PB12.
    let handset = gpiob.pb12.into_pull_up_input(&mut gpiob.crh);
    let _ = led.set_high();

    // UART1 on PA9/PA10 for DFPlayer Mini (9600 8N1).
    let tx_pin = gpioa.pa9.into_alternate_push_pull(&mut gpioa.crh);
    let rx_pin = gpioa.pa10;
    let serial = dp.USART1.serial(
        (tx_pin, rx_pin),
        serial::Config::default().baudrate(9600.bps()),
        &mut rcc,
    );
    let (mut df_tx, mut df_rx) = serial.split();

    // Tone output on PA8 (TIM1_CH1) for line tone + DTMF.
    let tone_pin = gpioa.pa8.into_alternate_push_pull(&mut gpioa.crh);
    let mut tone_pwm = dp
        .TIM1
        .pwm_hz::<Tim1NoRemap, _, _>(tone_pin, &mut afio.mapr, 425.Hz(), &mut rcc);
    tone_pwm.enable(Channel::C1);
    let tone_max = tone_pwm.get_max_duty();
    tone_pwm.set_duty(Channel::C1, 0);

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
    let mut dtmf_phase_high = false;
    let mut current_tone_hz: u32 = 0;
    let mut current_tone_duty: u16 = 0;
    let mut tone_enabled = false;
    let mut dialing_started = false;
    let mut number_accepted = false;
    let mut ringing = false;
    let mut ring_start_ms: u32 = 0;
    let mut ring_total_ms: u32 = 0;
    let mut ring_answer_file_index: u16 = 0;
    let mut playing = false;
    let mut busy_mode = false;
    let mut busy_start_ms: u32 = 0;
    let mut now_ms: u32 = 0;
    let mut offhook_start_ms: u32 = 0;
    let mut last_keypress_ms: u32 = 0;
    let mut last_digit_event_ms: u32 = 0;
    let mut prev_pressed_key: Option<char> = None;
    let mut prev_handset_up: bool = false;
    let mut df_parser = DfFrameParser::new();
    let mut dial_buf: [u8; 8] = [0; 8];
    let mut dial_len: usize = 0;
    rtt_init_print!();
    rprintln!("boot");
    // DFPlayer Mini init sequence: wait power-up, reset, select TF, then set volume.
    // Helps with unreliable startup on some modules/clones.
    delay_ms(&mut systick, 1000);
    dfplayer_send_command(&mut df_tx, 0x0C, 0); // reset
    delay_ms(&mut systick, 1200);
    dfplayer_send_command(&mut df_tx, 0x09, 2); // select TF card
    delay_ms(&mut systick, 200);
    dfplayer_send_command(&mut df_tx, 0x06, 18); // volume 0..30
    delay_ms(&mut systick, 200);

    loop {
        if systick.wait().is_ok() {
            now_ms = now_ms.wrapping_add(1);
        }

        loop {
            match df_rx.read() {
                Ok(b) => {
                    if let Some((cmd, param)) = df_parser.push(b) {
                        if cmd == 0x3D {
                            rprintln!("dfplayer: track finished param={}", param);
                            playing = false;
                        } else {
                            rprintln!("dfplayer: cmd=0x{:x} param={}", cmd, param);
                        }
                    }
                }
                Err(nb::Error::WouldBlock) => break,
                Err(_) => break,
            }
        }

        let pressed_key = remap_key(keypad.scan_key());
        let handset_up = handset.is_high();
        if handset_up && !prev_handset_up {
            offhook_start_ms = now_ms;
        }
        if !handset_up && prev_handset_up {
            // On handset down, force stop any playing track.
            if playing {
                rprintln!("handset down -> stop playback");
                dfplayer_stop(&mut df_tx);
                playing = false;
            }
        }
        if !handset_up {
            dialing_started = false;
            number_accepted = false;
            ringing = false;
            ring_start_ms = 0;
            ring_total_ms = 0;
            ring_answer_file_index = 0;
            busy_mode = false;
            dial_len = 0;
            last_keypress_ms = now_ms;
            last_digit_event_ms = now_ms;
            prev_pressed_key = None;
        } else if pressed_key.is_some() {
            dialing_started = true;
        }

        let key_press_event_raw = pressed_key.is_some() && prev_pressed_key.is_none();
        let key_press_event = key_press_event_raw
            && now_ms.wrapping_sub(last_digit_event_ms) >= KEY_EVENT_GUARD_MS;
        if handset_up && key_press_event {
            let key = pressed_key.unwrap();
            if !busy_mode && !number_accepted && !ringing && !playing && is_digit_key(key) {
                last_digit_event_ms = now_ms;
                last_keypress_ms = now_ms;
                rprintln!(
                    "digit event key={} now_ms={} last_keypress_ms={} dial_len={}",
                    key,
                    now_ms,
                    last_keypress_ms,
                    dial_len
                );
                if dial_len < dial_buf.len() {
                    dial_buf[dial_len] = key as u8;
                    dial_len += 1;
                } else {
                    // Buffer full: start a fresh number with current digit.
                    dial_buf[0] = key as u8;
                    dial_len = 1;
                }

                if let Some(route) = find_exact_route(&dial_buf[..dial_len]) {
                    rprintln!(
                        "dial match -> ringing total {}ms, file {}",
                        route.ring_total_ms,
                        route.file_index
                    );
                    ringing = true;
                    ring_start_ms = now_ms;
                    ring_total_ms = route.ring_total_ms;
                    ring_answer_file_index = route.file_index;
                    dial_len = 0;
                    last_keypress_ms = now_ms;
                } else if !is_valid_prefix(&dial_buf[..dial_len]) {
                    // Do not trigger busy immediately on wrong prefix.
                    // Keep collecting until timeout after the last entered digit.
                    rprintln!("invalid prefix, waiting timeout");
                }
            }
        }
        prev_pressed_key = pressed_key;

        if handset_up && ringing {
            let elapsed = now_ms.wrapping_sub(ring_start_ms);
            if elapsed >= ring_total_ms {
                ringing = false;
                number_accepted = true;
                rprintln!(
                    "ringing done -> answer play ROOT idx {}",
                    ring_answer_file_index
                );
                dfplayer_play_root_index(&mut df_tx, ring_answer_file_index);
                playing = true;
            }
        }

        if handset_up
            && !dialing_started
            && !busy_mode
            && !number_accepted
            && !ringing
            && now_ms.wrapping_sub(offhook_start_ms) >= OFFHOOK_IDLE_TIMEOUT_MS
        {
            rprintln!(
                "offhook idle timeout -> busy now_ms={} offhook_start_ms={} delta={}",
                now_ms,
                offhook_start_ms,
                now_ms.wrapping_sub(offhook_start_ms)
            );
            busy_mode = true;
            busy_start_ms = now_ms;
        }

        if handset_up
            && dialing_started
            && dial_len > 0
            && pressed_key.is_none()
            && !busy_mode
            && !number_accepted
            && !ringing
        {
            if now_ms.wrapping_sub(last_keypress_ms) >= DIAL_TIMEOUT_MS {
                rprintln!(
                    "dial timeout/wrong -> busy now_ms={} last_keypress_ms={} delta={} dial_len={}",
                    now_ms,
                    last_keypress_ms,
                    now_ms.wrapping_sub(last_keypress_ms),
                    dial_len
                );
                busy_mode = true;
                busy_start_ms = now_ms;
            }
        }

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
        prev_handset_up = handset_up;

        heartbeat = heartbeat.wrapping_add(1);
        if heartbeat % 1 == 0 {
            dtmf_phase_high = !dtmf_phase_high;
        }

        let desired_tone_hz = if handset_up {
            if busy_mode {
                if busy_tone_on(now_ms, busy_start_ms) {
                    425
                } else {
                    0
                }
            } else if playing {
                // Ignore keypad tones while file is playing.
                0
            } else if let Some(key) = pressed_key {
                if let Some((f_low, f_high)) = dtmf_frequencies(key) {
                    if SINGLE_TONE_KEYS {
                        f_low
                    } else if dtmf_phase_high {
                        f_high
                    } else {
                        f_low
                    }
                } else {
                    0
                }
            } else if ringing {
                let elapsed = now_ms.wrapping_sub(ring_start_ms);
                if ring_tone_on_for_total(elapsed, ring_total_ms, RING_ON_MS, RING_OFF_MS) {
                    425
                } else {
                    0
                }
            } else if number_accepted {
                0
            } else if !dialing_started {
                // Line tone only before first key press.
                425
            } else {
                0
            }
        } else {
            0
        };
        let desired_tone_duty = if busy_mode {
            tone_max / 32
        } else if handset_up && pressed_key.is_none() && !dialing_started {
            // Lower line tone level for handset path.
            tone_max / 32
        } else {
            // Lower DTMF level for handset path.
            tone_max / 32
        };

        if desired_tone_hz == 0 {
            if tone_enabled {
                tone_pwm.set_duty(Channel::C1, 0);
                tone_enabled = false;
                current_tone_hz = 0;
                current_tone_duty = 0;
            }
        } else {
            if desired_tone_hz != current_tone_hz {
                tone_pwm.set_period(desired_tone_hz.Hz());
                current_tone_hz = desired_tone_hz;
            }
            if !tone_enabled || desired_tone_duty != current_tone_duty {
                tone_pwm.set_duty(Channel::C1, desired_tone_duty);
                current_tone_duty = desired_tone_duty;
            }
            if !tone_enabled {
                tone_enabled = true;
            }
        }

        asm::nop();
    }
}
