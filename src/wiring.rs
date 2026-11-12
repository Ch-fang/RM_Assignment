//! Complete the two TODOs in this file.

use embassy_stm32::{Peripherals, can::Can, usart::BufferedUart, gpio::{Flex, Level, Output, Speed}};

use crate::swd::Swd;

pub struct Ports {
    pub can: Can<'static>,
    pub uart: Option<BufferedUart<'static>>,
}

pub fn configure(p: Peripherals) -> (crate::board::Usb, Ports, Option<Swd>) {
    let usb = crate::board::usb(p.USB_OTG_HS, p.PA12, p.PA11);

    // TODO 1: Pick two exposed 3.3 V GPIOs and pass them to Swd::new.
    // Use Output for SWCLK (initially low), and Flex for bidirectional SWDIO.
    let swclk = Output::new(p.PD15, Level::Low, Speed::VeryHigh);
    let swdio = Flex::new(p.PD14);
    let swd = Some(Swd::new(swclk, swdio));

    // TODO 2: Configure a buffered UART at 115200 baud, 8N1.
    // Identify the peripheral, RX/TX pins and interrupt from the schematic.
    // Bind the interrupt and provide static RX and TX buffers.
    let uart = None;

    let can = crate::board::can1(p.FDCAN1, p.PD0, p.PD1);
    (usb, Ports { can, uart }, swd)
}
