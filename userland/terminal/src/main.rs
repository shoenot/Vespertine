#![no_std]
#![no_main]

mod term;

use alloc::vec::Vec;
use alloc::vec;
use vespertine_abi::app::termios::*;
use vespertine_abi::protocol::PacketType;
use vespertine_abi::tag::{TAG_APP_TERM, TAG_SYS_PROCMAN, TAG_SYS_SOCKFAC};
use vespertine_abi::{
    AccessRights, HandleGrant, Invocation, ProcessInitPackage, Signal, WaitItem, WaitOp,
};
use vespertine_rt::syscall::{sys_invoke, sys_read, sys_sleep, sys_write_bytes};
use vespertine_rt::thread as rt_thread;
use vespertine_std::fs::walk_path;
use vespertine_std::log::SystemLog;
use vespertine_std::socket::Socket;
use vespertine_std::{Error, fb::Framebuffer};
use vespertine_std::{ErrorKind, Exec, Write, env};

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
    let log = SystemLog::connect();
    let fb = Framebuffer::open()?;
    let info = fb.info();

    let width_chars = (info.width - 2 * PADDING_X) / 8;
    let height_chars = (info.height - 2 * PADDING_Y) / 16;

    log.write_string("Creating sockets".into())?;

    let (term_stdin, app_stdin) = Socket::new_pair()?;
    let (term_stdout, app_stdout) = Socket::new_pair()?;
    let (ctrl_term, app_ctrl) = Socket::new_pair()?;
    let (blink_read, blink_write) = Socket::new_pair()?;

    log.write_string("Created sockets".into())?;

    let mut grid = TerminalGrid {
        width_chars,
        height_chars,
        cursor_x: 0,
        cursor_y: 0,
        cursor_visible: true,
        cursor_blink_on: true,
        termios: Termios::new(),
        can_buffer: Vec::new(),
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
        app_source: term_stdin.handle(),
    };

    grid.clear_screen();

    let kbd_handle = env::source();

    log.write_string("Launching shell".into())?;

    Exec::new("shell")
        .source(app_stdin.handle())
        .sink(app_stdout.handle())
        .root_rights(AccessRights::READ | AccessRights::WRITE | AccessRights::CREATE)
        .grant_new(app_ctrl.handle(), TAG_APP_TERM, AccessRights::all())?
        .inherit_capabilities()
        .spawn()?;

    // Spawn the cursor blinker thread
    let clock = walk_path("/System/Services/Clock", env::root())?;
    rt_thread::spawn(move || {
        let dummy = [1u8; 1];
        loop {
            let _ = sys_sleep(500, clock);
            let _ = sys_write_bytes(blink_write.handle(), &dummy);
        }
    }).map_err(|_| Error {
        kind: ErrorKind::InvalidArgument,
        message: "Failed to spawn blink thread".into(),
    })?;

    let mut wait_items = [
        WaitItem {
            handle: kbd_handle,
            signal: Signal::READABLE,
            pending: Signal(0),
        },
        WaitItem {
            handle: term_stdout.handle(),
            signal: Signal::READABLE,
            pending: Signal(0),
        },
        WaitItem {
            handle: blink_read.handle(),
            signal: Signal::READABLE,
            pending: Signal(0),
        },
        WaitItem {
            handle: ctrl_term.handle(),
            signal: Signal::READABLE,
            pending: Signal(0),
        }
    ];

    let mut vte_parser = vte::Parser::new();
    let mut buf = [0u8; 256];

    loop {
        grid.draw_cursor(grid.cursor_blink_on);

        // block until either kbd or stdout is readable
        let wait_op = WaitOp::Many {
            items_ptr: wait_items.as_mut_ptr() as usize,
            count: wait_items.len(),
        };
        sys_invoke(env::self_handle(), &Invocation::Wait(wait_op))?;

        grid.draw_cursor(false);

        // blink cursor
        if wait_items[2].pending.contains(Signal::READABLE) {
            let mut dummy = [0u8; 1];
            let _ = sys_read(blink_read.handle(), dummy.as_mut_ptr(), 1, 0);
            grid.cursor_blink_on = !grid.cursor_blink_on;
        }

        // kbd input - fwd to app, also echo locally
        if wait_items[0].pending.contains(Signal::READABLE) {
            grid.cursor_blink_on = true; // make it solid while typing
            
            match sys_read(kbd_handle, buf.as_mut_ptr(), buf.len(), 0) {
                Ok(n) if n > 0 => {
                    for &raw_byte in &buf[..n] {
                        let mut processed_byte = raw_byte;
                        let iflag = grid.termios.c_iflag;
                        let lflag = grid.termios.c_lflag;
                        let oflag = grid.termios.c_oflag;

                        let mut should_echo = false;

                        // c_iflag transformations 
                        if check_flag(iflag, ISTRIP) {
                            processed_byte &= 0x7f; 
                        }

                        if check_flag(iflag, IUCLC) && processed_byte.is_ascii_uppercase() {
                            processed_byte = processed_byte.to_ascii_lowercase();
                        }

                        if processed_byte == b'\r' {
                            if check_flag(iflag, IGNCR) {
                                continue;
                            }
                            if check_flag(iflag, ICRNL) {
                                processed_byte = b'\n'
                            }
                        } else if processed_byte == b'\n' {
                            if check_flag(iflag, INLCR) {
                                processed_byte = b'\r'
                            }
                        }

                        
                        // TODO: ISIG handling 
                        
                        // ECHO / ECHONL handling 
                        if check_flag(lflag, ECHO) {
                            should_echo = true;
                        } else if check_flag(lflag, ECHONL) && processed_byte == b'\n' {
                            should_echo = true;
                        }

                        if should_echo && !matches!(processed_byte, b'\x08' | b'\x7f') {
                            if processed_byte == b'\n' && 
                               check_flag(oflag, OPOST) &&
                               check_flag(oflag, ONLCR) {
                                vte_parser.advance(&mut grid, &[b'\r', b'\n']);
                            } else {
                                vte_parser.advance(&mut grid, &[processed_byte]);
                            }
                        }

                        // ICANON
                        if (grid.termios.c_lflag & ICANON) == 0 {
                            // raw mode: send immediately to application
                            let out_buf = [processed_byte];
                            let _ = sys_write_bytes(term_stdin.handle(), &out_buf);
                        } else {
                            // canonical mode: buffer locally until enter/newline
                            match processed_byte {
                                b'\x08' | b'\x7f' => {
                                    if let Some(_popped) = grid.can_buffer.pop() {
                                        if check_flag(lflag, ECHOE) {
                                            vte_parser.advance(&mut grid, &[b'\x08', b' ', b'\x08']);
                                        }
                                    }
                                }
                                b'\n' => {
                                    grid.can_buffer.push(b'\n');
                                    let _ = sys_write_bytes(term_stdin.handle(), &grid.can_buffer);
                                    grid.can_buffer.clear();
                                }
                                other => {
                                    grid.can_buffer.push(other);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // application output
        if wait_items[1].pending.contains(Signal::READABLE) {
            let mut app_buf = [0u8; 256];
            match sys_read(term_stdout.handle(), app_buf.as_mut_ptr(), app_buf.len(), 0) {
                Ok(n) if n > 0 => {
                    let oflag = grid.termios.c_oflag;

                    if check_flag(oflag, OPOST) {
                        for &app_byte in &app_buf[..n] {
                            let mut processed_byte = app_byte;

                            if check_flag(oflag, OLCUC) && processed_byte.is_ascii_lowercase() {
                                processed_byte = processed_byte.to_ascii_uppercase();
                            }

                            if processed_byte == b'\r' {
                                if check_flag(oflag, OCRNL) {
                                    processed_byte = b'\n';
                                } else if check_flag(oflag, ONOCR) && grid.cursor_x == 0 {
                                    continue
                                }
                            } else if processed_byte == b'\n' && check_flag(oflag, ONLCR) {
                                vte_parser.advance(&mut grid, &[b'\r', b'\n']);
                                continue
                            }

                            if processed_byte == b'\n' && check_flag(oflag, ONLRET) {
                                grid.cursor_x = 0;
                            }

                            vte_parser.advance(&mut grid, &[processed_byte]);
                        }
                    } else {
                        // raw mode
                        vte_parser.advance(&mut grid, &app_buf[..n]);
                    }
                },
                Ok(0) => {
                    break;
                },
                _ => {}
            }
        }

        if wait_items[3].pending.contains(Signal::READABLE) {
            match ctrl_term.recv_packet::<TermCommand>() {
                Ok((header, cmd)) => {
                    match cmd {
                        TermCommand::SetTermios(t) => grid.apply_termios(t),
                        TermCommand::GetTermios => {
                            let _ = ctrl_term.send_packet(PacketType::Termios as u32, &grid.termios);
                        },
                        TermCommand::GetWindowSize => {
                            let _ = ctrl_term.send_packet::<(u32, u32)>(PacketType::TermSize as u32, &(width_chars as u32, height_chars as u32));
                            grid.current_bg = 0x0000DD;
                            grid.clear_screen();
                        }
                    }
                },
                Err(_) => {},
            }
        }

        wait_items[0].pending = Signal(0);
        wait_items[1].pending = Signal(0);
        wait_items[2].pending = Signal(0);
        wait_items[3].pending = Signal(0);
    }
    Ok(())
}

impl TerminalGrid {
    pub fn apply_termios(&mut self, new_termios: Termios) {
        let old_icanon = check_flag(self.termios.c_lflag, ICANON);
        let new_icanon = check_flag(new_termios.c_lflag, ICANON);

        self.termios = new_termios;

        if old_icanon && !new_icanon {
            self.can_buffer.clear();
        }
    }
}
