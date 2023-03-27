#![no_std]
#![no_main]

// pick a panicking behavior
use panic_halt as _; // you can put a breakpoint on `rust_begin_unwind` to catch panics
// use panic_abort as _; // requires nightly
// use panic_itm as _; // logs messages over ITM; requires ITM support
// use panic_semihosting as _; // logs messages to the host stderr; requires a debugger

//use cortex_m::asm;
use core::cell::RefCell;
use cortex_m::interrupt::Mutex;
use cortex_m_rt::entry;
//use cortex_m_semihosting::hprintln;
use stm32f4xx_hal::{
    pac::{self, interrupt, Interrupt, TIM2},
    prelude::*,
    timer::{CounterUs, Event},
    otg_fs::{UsbBus, USB},
    gpio::{self, Output, PushPull},
};
use usb_device::prelude::*;
use usbd_human_interface_device::page::Keyboard;
use usbd_human_interface_device::device::keyboard::{
    KeyboardLedsReport, NKROBootKeyboardInterface
};
use usbd_human_interface_device::prelude::*;


static mut EP_MEMORY: [u32; 1024] = [0; 1024];

static GLOBAL_LED: Mutex<RefCell<Option<gpio::PD15<Output<PushPull>>>>> = {
    Mutex::new(RefCell::new(None))
};

static GLOBAL_TIMER: Mutex<RefCell<Option<CounterUs<TIM2>>>> = {
    Mutex::new(RefCell::new(None))
};


#[interrupt]
fn TIM2() {
    static mut LED: Option<gpio::PD15<Output<PushPull>>> = None;
    static mut TIMER: Option<CounterUs<TIM2>> = None;

    let led = LED.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // move the led pin to here
            GLOBAL_LED.borrow(cs).replace(None).unwrap()
        })
    });

    let timer = TIMER.get_or_insert_with(|| {
        cortex_m::interrupt::free(|cs| {
            // move the timer here
            GLOBAL_TIMER.borrow(cs).replace(None).unwrap()
        })
    });

    led.toggle();
    let _ = timer.wait();
}


#[entry]
fn main() -> ! {
    // semihosting
    //    hprintln!("Sup Bitch").unwrap();

    // take peripherals from hal
    let device_peripherals = pac::Peripherals::take().unwrap();

    // CLOCKS AND TIMERS
    // configure the clocks
    let rcc = device_peripherals.RCC.constrain();

    let clocks = rcc
            .cfgr
            .use_hse(8.MHz())
            .sysclk(48.MHz())
            .pclk1(8.MHz())
            .require_pll48clk()
            .freeze();

    // timer config
    let mut timer = device_peripherals.TIM2.counter(&clocks);
    timer.start(1.secs()).unwrap();

    // keyboard tick timer
    let mut key_timer = device_peripherals.TIM1.counter_ms(&clocks);
    key_timer.start(1.millis()).unwrap();

    // key send timer
    let mut send_key_timer = device_peripherals.TIM3.counter_ms(&clocks);
    send_key_timer.start(1.secs()).unwrap();

    // interrupt on timer expire
    timer.listen(Event::Update);

    // move time to global variable for use in interrupt handler
    cortex_m::interrupt::free(|cs| {
        *GLOBAL_TIMER.borrow(cs).borrow_mut() = Some(timer)
    });

    // enable the timer interrupt
    unsafe {
        cortex_m::peripheral::NVIC::unmask(Interrupt::TIM2);
    }

    // GPIO
    // setup gpio registers
    let gpioa = device_peripherals.GPIOA.split();
    let gpiod = device_peripherals.GPIOD.split();

    // configure led for timer interrupt and turn on
    let mut led = gpiod.pd15.into_push_pull_output();
    led.set_high();

    // move the led into the global mutex to allow the interrupt to handle it
    // note - interrupt::free() runs the provided closure in a critical state
    cortex_m::interrupt::free(|cs| {
        *GLOBAL_LED.borrow(cs).borrow_mut() = Some(led)
    });

    // USB
    // configure usb settings
    let usb = USB {
        usb_global: device_peripherals.OTG_FS_GLOBAL,
        usb_device: device_peripherals.OTG_FS_DEVICE,
        usb_pwrclk: device_peripherals.OTG_FS_PWRCLK,
        pin_dm: gpioa.pa11.into_alternate(),
        pin_dp: gpioa.pa12.into_alternate(),
        hclk: clocks.hclk(),
    };

    let usb_bus = UsbBus::new(usb, unsafe {&mut EP_MEMORY});

    let mut keyboard = UsbHidClassBuilder::new()
        .add_interface(
                NKROBootKeyboardInterface::default_config(),
        )
        .build(&usb_bus);

    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x1209, 0x0001))
        .manufacturer("Deez Nuts")
        .product("Simple Keyboard")
        .serial_number("TEST")
        .build();

    // Garbage Code Only Welcome Here
    let mut _leds: KeyboardLedsReport;

    loop {
        // send example key every second
        let key = if send_key_timer.wait().is_ok() {
            [Keyboard::A]
        } else {
            [Keyboard::NoEventIndicated]
        };

        keyboard.interface().write_report(key).ok();

        // tick onece per ms
        if key_timer.wait().is_ok() {
            keyboard.interface().tick().unwrap();
        }

        if usb_dev.poll(&mut [&mut keyboard]) {
            match keyboard.interface().read_report() {
                Ok(l) => {
                    _leds = l;
                }
                _ => {}
            }
        }
    }
}
