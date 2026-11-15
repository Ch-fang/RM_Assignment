use core::fmt::Write;
use embassy_stm32::can::{
    Can, Frame,
    frame::{FdFrame, Header},
};
use embassy_time::{Duration, with_timeout};
use embedded_can::{Frame as _, Id, StandardId};
use heapless::String;

use crate::board;
use crate::swd::Swd;

type Reply = String<192>;

const MAX_FD_PAYLOAD: usize = 64;

pub struct Line {
    bytes: [u8; 256],
    len: usize,
}

impl Line {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 256],
            len: 0,
        }
    }

    pub fn push(&mut self, byte: u8) -> Option<&str> {
        if byte == b'\n' {
            let n = core::mem::take(&mut self.len);
            // A full buffer may contain a truncated command. Discard that line.
            if n == self.bytes.len() {
                return Some("");
            }
            return Some(core::str::from_utf8(&self.bytes[..n]).unwrap_or_default());
        }
        if self.len < self.bytes.len() {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
        None
    }
}

fn fixed(text: &str) -> Reply {
    Reply::try_from(text).expect("fixed reply fits")
}

async fn receive_fd(can: &mut Can<'_>) -> Result<FdFrame, &'static str> {
    match with_timeout(Duration::from_millis(100), can.read_fd()).await {
        Ok(Ok(envelope)) => Ok(envelope.frame),
        Ok(Err(_)) => Err("ERR CAN RX\n"),
        Err(_) => Err("EMPTY\n"),
    }
}

pub async fn execute(command: &str, can: &mut Can<'_>, swd: &mut Option<Swd>) -> Reply {
    let mut words = command.split_ascii_whitespace();
    let mut out = Reply::new();
    match (words.next(), words.next()) {
        (Some("PING"), None) => return fixed("OK DM-MC02\n"),
        (Some("SWD"), Some("ID")) if words.next().is_none() => {
            let Some(swd) = swd.as_mut() else {
                return fixed("ERR SWD_NOT_CONFIGURED\n");
            };
            match swd.read_dpidr() {
                Some(id) => writeln!(out, "OK DPIDR {id:08X}").expect("SWD reply fits"),
                None => return fixed("ERR SWD\n"),
            }
        }
        (Some("CAN"), Some("TX")) => {
            let (Some(id), Some(hex), None) = (words.next(), words.next(), words.next()) else {
                return fixed("ERR SYNTAX\n");
            };
            let Some(frame) = parse_frame(id, hex) else {
                return fixed("ERR FRAME\n");
            };
            return match with_timeout(Duration::from_millis(100), can.write(&frame)).await {
                Ok(None) => fixed("OK QUEUED\n"),
                _ => fixed("ERR CAN TX\n"),
            };
        }
        (Some("CAN"), Some("RX")) if words.next().is_none() => {
            let frame = match receive_fd(can).await {
                Ok(frame) => frame,
                Err(reply) => return fixed(reply),
            };
            let Id::Standard(id) = *frame.id() else {
                return fixed("ERR UNSUPPORTED_FRAME\n");
            };
            if frame.is_remote_frame() {
                return fixed("ERR UNSUPPORTED_FRAME\n");
            }
            if frame.header().fdcan() {
                write_fd_frame(&mut out, id.as_raw(), &frame);
            } else {
                write_classic_frame(&mut out, id.as_raw(), &frame);
            }
        }
        (Some("FD"), Some("TX")) => {
            let (Some(id), Some(hex), None) = (words.next(), words.next(), words.next()) else {
                return fixed("ERR SYNTAX\n");
            };
            let frame = match parse_fd_frame(id, hex) {
                Ok(frame) => frame,
                Err(reply) => return fixed(reply),
            };
            return match with_timeout(Duration::from_millis(100), can.write_fd(&frame)).await {
                Ok(_) => fixed("OK FD QUEUED\n"),
                Err(_) => fixed("ERR CAN TX\n"),
            };
        }
        (Some("FD"), Some("RX")) if words.next().is_none() => {
            let frame = match receive_fd(can).await {
                Ok(frame) => frame,
                Err(reply) => return fixed(reply),
            };
            let Id::Standard(id) = *frame.id() else {
                return fixed("ERR UNSUPPORTED_FRAME\n");
            };
            if frame.is_remote_frame() {
                return fixed("ERR UNSUPPORTED_FRAME\n");
            }
            write_fd_frame(&mut out, id.as_raw(), &frame);
        }
        (Some("FD"), Some("INFO")) if words.next().is_none() => {
            let (nbtp, dbtp, cccr, tdcr) = board::fd_regs();
            writeln!(
                out,
                "OK FD NBTP={nbtp:08X} DBTP={dbtp:08X} CCCR={cccr:08X} TDCR={tdcr:08X}"
            )
            .expect("FD info fits");
        }
        _ => return fixed("ERR SYNTAX\n"),
    }
    out
}

fn write_payload(out: &mut Reply, data: &[u8]) {
    if data.is_empty() {
        out.push('-').expect("empty payload fits");
    }
    for byte in data {
        write!(out, "{byte:02X}").expect("payload fits");
    }
}

fn write_classic_frame(out: &mut Reply, id: u16, frame: &FdFrame) {
    write!(out, "OK RX {id:03X} ").expect("CAN prefix fits");
    write_payload(out, frame.data());
    out.push('\n').expect("line ending fits");
}

fn write_fd_frame(out: &mut Reply, id: u16, frame: &FdFrame) {
    let fd = frame.header().fdcan() as u8;
    let brs = frame.header().bit_rate_switching() as u8;
    write!(
        out,
        "OK RXFD {id:03X} F{fd} B{brs} {:02} ",
        frame.data().len()
    )
    .expect("FD prefix fits");
    write_payload(out, frame.data());
    out.push('\n').expect("line ending fits");
}

fn parse_frame(id: &str, payload: &str) -> Option<Frame> {
    let id = u16::from_str_radix(id, 16).ok()?;
    let mut bytes = [0; 8];
    let payload = if payload == "-" { "" } else { payload };
    if !payload.is_ascii() || payload.len() > 16 || !payload.len().is_multiple_of(2) {
        return None;
    }
    for (i, byte) in bytes[..payload.len() / 2].iter_mut().enumerate() {
        *byte = u8::from_str_radix(&payload[i * 2..i * 2 + 2], 16).ok()?;
    }
    Frame::new_standard(id, &bytes[..payload.len() / 2]).ok()
}

fn parse_fd_frame(id: &str, payload: &str) -> Result<FdFrame, &'static str> {
    let id = u16::from_str_radix(id, 16).map_err(|_| "ERR SYNTAX\n")?;
    let id = StandardId::new(id).ok_or("ERR SYNTAX\n")?;
    let payload = if payload == "-" { "" } else { payload };
    if !payload.is_ascii() || !payload.len().is_multiple_of(2) || payload.len() > MAX_FD_PAYLOAD * 2
    {
        return Err("ERR FD FRAME\n");
    }
    let len = payload.len() / 2;
    if !matches!(len, 0..=8 | 12 | 16 | 20 | 24 | 32 | 48 | 64) {
        return Err("ERR FD LEN\n");
    }
    let mut bytes = [0; MAX_FD_PAYLOAD];
    for (i, byte) in bytes[..len].iter_mut().enumerate() {
        *byte = u8::from_str_radix(&payload[i * 2..i * 2 + 2], 16).map_err(|_| "ERR FD FRAME\n")?;
    }
    FdFrame::new(
        Header::new_fd(id.into(), len as u8, false, true),
        &bytes[..len],
    )
    .map_err(|_| "ERR FD FRAME\n")
}
