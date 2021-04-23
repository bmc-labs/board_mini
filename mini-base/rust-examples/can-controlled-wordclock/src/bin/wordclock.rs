#![no_main]
#![no_std]

use wordclock as _;

use bxcan::filter::Mask32;
use cortex_m_rt::entry;
use embedded_hal::digital::v2::OutputPin;
use nb::block;
use smart_leds::{brightness, gamma, SmartLedsWrite, RGB8};
use stm32f1xx_hal::{can::Can, pac, prelude::*, timer::Timer};
use ws2812_timer_delay::Ws2812;


#[derive(Copy, Clone)]
struct Word {
  pub from_x: usize,
  pub to_x:   usize,
  pub y:      usize,
}


#[rustfmt::skip]
const OFFSET   : usize = 2;
#[rustfmt::skip]
const WCLK_SIZE: usize = 10 * 11 + 4;

// always on: ES, IST
#[rustfmt::skip]
const ES : Word = Word { from_x: 0, to_x: 2, y: 0, };
#[rustfmt::skip]
const IST: Word = Word { from_x: 3, to_x: 6, y: 0, };

// sometimes needed: DIGITS, UHR
#[rustfmt::skip]
const DIGITS: [usize; 4] = [0, 1, 113, 112];
#[rustfmt::skip]
const UHR   : Word = Word { from_x: 8, to_x: 11, y: 9, };

// composite relational definitions:
#[rustfmt::skip]
const FUENF      : Word = Word { from_x: 7, to_x: 11, y: 0, };
#[rustfmt::skip]
const ZEHN       : Word = Word { from_x: 0, to_x:  4, y: 1, };
#[rustfmt::skip]
const ZWANZIG    : Word = Word { from_x: 4, to_x: 11, y: 1, };
#[rustfmt::skip]
const DREIVIERTEL: Word = Word { from_x: 0, to_x: 11, y: 2, };
#[rustfmt::skip]
const VIERTEL    : Word = Word { from_x: 4, to_x: 11, y: 2, };
#[rustfmt::skip]
const VOR        : Word = Word { from_x: 0, to_x:  3, y: 3, };
#[rustfmt::skip]
const NACH       : Word = Word { from_x: 7, to_x: 11, y: 3, };
#[rustfmt::skip]
const HALB       : Word = Word { from_x: 0, to_x:  4, y: 4, };

// hours:
#[rustfmt::skip]
const HOURS: [Word; 13] = [Word { from_x: 0, to_x:  0, y: 0, }, // dummy
                           Word { from_x: 0, to_x:  4, y: 5, },
                           Word { from_x: 7, to_x: 11, y: 5, },
                           Word { from_x: 0, to_x:  4, y: 6, },
                           Word { from_x: 7, to_x: 11, y: 6, },
                           Word { from_x: 7, to_x: 11, y: 4, },
                           Word { from_x: 0, to_x:  5, y: 7, },
                           Word { from_x: 0, to_x:  6, y: 8, },
                           Word { from_x: 7, to_x: 11, y: 7, },
                           Word { from_x: 3, to_x:  7, y: 9, },
                           Word { from_x: 0, to_x:  4, y: 9, },
                           Word { from_x: 5, to_x:  8, y: 4, },
                           Word { from_x: 6, to_x: 11, y: 8, }];

// switch RGB structs
#[rustfmt::skip]
const OFF: RGB8 = RGB8::new(0x00, 0x00, 0x00);
#[rustfmt::skip]
const ON : RGB8 = RGB8::new(0xff, 0xff, 0xff);


#[entry]
fn main() -> ! {
  defmt::info!("starting up");

  defmt::info!("acquiring peripherals");
  // cp: core peripherals, dp: device specific peripherals
  let cp = cortex_m::Peripherals::take().unwrap();
  let dp = pac::Peripherals::take().unwrap();

  defmt::info!("acquiring flash and rcc devices");
  let mut flash = dp.FLASH.constrain();
  let mut rcc = dp.RCC.constrain();

  // To meet CAN clock accuracy requirements an external crystal or ceramic
  // resonator must be used. The blue pill has a 8MHz external crystal.
  // Other boards might have a crystal with another frequency or none at all.
  defmt::info!("set crystal to external and set clock");
  let clocks = rcc.cfgr
                  .use_hse(8.mhz())
                  .sysclk(72.mhz())
                  .hclk(72.mhz())
                  .pclk1(36.mhz())
                  .pclk2(72.mhz())
                  .adcclk(12.mhz())
                  .freeze(&mut flash.acr);

  defmt::info!("setting up AFIO");
  let mut afio = dp.AFIO.constrain(&mut rcc.apb2);

  // set pin PA0 high to activate CAN final resistor
  // NOT WORKING ON rev0 OF mini::base BECAUSE OF HARDWARE BUG
  defmt::info!("activating CAN final resistor");
  let mut gpioa = dp.GPIOA.split(&mut rcc.apb2);
  let mut final_resistor = gpioa.pa0.into_push_pull_output(&mut gpioa.crl);
  final_resistor.set_high().unwrap();

  defmt::info!("setting up CAN interface");
  let mut can1 = {
    #[cfg(not(feature = "connectivity"))]
    let can = Can::new(dp.CAN1, &mut rcc.apb1, dp.USB);
    #[cfg(feature = "connectivity")]
    let can = Can::new(dp.CAN1, &mut rcc.apb1);

    let mut gpiob = dp.GPIOB.split(&mut rcc.apb2);
    let rx = gpiob.pb8.into_floating_input(&mut gpiob.crh);
    let tx = gpiob.pb9.into_alternate_push_pull(&mut gpiob.crh);
    can.assign_pins((tx, rx), &mut afio.mapr);

    bxcan::Can::new(can)
  };

  // APB1 (PCLK1): 36MHz, Bit rate: 500kBit/s, Sample Point 88.9%
  // Value was calculated with http://www.bittiming.can-wiki.info/
  defmt::info!("configuring CAN interface bit timing");
  can1.modify_config().set_bit_timing(0x001e_0003);

  // Configure filters so that can frames can be received.
  defmt::info!("configuring filters for the CAN interface");
  let mut filters = can1.modify_filters();
  filters.enable_bank(0, Mask32::accept_all());

  // Drop filters to leave filter configuraiton mode.
  drop(filters);

  // Select the interface.
  defmt::info!("set CAN accessible");
  let mut can = can1;

  // Split the peripheral into transmitter and receiver parts.
  block!(can.enable()).unwrap();

  defmt::info!("acquire and configure pin");
  let ctrl_pin = gpioa.pa2.into_push_pull_output(&mut gpioa.crl);
  let wclk_pin = gpioa.pa3.into_push_pull_output(&mut gpioa.crl);

  defmt::info!("set up timers");
  let mut wait_timer = Timer::syst(cp.SYST, &clocks).start_count_down(5.hz());
  let ctrl_timer =
    Timer::tim2(dp.TIM2, &clocks, &mut rcc.apb1).start_count_down(3.mhz());
  let wclk_timer =
    Timer::tim3(dp.TIM3, &clocks, &mut rcc.apb1).start_count_down(3.mhz());

  defmt::info!("set up color containers");
  const CTRL_SIZE: usize = 3;
  let mut ctrl_data: [RGB8; CTRL_SIZE] = [OFF; CTRL_SIZE];
  let mut wclk_data: [RGB8; WCLK_SIZE] = [OFF; WCLK_SIZE];

  defmt::info!("set up WS2812 control struct");
  let mut ctrl = Ws2812::new(ctrl_timer, ctrl_pin);
  let mut wclk = Ws2812::new(wclk_timer, wclk_pin);

  wclk.write(gamma(wclk_data.iter().cloned())).unwrap();
  defmt::info!("entering event loop");
  loop {
    for i in 0..3 {
      defmt::info!("loading ctrl LED {}", i);
      ctrl_data[i] = ON;
      ctrl.write(brightness(gamma(ctrl_data.iter().cloned()), 0x10))
          .unwrap();
      block!(wait_timer.wait()).unwrap();
    }

    reset_ws2812(&mut ctrl_data);
    ctrl.write(ctrl_data.iter().cloned()).unwrap();

    defmt::info!("waiting for CAN frame...");
    if let Ok(frame) = block!(can.receive()) {
      let id = match frame.id() {
        bxcan::Id::Standard(id) => id.as_raw(),
        _ => {
          defmt::error!("FATAL: unable to read frame id");
          wordclock::exit();
        }
      };
      defmt::info!("received message with id {:x}: time data", id);

      let (hour, minute) = match frame.data() {
        Some(data) => (data[0], data[1]),
        None => {
          defmt::error!("FATAL: unable to read time data");
          wordclock::exit();
        }
      };
      defmt::info!("received time: it is {}:{}", hour, minute);

      reset_wclk(&mut wclk_data);

      match minute {
        0..=4 => set_word(&mut wclk_data, UHR, ON),
        5..=9 => {
          set_word(&mut wclk_data, FUENF, ON);
          set_word(&mut wclk_data, NACH, ON);
        }
        10..=14 => {
          set_word(&mut wclk_data, ZEHN, ON);
          set_word(&mut wclk_data, NACH, ON);
        }
        15..=19 => {
          set_word(&mut wclk_data, VIERTEL, ON);
          set_word(&mut wclk_data, NACH, ON);
        }
        20..=24 => {
          set_word(&mut wclk_data, ZWANZIG, ON);
          set_word(&mut wclk_data, NACH, ON);
        }
        25..=29 => {
          set_word(&mut wclk_data, FUENF, ON);
          set_word(&mut wclk_data, VOR, ON);
          set_word(&mut wclk_data, HALB, ON);
        }
        30..=34 => {
          set_word(&mut wclk_data, HALB, ON);
        }
        35..=39 => {
          set_word(&mut wclk_data, FUENF, ON);
          set_word(&mut wclk_data, NACH, ON);
          set_word(&mut wclk_data, HALB, ON);
        }
        40..=44 => {
          set_word(&mut wclk_data, ZWANZIG, ON);
          set_word(&mut wclk_data, VOR, ON);
        }
        45..=49 => {
          set_word(&mut wclk_data, DREIVIERTEL, ON);
        }
        50..=54 => {
          set_word(&mut wclk_data, ZEHN, ON);
          set_word(&mut wclk_data, VOR, ON);
        }
        55..=59 => {
          set_word(&mut wclk_data, FUENF, ON);
          set_word(&mut wclk_data, VOR, ON);
        }
        _ => {
          defmt::error!("FATAL: minute ({}) out of range", minute);
          wordclock::exit();
        }
      }

      if minute < 25 {
        set_word(&mut wclk_data, HOURS[hour as usize], ON);
      } else {
        set_word(&mut wclk_data, HOURS[((hour + 1) % 12) as usize], ON);
      }

      for m in 1..((minute % 5) + 1) {
        wclk_data[DIGITS[(m - 1) as usize]] = ON;
      }

      wclk.write(gamma(wclk_data.iter().cloned())).unwrap();
    } else {
      defmt::error!("FATAL: CAN is BUS HEAVY");
      wordclock::exit();
    }
  }
}

fn set_word(data: &mut [RGB8], word: Word, rgb: RGB8) {
  for x in word.from_x..word.to_x {
    let y = match x % 2 {
      1 => 9 - word.y,
      _ => word.y,
    };

    data[(10 * x + y) + OFFSET] = rgb;
  }
}

fn reset_ws2812(data: &mut [RGB8]) {
  for led in data.iter_mut() {
    *led = OFF;
  }
}

fn reset_wclk(data: &mut [RGB8]) {
  reset_ws2812(data);

  set_word(data, ES, ON);
  set_word(data, IST, ON);
}
