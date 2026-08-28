#![no_std]
#![no_main]

//! Test if esp_println serial output works.

use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

#[esp_hal::main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_println::logger::init_logger_from_env();

    println!("HELLO from Rust esp_println");
    println!("Test line 2");
    println!("Test line 3");

    let mut led = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());
    let delay = Delay::new();

    // Blink slowly while printing
    for i in 0..20u32 {
        led.toggle();
        println!("tick {}", i);
        delay.delay_millis(300);
    }

    loop {
        delay.delay_millis(1000);
    }
}