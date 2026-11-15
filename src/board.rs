use core::num::{NonZeroU8, NonZeroU16};

use embassy_stm32::{
    Config, Peri, Peripherals, bind_interrupts,
    can::{
        Can, CanConfigurator, IT0InterruptHandler, IT1InterruptHandler,
        config::{DataBitTiming, FrameTransmissionConfig, NominalBitTiming},
    },
    peripherals,
    rcc::{
        Hse, HseMode, Hsi48Config, Pll, PllDiv, PllMul, PllPreDiv, PllSource,
        mux::{Fdcansel, Usbsel},
    },
    time::Hertz,
    usb::{self, Driver},
};
use static_cell::StaticCell;

pub type Usb = Driver<'static, peripherals::USB_OTG_HS>;

bind_interrupts!(struct UsbIrqs {
    OTG_HS => usb::InterruptHandler<peripherals::USB_OTG_HS>;
});

bind_interrupts!(struct CanIrqs {
    FDCAN1_IT0 => IT0InterruptHandler<peripherals::FDCAN1>;
    FDCAN1_IT1 => IT1InterruptHandler<peripherals::FDCAN1>;
});

/// FDCAN kernel clock from PLL2_Q: 24 MHz / 12 * 240 / 6 = 80 MHz.
pub const FDCAN_KERNEL_HZ: u32 = 80_000_000;

// 1 Mbit/s arbitration, 75% sample point.
const NOMINAL_PRESCALER: u16 = 1;
const NOMINAL_SEG1: u8 = 59;
const NOMINAL_SEG2: u8 = 20;
const NOMINAL_SJW: u8 = 20;

// 5 Mbit/s data phase, 75% sample point.
const DATA_PRESCALER: u16 = 1;
const DATA_SEG1: u8 = 11;
const DATA_SEG2: u8 = 4;
const DATA_SJW: u8 = 4;

// TDC units are mtq; with DBRP=1 one mtq equals one data-phase tq (12.5 ns).
const TDC_OFFSET: u8 = 12;
const TDC_FILTER: u8 = 15;

const _: () = {
    assert!(
        FDCAN_KERNEL_HZ as u64
            / NOMINAL_PRESCALER as u64
            / (1 + NOMINAL_SEG1 as u64 + NOMINAL_SEG2 as u64)
            == 1_000_000
    );
    assert!(
        FDCAN_KERNEL_HZ as u64
            / DATA_PRESCALER as u64
            / (1 + DATA_SEG1 as u64 + DATA_SEG2 as u64)
            == 5_000_000
    );
    assert!(TDC_FILTER > TDC_OFFSET);
};

/// The clock tree is provided. Keep SYSCLK at its default HSI setting.
pub fn init() -> Peripherals {
    let mut config = Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz(24_000_000),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll2 = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV12,
        mul: PllMul::MUL240,
        divp: Some(PllDiv::DIV2),
        divq: Some(PllDiv::DIV6),
        divr: Some(PllDiv::DIV2),
    });
    config.rcc.mux.fdcansel = Fdcansel::PLL2_Q;
    config.rcc.hsi48 = Some(Hsi48Config {
        sync_from_usb: true,
    });
    config.rcc.mux.usbsel = Usbsel::HSI48;
    embassy_stm32::init(config)
}

pub fn usb(
    otg: embassy_stm32::Peri<'static, peripherals::USB_OTG_HS>,
    dp: embassy_stm32::Peri<'static, peripherals::PA12>,
    dm: embassy_stm32::Peri<'static, peripherals::PA11>,
) -> Usb {
    static OUT: StaticCell<[u8; 512]> = StaticCell::new();
    let mut config = usb::Config::default();
    config.vbus_detection = false;
    Driver::new_fs(otg, UsbIrqs, dp, dm, OUT.init([0; 512]), config)
}

/// CAN1 setup: 1 Mbit/s arbitration, 5 Mbit/s FD+BRS data phase, TDC enabled.
pub fn can1(
    fdcan: Peri<'static, peripherals::FDCAN1>,
    rx: Peri<'static, peripherals::PD0>,
    tx: Peri<'static, peripherals::PD1>,
) -> Can<'static> {
    let mut can = CanConfigurator::new(fdcan, rx, tx, CanIrqs);

    let nominal = NominalBitTiming {
        prescaler: NonZeroU16::new(NOMINAL_PRESCALER).unwrap(),
        seg1: NonZeroU8::new(NOMINAL_SEG1).unwrap(),
        seg2: NonZeroU8::new(NOMINAL_SEG2).unwrap(),
        sync_jump_width: NonZeroU8::new(NOMINAL_SJW).unwrap(),
    };
    let data = DataBitTiming {
        // embassy-stm32 0.6.0 accepts this flag but never programs TDCR or DBTP.TDC.
        // enable_tdc() below applies the actual compensation after initialisation.
        transceiver_delay_compensation: true,
        prescaler: NonZeroU16::new(DATA_PRESCALER).unwrap(),
        seg1: NonZeroU8::new(DATA_SEG1).unwrap(),
        seg2: NonZeroU8::new(DATA_SEG2).unwrap(),
        sync_jump_width: NonZeroU8::new(DATA_SJW).unwrap(),
    };
    let config = can
        .config()
        .set_nominal_bit_timing(nominal)
        .set_data_bit_timing(data)
        .set_frame_transmit(FrameTransmissionConfig::AllowFdCanAndBRS);
    can.set_config(config);

    let can = can.into_normal_mode();
    enable_tdc();
    can
}

/// Program TDCR and DBTP.TDC through the PAC.
///
/// `apply_config()` writes DBTP with a plain `write()`, which clears TDC, so the
/// registers must be set by re-entering init mode after the driver is started.
fn enable_tdc() {
    let regs = stm32_metapac::FDCAN1;
    cortex_m::interrupt::free(|_| {
        regs.cccr().modify(|w| w.set_init(true));
        // Bounded wait: never hang boot before USB enumerates.
        for _ in 0..1_000_000 {
            if regs.cccr().read().init() {
                break;
            }
        }

        // CCE is only writable while INIT is set.
        regs.cccr().modify(|w| w.set_cce(true));
        regs.tdcr().write(|w| {
            w.set_tdcf(TDC_FILTER);
            w.set_tdco(TDC_OFFSET);
        });
        regs.dbtp().modify(|w| w.set_tdc(true));

        regs.cccr().modify(|w| {
            w.set_cce(false);
            w.set_init(false);
        });
        for _ in 0..1_000_000 {
            if !regs.cccr().read().init() {
                break;
            }
        }
    });
}

/// Raw NBTP, DBTP, CCCR and TDCR values, used by the `FD INFO` host command.
pub fn fd_regs() -> (u32, u32, u32, u32) {
    let regs = stm32_metapac::FDCAN1;
    (
        regs.nbtp().read().0,
        regs.dbtp().read().0,
        regs.cccr().read().0,
        regs.tdcr().read().0,
    )
}
