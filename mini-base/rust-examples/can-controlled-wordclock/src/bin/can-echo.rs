#![no_main]
#![no_std]

use wordclock as _;

use bxcan::filter::Mask32;
use cortex_m_rt::entry;
use embedded_hal::digital::v2::OutputPin;
use nb::block;
use stm32f1xx_hal::{can::Can, pac, prelude::*};

#[entry]
fn main() -> ! {
  defmt::info!("starting up");

  defmt::info!("acquiring peripherals");
  let dp = pac::Peripherals::take().unwrap();

  let mut flash = dp.FLASH.constrain();
  let mut rcc = dp.RCC.constrain();

  // To meet CAN clock accuracy requirements an external crystal or ceramic
  // resonator must be used. The blue pill has a 8MHz external crystal.
  // Other boards might have a crystal with another frequency or none at all.
  defmt::info!("set crystal to external");
  rcc.cfgr.use_hse(8.mhz()).freeze(&mut flash.acr);

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

  // APB1 (PCLK1): 8MHz, Bit rate: 125kBit/s, Sample Point 87.5%
  // Value was calculated with http://www.bittiming.can-wiki.info/
  defmt::info!("configuring CAN interface bit timing");
  can1.modify_config().set_bit_timing(0x001c_0003);

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

  // Echo back received packages in sequence.
  // See the `can-rtfm` example for an echo implementation that adheres to
  // correct frame ordering based on the transfer id.
  defmt::info!("entering event loop");
  loop {
    defmt::info!("waiting for CAN frame...");
    if let Ok(frame) = block!(can.receive()) {
      defmt::info!("received {:?}, echoing...", frame);
      block!(can.transmit(&frame)).unwrap();
      defmt::info!("echoed, looping around");
    } else {
      defmt::info!("FATAL");
    }
  }
}
