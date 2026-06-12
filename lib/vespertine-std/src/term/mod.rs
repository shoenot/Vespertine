use core::mem::forget;

use vespertine_abi::{
    app::termios::*,
    protocol::{PacketType, VESPER_MAGIC},
    tag::CAP_APP_TERMCTRL,
};

use crate::{Error, ErrorKind, env, socket::Socket};

fn get_ctrl_sock() -> Result<Socket, Error> {
    let term = env::capability(CAP_APP_TERMCTRL)
        .ok_or(Error {
            kind: ErrorKind::NotFound,
            message: "Terminal control socket not found".into(),
        })?
        .id;
    Ok(Socket::from_handle(term))
}

pub fn get_terminfo() -> Result<Termios, Error> {
    let sock = get_ctrl_sock()?;
    sock.send_packet(PacketType::TermCommand as u32, &TermCommand::GetTermios)?;
    let (_, termios) = sock.recv_packet::<Termios>()?;
    forget(sock);
    Ok(termios)
}

pub fn set_terminfo(t: Termios) -> Result<(), Error> {
    let sock = get_ctrl_sock()?;
    let cmd = TermCommand::SetTermios(t);
    let ret = sock.send_packet(PacketType::TermCommand as u32, &cmd);
    forget(sock);
    ret
}

pub fn get_termsize() -> Result<(usize, usize), Error> {
    let sock = get_ctrl_sock()?;
    sock.send_packet(PacketType::TermCommand as u32, &TermCommand::GetWindowSize)?;
    let (_, winsz) = sock.recv_packet::<(usize, usize)>()?;
    forget(sock);
    Ok(winsz)
}

pub fn get_term_cursor_position() -> Result<(usize, usize), Error> {
    let sock = get_ctrl_sock()?;
    sock.send_packet(PacketType::TermCommand as u32, &TermCommand::GetCursorPosition)?;
    let (_, (row, column)) = sock.recv_packet::<(usize, usize)>()?;
    forget(sock);
    Ok((row, column))
}

pub fn clear_term_screen() -> Result<(), Error> {
    let sock = get_ctrl_sock()?;
    sock.send_packet(PacketType::TermCommand as u32, &TermCommand::Clear)?;
    Ok(())
}

pub fn set_raw_mode() -> Result<(), Error> {
    let mut t = get_terminfo()?;
    t.c_iflag &= !(IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL | IXON);
    t.c_oflag &= !OPOST;
    t.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG);
    t.c_cflag &= !(CSIZE | PARENB);
    t.c_cflag |= CS8;
    set_terminfo(t)
}

pub fn check_raw_mode() -> Result<bool, Error> {
    let def = Termios::default();
    let t = get_terminfo()?;
    if t != def { return Ok(true) } else { return Ok(false) };
}

pub fn unset_raw_mode() -> Result<(), Error> {
    let t = Termios::default();
    set_terminfo(t)?;
    Ok(())
}
