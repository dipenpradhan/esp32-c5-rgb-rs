#![no_std]
#![no_main]

use embassy_executor::Spawner;
use esp_hal::{
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig},
    timer::{systimer::SystemTimer, OneShotTimer},
};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

#[esp_hal::main]
async fn main(_spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut led = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());

    // Use systimer for async delays (like led_effects_async does)
    let systimer = SystemTimer::new(peripherals.SYSTIMER);
    let mut timer = OneShotTimer::new(systimer.alarm0).into_async();

    // Just blink forever
    loop {
        led.toggle();
        timer.delay_millis_async(500u32).await;
    }
}