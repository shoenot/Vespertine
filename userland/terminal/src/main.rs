#![no_std]
#![no_main]

mod term;

use alloc::vec;
use vespertine_abi::tag::{TAG_SYS_PROCMAN, TAG_SYS_SOCKFAC};
use vespertine_abi::{
    AccessRights, HandleGrant, Invocation, ProcessInitPackage, Signal, WaitItem, WaitOp,
};
use vespertine_rt::syscall::{sys_create_socket, sys_invoke, sys_read, sys_write_bytes};
use vespertine_std::{Error, fb::Framebuffer};
use vespertine_std::{ErrorKind, Exec, env};

use crate::term::Cell;
use crate::term::{PADDING_X, PADDING_Y, TerminalGrid};

extern crate alloc;

pub const FG_COLOR: u32 = 0xe0ddd8;
pub const BG_COLOR: u32 = 0x11080d;

#[unsafe(no_mangle)]
pub extern "sysv64" fn main(pkg_ptr: *const ProcessInitPackage) {
    let pkg = unsafe { &*pkg_ptr };
    if let Err(e) = run(pkg) {
        let _ = e; // nothing to print to bc we are the terminal 
    }
}

#[unsafe(no_mangle)]
fn run(_pkg_ptr: *const ProcessInitPackage) -> Result<(), Error> {
    let sf = env::find_tag(TAG_SYS_SOCKFAC)
        .ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "SockFac not passed to terminal".into(),
        })?
        .id;
    let pm = env::find_tag(TAG_SYS_PROCMAN)
        .ok_or(Error {
            kind: ErrorKind::AccessDenied,
            message: "ProcMan not passed to terminal".into(),
        })?
        .id;

    let fb = Framebuffer::open()?;
    let info = fb.info();

    let width_chars = (info.width - 2 * PADDING_X) / 8;
    let height_chars = (info.height - 2 * PADDING_Y) / 16;

    let shell_stdin_packed = sys_create_socket(sf)?;
    let shell_stdout_packed = sys_create_socket(sf)?;

    let shell_stdin_write = shell_stdin_packed.1;
    let shell_stdin_read = shell_stdin_packed.0;
    let shell_stdout_read = shell_stdout_packed.0;
    let shell_stdout_write = shell_stdout_packed.1;

    let mut grid = TerminalGrid {
        width_chars,
        height_chars,
        cursor_x: 0,
        cursor_y: 0,
        input_len: 0,
        raw_mode: false,
        cursor_visible: true,
        cursor_blink_on: true,
        current_fg: FG_COLOR,
        current_bg: BG_COLOR,
        cells: vec![
            Cell {
                char: ' ',
                fg: FG_COLOR,
                bg: BG_COLOR
            };
            width_chars * height_chars
        ],
        fb,
        shell_source: shell_stdin_write,
    };

    grid.clear_screen();

    let kbd_handle = env::source();

    let sf_grant = HandleGrant {
        id: sf,
        rights: AccessRights::all(),
        tag: TAG_SYS_SOCKFAC,
    };
    let pm_grant = HandleGrant {
        id: pm,
        rights: AccessRights::all(),
        tag: TAG_SYS_PROCMAN,
    };

    Exec::new("shell")
        .source(shell_stdin_read)
        .sink(shell_stdout_write)
        .root_rights(AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE)
        .grant(pm_grant)
        .grant(sf_grant)
        .spawn()?;

    let mut vte_parser = vte::Parser::new();
    let mut buf = [0u8; 256];

    let mut wait_items = [
        WaitItem {
            handle: kbd_handle,
            signal: Signal::READABLE,
            pending: Signal(0),
        },
        WaitItem {
            handle: shell_stdout_read,
            signal: Signal::READABLE,
            pending: Signal(0),
        },
    ];

    loop {
        grid.draw_cursor(true);
        // block until either kbd or stdout is readable
        let wait_op = WaitOp::Many {
            items_ptr: wait_items.as_mut_ptr() as usize,
            count: wait_items.len(),
        };
        sys_invoke(env::self_handle(), &Invocation::Wait(wait_op))?;

        grid.draw_cursor(false);
        // kbd input - fwd to shell, also echo locally
        if wait_items[0].pending.contains(Signal::READABLE) {
            match sys_read(kbd_handle, buf.as_mut_ptr(), buf.len(), 0) {
                Ok(n) if n > 0 => {
                    if grid.raw_mode {
                        let _ = sys_write_bytes(shell_stdin_write, &buf[..n]);
                    } else {
                        let first_char = buf[0];
                        if first_char == b'\x08' {
                            // backspace
                            if grid.input_len > 0 {
                                grid.input_len -= 1;
                                vte_parser.advance(&mut grid, &buf[..n]);
                                let _ = sys_write_bytes(shell_stdin_write, &buf[..n]);
                            }
                        } else {
                            // reset input_len on enter
                            if first_char == b'\n' || first_char == b'\r' {
                                grid.input_len = 0;
                            } else if first_char >= 32 || first_char == b'\t' {
                                // increment len by number of read bytes
                                grid.input_len += n;
                            }
                            vte_parser.advance(&mut grid, &buf[..n]);
                            let _ = sys_write_bytes(shell_stdin_write, &buf[..n]);
                        }
                    }
                }
                _ => {}
            }
        }

        // shell output
        if wait_items[1].pending.contains(Signal::READABLE) {
            grid.input_len = 0;
            match sys_read(shell_stdout_read, buf.as_mut_ptr(), buf.len(), 0) {
                Ok(n) if n > 0 => {
                    vte_parser.advance(&mut grid, &buf[..n]);
                }
                Ok(0) => {
                    break;
                }
                _ => {}
            }
        }

        wait_items[0].pending = Signal(0);
        wait_items[1].pending = Signal(0);
    }
    Ok(())
}
