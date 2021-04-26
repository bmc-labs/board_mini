#![no_main]
#![no_std]

use wordclock as _;

use cortex_m_rt::entry;
use nb::block;
use smart_leds::{brightness, gamma, SmartLedsWrite, RGB8};
use stm32f1xx_hal::{pac, prelude::*, timer::Timer};
use ws2812_timer_delay::Ws2812;


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
  let clocks = rcc.cfgr.use_hse(8.mhz()).freeze(&mut flash.acr);

  defmt::info!("acquire and configure pin");
  let mut gpioa = dp.GPIOA.split(&mut rcc.apb2);
  let ws2812_pin = gpioa.pa2.into_push_pull_output(&mut gpioa.crl);

  defmt::info!("set up timers");
  let mut wait_timer = Timer::syst(cp.SYST, &clocks).start_count_down(1.hz());
  let ws2812_timer =
    Timer::tim2(dp.TIM2, &clocks, &mut rcc.apb1).start_count_down(3.mhz());

  defmt::info!("set up color containers");
  const STRIPE_LENGTH: usize = 3;
  let mut ws2812_data: [RGB8; STRIPE_LENGTH] = [ON, OFF, OFF];

  defmt::info!("set up WS2812 control struct");
  let mut ws2812 = Ws2812::new(ws2812_timer, ws2812_pin);

  defmt::info!("entering event loop");
  loop {
    ws2812_data.rotate_right(1);
    ws2812.write(brightness(gamma(ws2812_data.iter().cloned()), 0x01))
          .unwrap();
    block!(wait_timer.wait()).unwrap();
  }
}
