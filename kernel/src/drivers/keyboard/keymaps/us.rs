use vespertine_abi::key::{KeyCode, Modifiers};

use crate::drivers::keyboard::scancodes::Keymap;

pub struct Keymap_US;

impl Keymap for Keymap_US {
    fn name(&self) -> &'static str {
        "US"
    }

    fn map_key(&self, code: KeyCode, mods: Modifiers) -> char {
        let shift = mods.contains(Modifiers::SHIFT);
        let capsl = mods.contains(Modifiers::CAPSL);
        let numl  = mods.contains(Modifiers::NUML);

        let uppercase = shift ^ capsl;

        match code {
            KeyCode::A => if uppercase { 'A' } else { 'a' },
            KeyCode::B => if uppercase { 'B' } else { 'b' },
            KeyCode::C => if uppercase { 'C' } else { 'c' },
            KeyCode::D => if uppercase { 'D' } else { 'd' },
            KeyCode::E => if uppercase { 'E' } else { 'e' },
            KeyCode::F => if uppercase { 'F' } else { 'f' },
            KeyCode::G => if uppercase { 'G' } else { 'g' },
            KeyCode::H => if uppercase { 'H' } else { 'h' },
            KeyCode::I => if uppercase { 'I' } else { 'i' },
            KeyCode::J => if uppercase { 'J' } else { 'j' },
            KeyCode::K => if uppercase { 'K' } else { 'k' },
            KeyCode::L => if uppercase { 'L' } else { 'l' },
            KeyCode::M => if uppercase { 'M' } else { 'm' },
            KeyCode::N => if uppercase { 'N' } else { 'n' },
            KeyCode::O => if uppercase { 'O' } else { 'o' },
            KeyCode::P => if uppercase { 'P' } else { 'p' },
            KeyCode::Q => if uppercase { 'Q' } else { 'q' },
            KeyCode::R => if uppercase { 'R' } else { 'r' },
            KeyCode::S => if uppercase { 'S' } else { 's' },
            KeyCode::T => if uppercase { 'T' } else { 't' },
            KeyCode::U => if uppercase { 'U' } else { 'u' },
            KeyCode::V => if uppercase { 'V' } else { 'v' },
            KeyCode::W => if uppercase { 'W' } else { 'w' },
            KeyCode::X => if uppercase { 'X' } else { 'x' },
            KeyCode::Y => if uppercase { 'Y' } else { 'y' },
            KeyCode::Z => if uppercase { 'Z' } else { 'z' },

            // NUMBERS
            KeyCode::Num1 => if shift { '!' } else { '1' },
            KeyCode::Num2 => if shift { '@' } else { '2' },
            KeyCode::Num3 => if shift { '#' } else { '3' },
            KeyCode::Num4 => if shift { '$' } else { '4' },
            KeyCode::Num5 => if shift { '%' } else { '5' },
            KeyCode::Num6 => if shift { '^' } else { '6' },
            KeyCode::Num7 => if shift { '&' } else { '7' },
            KeyCode::Num8 => if shift { '*' } else { '8' },
            KeyCode::Num9 => if shift { '(' } else { '9' },
            KeyCode::Num0 => if shift { ')' } else { '0' },

            // SYMBOLS
            KeyCode::Grave        => if shift { '~' } else { '`' },
            KeyCode::Minus        => if shift { '_' } else { '-' },
            KeyCode::Equal        => if shift { '+' } else { '=' },
            KeyCode::LeftBracket  => if shift { '{' } else { '[' },
            KeyCode::RightBracket => if shift { '}' } else { ']' },
            KeyCode::Backslash    => if shift { '|' } else { '\\' },
            KeyCode::Semicolon     => if shift { ':' } else { ';' },
            KeyCode::Quote        => if shift { '"' } else { '\'' },
            KeyCode::Comma        => if shift { '<' } else { ',' },
            KeyCode::Period       => if shift { '>' } else { '.' },
            KeyCode::Slash        => if shift { '?' } else { '/' },
            KeyCode::IntlBackslash => if shift { '|' } else { '\\' },

            // CONTROL
            KeyCode::Space     => ' ',
            KeyCode::Tab       => '\t',
            KeyCode::Enter     => '\r',
            KeyCode::Backspace => '\x08',

            // NUMPAD
            KeyCode::Kp0 => if numl { '0' } else { '\0' },
            KeyCode::Kp1 => if numl { '1' } else { '\0' },
            KeyCode::Kp2 => if numl { '2' } else { '\0' },
            KeyCode::Kp3 => if numl { '3' } else { '\0' },
            KeyCode::Kp4 => if numl { '4' } else { '\0' },
            KeyCode::Kp5 => if numl { '5' } else { '\0' },
            KeyCode::Kp6 => if numl { '6' } else { '\0' },
            KeyCode::Kp7 => if numl { '7' } else { '\0' },
            KeyCode::Kp8 => if numl { '8' } else { '\0' },
            KeyCode::Kp9 => if numl { '9' } else { '\0' },
            KeyCode::KpDecimal  => if numl { '.' } else { '\0' },
            KeyCode::KpDivide   => '/',
            KeyCode::KpMultiply => '*',
            KeyCode::KpSubtract => '-',
            KeyCode::KpAdd      => '+',
            KeyCode::KpEnter    => '\r',
            KeyCode::KpEqual    => '=',

            _ => '\0',
        }
    }
}
