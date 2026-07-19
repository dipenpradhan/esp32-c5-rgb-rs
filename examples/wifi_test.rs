#![no_std]
#![no_main]

//! WiFi test with LED indication.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::ToString;
use embassy_executor::Spawner;
use embassy_net::{Config, Runner, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    interrupt::software::SoftwareInterruptControl,
    ram,
    timer::timg::TimerGroup,
};
use esp_println::println;
use esp_radio::wifi::{
    Config as WifiConfig,
    ControllerConfig,
    Interface,
    WifiController,
    sta::StationConfig,
};

esp_bootloader_esp_idf::esp_app_desc!();
use esp_backtrace as _;

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const WIFI_SSID: &str = "D";
const WIFI_PASS: &str = "REDACTED_WIFI_PASSWORD";

#[esp_hal::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Blink LED 3 times at startup
    let mut led = Output::new(peripherals.GPIO27, Level::Low, OutputConfig::default());
    let delay = Delay::new();
    for _ in 0..3 {
        led.set_high();
        delay.delay_millis(200);
        led.set_low();
        delay.delay_millis(200);
    }

    // Init logger
    esp_println::logger::init_logger_from_env();
    println!("=== WiFi test starting ===");

    // Two heap allocators (required by esp-radio)
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);
    println!("Heaps initialized");

    // Start RTOS
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    println!("RTOS started");

    // LED stays on
    led.set_high();

    // WiFi init
    println!("WiFi: creating controller...");
    let station_config = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(WIFI_PASS.to_string()),
    );

    let wifi_interface = Interface::station();
    let controller = WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("WiFi init failed");
    println!("WiFi: controller created!");

    let config = Config::dhcpv4(Default::default());
    let (stack, runner) = embassy_net::new(
        wifi_interface,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        0,
    );

    println!("WiFi: spawning tasks...");
    spawner.spawn(connection(controller).expect("spawn connection"));
    spawner.spawn(net_task(runner).expect("spawn net_task"));

    println!("WiFi: waiting for DHCP...");
    stack.wait_config_up().await;

    if let Some(config) = stack.config_v4() {
        println!("WiFi: Got IP: {}", config.address);
        // Fast blink = got IP
        for _ in 0..5 {
            led.set_low();
            Timer::after(Duration::from_millis(100)).await;
            led.set_high();
            Timer::after(Duration::from_millis(100)).await;
        }
    }

    println!("WiFi: entering loop...");
    led.set_low(); // LED off = waiting for connection
    loop {
        Timer::after(Duration::from_millis(5000)).await;
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        println!("WiFi: connecting...");
        match controller.connect_async().await {
            Ok(info) => {
                println!("WiFi: connected {:?}", info);
                let info = controller.wait_for_disconnect_async().await.ok();
                println!("WiFi: disconnected {:?}", info);
            }
            Err(e) => {
                println!("WiFi: connect error {:?}", e);
            }
        }
        Timer::after(Duration::from_millis(5000)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}