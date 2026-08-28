#![no_std]
#![no_main]

use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

#[esp_hal::main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut led = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());
    use esp_hal::delay::Delay;
    let delay = Delay::new();

    loop {
        led.toggle();
        delay.delay_millis(500);
    }
}