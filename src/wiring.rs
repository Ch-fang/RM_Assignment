//! Complete the two TODOs in this file.

use embassy_stm32::{Peripherals, can::Can, usart::{self, BufferedUart}, gpio::{Flex, Level, Output, Speed}, bind_interrupts, peripherals};
use static_cell::StaticCell;
use crate::swd::Swd;

bind_interrupts!(struct Irqs {
    UART8 => usart::BufferedInterruptHandler<peripherals::UART8>;
});

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
    static TX_BUF: StaticCell<[u8; 256]> = StaticCell::new();
    static RX_BUF: StaticCell<[u8; 256]> = StaticCell::new();
    let tx_buf = TX_BUF.init([0; 256]);
    let rx_buf = RX_BUF.init([0; 256]);

    let mut uart_config = usart::Config::default();
    uart_config.baudrate = 115_200;
    uart_config.data_bits = usart::DataBits::DataBits8;
    uart_config.parity = usart::Parity::ParityNone;
    uart_config.stop_bits = usart::StopBits::STOP1;
    
    let uart = Some(
        BufferedUart::new(
            p.UART8,
            p.PE0,
            p.PE1,
            tx_buf,
            rx_buf,
            Irqs,
            uart_config,
        )
        .unwrap(),
    );

    let can = crate::board::can1(p.FDCAN1, p.PD0, p.PD1);
    (usb, Ports { can, uart }, swd)
}
