#![no_std]
#![no_main]

mod term;

use alloc::vec;
use alloc::vec::Vec;

use vespertine_abi::app::termios::*;
use vespertine_abi::protocol::PacketType;
use vespertine_abi::tag::{
    CAP_APP_TERMCTRL,
    CAP_LAUNCHER_EXEC,
    CAP_LAUNCHER_GRANT,
};
use vespertine_abi::{
    AccessRights,
    ProcessInitPackage,
};
use vespertine_rt::syscall::{
    sys_read,
    sys_set_read_policy,
    sys_write_bytes,
};
use vespertine_rt::thread as rt_thread;
use vespertine_std::clock::Time;
use vespertine_std::fb::Framebuffer;
use vespertine_std::log::SystemLog;
use vespertine_std::proc::Waiter;
use vespertine_std::socket::Socket;
use vespertine_std::{
    Error,
    ErrorKind,
    Exec,
    Write,
    env,
};

use crate::term::{
    Cell,
    PADDING_X,
    PADDING_Y,
    TerminalGrid,
};

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

    let width_cols = info.width - 2 * PADDING_X;
    let height_rows = info.height - 2 * PADDING_Y;
    let width_chars = width_cols / 8;
    let height_chars = height_rows / 16;

    let (term_stdin, app_stdin) = Socket::new_pair()?;
    let (term_stdout, app_stdout) = Socket::new_pair()?;
    let (ctrl_term, app_ctrl) = Socket::new_pair()?;
    let (blink_read, blink_write) = Socket::new_pair()?;

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
        cells: vec![Cell { char: ' ', fg: FG_COLOR, bg: BG_COLOR }; width_chars * height_chars],
        fb,
        app_source: term_stdin.handle(),
    };

    grid.clear_screen();

    let kbd_handle = env::source();

    log.write_string("Launching dusk".into())?;

    Exec::new("dusk".into())
        .source(app_stdin.handle())
        .sink(app_stdout.handle())
        .cwd(env::cwd(), AccessRights::all())
        .root_rights(AccessRights::all())
        .grant(CAP_LAUNCHER_EXEC, AccessRights::READ | AccessRights::WRITE | AccessRights::EXECUTE)?
        .grant(CAP_LAUNCHER_GRANT, AccessRights::MUTATE)?
        .grant_new(app_ctrl.handle(), CAP_APP_TERMCTRL, AccessRights::READ | AccessRights::WRITE)?
        .inherit_capabilities()
        .spawn()?;

    // Spawn the cursor blinker thread
    rt_thread::spawn(move || {
        let dummy = [1u8; 1];
        loop {
            if Time::sleep_ms(500).is_err() {
                break;
            }
            if sys_write_bytes(blink_write.handle(), &dummy).is_err() {
                break;
            }
        }
    })
    .map_err(|_| Error { kind: ErrorKind::InvalidArgument, message: "Failed to spawn blink thread".into() })?;

    let mut waiter =
        Waiter::new().readable(kbd_handle).readable(term_stdout.handle()).readable(blink_read.handle()).readable(ctrl_term.handle());

    let mut vte_parser = vte::Parser::new();
    let mut buf = [0u8; 256];

    loop {
        grid.draw_cursor(grid.cursor_blink_on);

        // block until either kbd or stdout is readable
        waiter.wait()?;
        grid.draw_cursor(false);

        // blink cursor
        if waiter.ready(2) {
            let mut dummy = [0u8; 1];
            let _ = sys_read(blink_read.handle(), dummy.as_mut_ptr(), 1, 0);
            grid.cursor_blink_on = !grid.cursor_blink_on;
        }

        // kbd input - fwd to app, also echo locally
        if waiter.ready(0) {
            grid.cursor_blink_on = true; // make it solid while typing

            match sys_read(kbd_handle, buf.as_mut_ptr(), buf.len(), 0) {
                Ok(n) if n > 0 => {
                    let mut raw_trans_buffer = Vec::new();

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
                                continue; // ignore carriage return 
                            }
                            if check_flag(iflag, ICRNL) {
                                processed_byte = b'\n' // \r -> \n
                            }
                        } else if processed_byte == b'\n' {
                            if check_flag(iflag, INLCR) {
                                processed_byte = b'\r' // \n -> \r
                            }
                        }

                        // TODO: ISIG handling
                        if check_flag(lflag, ISIG) {
                            if processed_byte == grid.termios.c_cc[VINTR as usize] {}
                        }

                        // ECHO / ECHONL handling
                        if check_flag(lflag, ECHO) {
                            should_echo = true;
                        } else if check_flag(lflag, ECHONL) && processed_byte == b'\n' {
                            should_echo = true;
                        }

                        if should_echo && !matches!(processed_byte, b'\x08' | b'\x7f') {
                            if processed_byte == b'\n' && check_flag(oflag, OPOST) && check_flag(oflag, ONLCR) {
                                vte_parser.advance(&mut grid, &[b'\r', b'\n']);
                            } else {
                                vte_parser.advance(&mut grid, &[processed_byte]);
                            }
                        }

                        // ICANON
                        if (grid.termios.c_lflag & ICANON) == 0 {
                            // raw mode: accumulate into raw mode buffer for atomicity
                            raw_trans_buffer.push(processed_byte);
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

                    if !check_flag(grid.termios.c_lflag, ICANON) && !raw_trans_buffer.is_empty() {
                        let _ = sys_write_bytes(term_stdin.handle(), &raw_trans_buffer);
                    }
                }
                _ => {}
            }
        }

        // application output
        if waiter.ready(1) {
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
                                    continue;
                                }
                            } else if processed_byte == b'\n' && check_flag(oflag, ONLCR) {
                                vte_parser.advance(&mut grid, &[b'\r', b'\n']);
                                continue;
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
                }
                Ok(0) => {
                    break;
                }
                _ => {}
            }
        }

        if waiter.ready(3) {
            match ctrl_term.recv_packet::<TermCommand>() {
                Ok((_header, cmd)) => match cmd {
                    TermCommand::SetTermios(t) => {
                        let canonical = check_flag(t.c_lflag, ICANON);

                        let min = if canonical { 1 } else { t.c_cc[VMIN] as usize };

                        let timeout_ds = if canonical { 0 } else { t.c_cc[VTIME] as usize };

                        let current_is_raw = grid.termios != Termios::default();
                        let new_is_default = t == Termios::default();

                        grid.apply_termios(t);
                        let _ = sys_set_read_policy(term_stdin.handle(), min, timeout_ds);
                        if current_is_raw && new_is_default {
                            grid.clear_screen();
                        }
                    }
                    TermCommand::GetTermios => {
                        let _ = ctrl_term.send_packet(PacketType::Termios as u32, &grid.termios);
                    }
                    TermCommand::GetWindowSize => {
                        let wsize = WinSize {
                            ws_row: height_chars as u16,
                            ws_col: width_chars as u16,
                            ws_xpixel: width_cols as u16,
                            ws_ypixel: height_rows as u16,
                        };
                        let _ = ctrl_term.send_packet::<WinSize>(PacketType::TermSize as u32, &wsize);
                    }
                    TermCommand::GetCursorPosition => {
                        let cursor = (grid.cursor_y, grid.cursor_x);
                        let _ = ctrl_term.send_packet::<(usize, usize)>(PacketType::TermCursorPos as u32, &cursor);
                    }
                    TermCommand::Clear => {
                        grid.clear_screen();
                    }
                },
                Err(_) => {}
            }
        }

        waiter.clear();
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
