#![allow(dead_code)]

use core::hint::spin_loop;
use alloc::boxed::Box;

use async_trait::async_trait;
use hal::io::{
    inb,
    outb,
};
use vespertine_abi::{AccessRights, BrokerOp, Invocation, SYSTEM_USER, tag::CAP_SERIAL_WRITE};

use crate::{executor::{Executor, async_sleep::sleep_async}, object::{help::RightsWrapper, invoke::InvocationError, ipc::socket::SocketEndpoint, obj::KernelObject}, process::current_process, security::permissions::FilePermissions, sync::TicketLock};

//////////////
/// DRIVER ///
//////////////

const PORT: u16 = 0x3f8;

pub fn init_serial() {
    unsafe {
        outb(PORT + 1, 0x00); // disable all interrupts
        outb(PORT + 3, 0x80); // enable dlab (set baud rate divisor)
        outb(PORT + 0, 0x03); // set divisor to 3 (lo byte) 38400 baud 
        outb(PORT + 1, 0x00); //                  (hi byte)
        outb(PORT + 3, 0x03); // 8 bits, no parity, one stop bit
        outb(PORT + 2, 0xC7); // enable FIFO, clear them, with 14 byte threshold
        outb(PORT + 4, 0x0B); // IRQs enabled, RTS/DSR set 
    }
}
pub struct SerialWriter;
impl SerialWriter {
    pub fn new() -> SerialWriter { SerialWriter }
}

static TX_LOCK: TicketLock<()> = TicketLock::new(());

fn try_write_byte(byte: u8) -> bool {
    let _guard = TX_LOCK.lock();

    unsafe {
        if inb(PORT + 5) & 0x20 == 0 {
            return false;
        }

        outb(PORT, byte);
    }

    true
}

fn write_byte_blocking(byte: u8) {
    while !try_write_byte(byte) {
        spin_loop();
    }
}

pub fn log_to_serial(s: &str) {
    for byte in s.bytes() {
        write_byte_blocking(byte);
    }
}

impl core::fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.chars() {
            write_byte_blocking(ch as u8);
        }
        Ok(())
    }
}

pub fn log_u32_to_serial(mut n: u32) {
    let mut buffer = [0u8; 16];
    let mut i = buffer.len();
    while n > 0 && i > 0 {
        i -= 1;
        buffer[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    let numstr = core::str::from_utf8(&buffer[i..]).unwrap();
    log_to_serial(&numstr);
}

pub fn log_u64_to_serial(mut n: u64) {
    let mut buffer = [0u8; 24];
    let mut i = buffer.len();
    while n > 0 && i > 0 {
        i -= 1;
        buffer[i] = (n % 10) as u8 + b'0';
        n /= 10;
    }
    let numstr = core::str::from_utf8(&buffer[i..]).unwrap();
    log_to_serial(&numstr);
}

/////////////////////
/// KERNEL OBJECT ///
/////////////////////

#[derive(Debug)]
pub struct SerialDevice;

#[async_trait]
impl KernelObject for SerialDevice {
    fn type_name(&self) ->  &'static str { "Serial" }

    fn permissions(&self) -> Option<FilePermissions> {
        Some(FilePermissions { 
            owner: SYSTEM_USER, 
            owner_rights: AccessRights::READ, 
            other_rights: AccessRights::READ, 
        })
    }

    async fn invoke(&self, invocation: Invocation, calling_rights: AccessRights) -> Result<usize, InvocationError> {
        match invocation {
            Invocation::Broker(BrokerOp::Request { capability, requested_rights }) => {
                calling_rights.err_if_no(AccessRights::CREATE)?;

                if capability != CAP_SERIAL_WRITE {
                    return Err(InvocationError::InvalidArgument);
                }

                if requested_rights != AccessRights::WRITE {
                    return Err(InvocationError::AccessDenied);
                }

                let caller = current_process().ok_or(InvocationError::InvalidHandle)?;
                let (client, driver) = SocketEndpoint::new_pair();
                let handle = caller.handles.write().insert(client, AccessRights::WRITE);

                Executor::new().spawn(async move {
                    let mut buf = [0u8; 128];
                    while let Ok(1..) = drain_serial(&driver, &mut buf).await {}
                });

                Ok(handle.0)
            },
            _ => Err(InvocationError::UnsupportedOperation),
        }
    }
}

async fn drain_serial(driver: &SocketEndpoint, buf: &mut [u8]) -> Result<usize, InvocationError> {
    let count = driver.read_internal(buf).await?;

    for &byte in &buf[..count] {
        while !try_write_byte(byte) {
            sleep_async(1).await;
        }
    }

    Ok(count)
}
