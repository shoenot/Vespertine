use alloc::vec::Vec;
use vespertine_abi::key::{KeyCode, KeyEvent, Modifiers};


pub fn translate_key_event(event: &KeyEvent, out: &mut Vec<u8>) {
    if !event.is_press() {
        return;
    }

    // ignore standalone modifier
    match event.code {
        KeyCode::LeftShift
        | KeyCode::RightShift
        | KeyCode::LeftCtrl
        | KeyCode::RightCtrl
        | KeyCode::LeftAlt
        | KeyCode::RightAlt
        | KeyCode::LeftSuper
        | KeyCode::RightSuper
        | KeyCode::CapsLock
        | KeyCode::Menu
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::Unknown => return,
        _ => {},
    }

    let shift = event.mods.contains(Modifiers::SHIFT);
    let alt = event.mods.contains(Modifiers::ALT);
    let ctrl = event.mods.contains(Modifiers::CTRL);
    let super_key = event.mods.contains(Modifiers::SUPER);

    let mod_param = 1 
        + (if shift { 1 } else { 0 })
        + (if alt { 2 } else { 0 })
        + (if ctrl { 4 } else { 0 })
        + (if super_key { 8 } else { 0 });

    match event.code {
        KeyCode::Enter | KeyCode::KpEnter => {
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[13;{}u", mod_param).as_bytes());
            } else {
                out.push(b'\r');
            }
            return
        },
        KeyCode::Tab => {
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[9;{}u", mod_param).as_bytes());
            } else {
                out.push(b'\t');
            }
            return;
        },
        KeyCode::Backspace => {
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[127;{}u", mod_param).as_bytes());
            } else {
                out.push(b'\x08');
            }
            return;
        },
        KeyCode::Escape => {
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[27;{}u", mod_param).as_bytes());
            } else {
                out.push(b'\x1b');
            }
            return;
        },
        _ => {}
    }

    let nav = match event.code {
        KeyCode::Up => Some(('A', 0)),
        KeyCode::Down => Some(('B', 0)),
        KeyCode::Right => Some(('C', 0)),
        KeyCode::Left => Some(('D', 0)),
        KeyCode::Home => Some(('H', 0)),
        KeyCode::End => Some(('F', 0)),
        KeyCode::Insert => Some(('~', 2)),
        KeyCode::Delete => Some(('~', 3)),
        KeyCode::PgUp => Some(('~', 5)),
        KeyCode::PgDown => Some(('~', 6)),
        _ => None,
    };
    
    if let Some((final_char, tilde_num)) = nav {
        if tilde_num > 0 {
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[{};{}~", tilde_num, mod_param).as_bytes());
            } else {
                out.extend_from_slice(alloc::format!("\x1b[{}~", tilde_num).as_bytes());
            }
        } else {
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[1;{}{}", mod_param, final_char).as_bytes());
            } else {
                out.extend_from_slice(alloc::format!("\x1b[{}", final_char).as_bytes());
            }
        }
        return;
    }
    
    if let Some(f_idx) = match event.code {
        KeyCode::F1 => Some(1),
        KeyCode::F2 => Some(2),
        KeyCode::F3 => Some(3),
        KeyCode::F4 => Some(4),
        KeyCode::F5 => Some(5),
        KeyCode::F6 => Some(6),
        KeyCode::F7 => Some(7),
        KeyCode::F8 => Some(8),
        KeyCode::F9 => Some(9),
        KeyCode::F10 => Some(10),
        KeyCode::F11 => Some(11),
        KeyCode::F12 => Some(12),
        _ => None,
    } {
        if f_idx <= 4 {
            let ch = (b'P' + (f_idx - 1)) as char;
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[1;{}{}", mod_param, ch).as_bytes());
            } else {
                out.extend_from_slice(alloc::format!("\x1bO{}", ch).as_bytes());
            }
        } else {
            let num = match f_idx {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                _ => unreachable!(),
            };
            if mod_param > 1 {
                out.extend_from_slice(alloc::format!("\x1b[{};{}~", num, mod_param).as_bytes());
            } else {
                out.extend_from_slice(alloc::format!("\x1b[{}~", num).as_bytes());
            }
        }
        return;
    }
    
    if ctrl {
        if event.code >= KeyCode::A && event.code <= KeyCode::Z {
            let ctrl_char = (event.code as u8 - KeyCode::A as u8) + 1;
            if alt {
                out.push(0x1b);
            }
            out.push(ctrl_char);
            return;
        }
    
        match event.code {
            KeyCode::Space => {
                if alt {
                    out.push(0x1b);
                }
                out.push(0x00);
                return;
            }
            KeyCode::LeftBracket => { out.push(0x1b); return; }  // ctrl+[ = ESC
            KeyCode::Backslash => { out.push(0x1c); return; }    // ctrl+\ = SIGQUIT
            KeyCode::RightBracket => { out.push(0x1d); return; }
            _ => {}
        }
    
        if event.unicode != '\0' {
            out.extend_from_slice(alloc::format!("\x1b[{};{}u", event.unicode as u32, mod_param).as_bytes());
            return;
        }
    }
    
    if alt && event.unicode != '\0' {
        out.push(0x1b);
        let mut buf = [0u8; 4];
        out.extend_from_slice(event.unicode.encode_utf8(&mut buf).as_bytes());
        return;
    }
    
    if event.unicode != '\0' {
        let mut buf = [0u8; 4];
        out.extend_from_slice(event.unicode.encode_utf8(&mut buf).as_bytes());
    }
}
