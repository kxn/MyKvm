use ipkvm_core::KeyboardUsage;

use super::RfbKeyboardError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShiftRequirement {
    Required,
    NotRequired,
}

impl ShiftRequirement {
    #[cfg(test)]
    fn from_required(required: bool) -> Self {
        if required {
            Self::Required
        } else {
            Self::NotRequired
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MappedKey {
    Direct(KeyboardUsage),
    Character {
        usage: KeyboardUsage,
        shift: ShiftRequirement,
    },
    IgnoredLock,
}

const UNSHIFTED_PUNCTUATION: &[(char, u8)] = &[
    (' ', 0x2c),
    ('-', 0x2d),
    ('=', 0x2e),
    ('[', 0x2f),
    (']', 0x30),
    ('\\', 0x31),
    (';', 0x33),
    ('\'', 0x34),
    ('`', 0x35),
    (',', 0x36),
    ('.', 0x37),
    ('/', 0x38),
];

const SHIFTED_PUNCTUATION: &[(char, u8)] = &[
    ('!', 0x1e),
    ('@', 0x1f),
    ('#', 0x20),
    ('$', 0x21),
    ('%', 0x22),
    ('^', 0x23),
    ('&', 0x24),
    ('*', 0x25),
    ('(', 0x26),
    (')', 0x27),
    ('_', 0x2d),
    ('+', 0x2e),
    ('{', 0x2f),
    ('}', 0x30),
    ('|', 0x31),
    (':', 0x33),
    ('"', 0x34),
    ('~', 0x35),
    ('<', 0x36),
    ('>', 0x37),
    ('?', 0x38),
];

pub(super) fn map_keysym(keysym: u32) -> Result<MappedKey, RfbKeyboardError> {
    match keysym {
        0xfe20 => return Ok(character_key(0x2b, ShiftRequirement::Required)),
        0xff08 => return Ok(direct(0x2a)),
        0xff09 => return Ok(direct(0x2b)),
        0xff0d => return Ok(direct(0x28)),
        0xff13 => return Ok(direct(0x48)),
        0xff14 => return Ok(direct(0x47)),
        0xff15 | 0xff61 => return Ok(direct(0x46)),
        0xff1b => return Ok(direct(0x29)),
        0xff50 => return Ok(direct(0x4a)),
        0xff51 => return Ok(direct(0x50)),
        0xff52 => return Ok(direct(0x52)),
        0xff53 => return Ok(direct(0x4f)),
        0xff54 => return Ok(direct(0x51)),
        0xff55 => return Ok(direct(0x4b)),
        0xff56 => return Ok(direct(0x4e)),
        0xff57 => return Ok(direct(0x4d)),
        0xff63 => return Ok(direct(0x49)),
        0xff67 => return Ok(direct(0x65)),
        0xff80 => return Ok(direct(0x2c)),
        0xff89 => return Ok(direct(0x2b)),
        0xff8d => return Ok(direct(0x58)),
        0xff91..=0xff94 => return Ok(direct(0x3a + (keysym - 0xff91) as u8)),
        0xff95 => return Ok(direct(0x5f)),
        0xff96 => return Ok(direct(0x5c)),
        0xff97 => return Ok(direct(0x60)),
        0xff98 => return Ok(direct(0x5e)),
        0xff99 => return Ok(direct(0x5a)),
        0xff9a => return Ok(direct(0x61)),
        0xff9b => return Ok(direct(0x5b)),
        0xff9c => return Ok(direct(0x59)),
        0xff9d => return Ok(direct(0x5d)),
        0xff9e => return Ok(direct(0x62)),
        0xff9f => return Ok(direct(0x63)),
        0xffaa => return Ok(direct(0x55)),
        0xffab => return Ok(direct(0x57)),
        0xffac => return Ok(direct(0x85)),
        0xffad => return Ok(direct(0x56)),
        0xffae => return Ok(direct(0x63)),
        0xffaf => return Ok(direct(0x54)),
        0xffb0 => return Ok(direct(0x62)),
        0xffb1..=0xffb9 => return Ok(direct(0x58 + (keysym - 0xffb0) as u8)),
        0xffbd => return Ok(direct(0x67)),
        0xffbe..=0xffc9 => return Ok(direct(0x3a + (keysym - 0xffbe) as u8)),
        0xffca..=0xffd1 => return Ok(direct(0x68 + (keysym - 0xffca) as u8)),
        0xffe1 => return Ok(direct(0xe1)),
        0xffe2 => return Ok(direct(0xe5)),
        0xffe3 => return Ok(direct(0xe0)),
        0xffe4 => return Ok(direct(0xe4)),
        0xffe7 | 0xffeb => return Ok(direct(0xe3)),
        0xffe8 | 0xffec => return Ok(direct(0xe7)),
        0xffe9 => return Ok(direct(0xe2)),
        0xffea => return Ok(direct(0xe6)),
        0xff7f | 0xffe5 => return Ok(MappedKey::IgnoredLock),
        0xffff => return Ok(direct(0x4c)),
        _ => {}
    }

    let Some(character) = char::from_u32(keysym) else {
        return Err(RfbKeyboardError::UnsupportedKeysym(keysym));
    };
    if !('\u{20}'..='\u{7e}').contains(&character) {
        return Err(RfbKeyboardError::UnsupportedKeysym(keysym));
    }

    let (usage_value, shift) =
        map_ascii(character).ok_or(RfbKeyboardError::UnsupportedKeysym(keysym))?;
    Ok(character_key(usage_value, shift))
}

fn direct(value: u8) -> MappedKey {
    MappedKey::Direct(usage(value))
}

fn character_key(value: u8, shift: ShiftRequirement) -> MappedKey {
    MappedKey::Character {
        usage: usage(value),
        shift,
    }
}

fn usage(value: u8) -> KeyboardUsage {
    KeyboardUsage::new(value).expect("key map contains valid HID usages")
}

fn map_ascii(character: char) -> Option<(u8, ShiftRequirement)> {
    if character.is_ascii_lowercase() {
        return Some((
            0x04 + (character as u8 - b'a'),
            ShiftRequirement::NotRequired,
        ));
    }
    if character.is_ascii_uppercase() {
        return Some((0x04 + (character as u8 - b'A'), ShiftRequirement::Required));
    }
    if let Some(position) = b"1234567890"
        .iter()
        .position(|digit| *digit == character as u8)
    {
        return Some((0x1e + position as u8, ShiftRequirement::NotRequired));
    }
    if let Some((_, usage)) = UNSHIFTED_PUNCTUATION
        .iter()
        .find(|(candidate, _)| *candidate == character)
    {
        return Some((*usage, ShiftRequirement::NotRequired));
    }
    SHIFTED_PUNCTUATION
        .iter()
        .find(|(candidate, _)| *candidate == character)
        .map(|(_, usage)| (*usage, ShiftRequirement::Required))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_representative_printable_ascii_keys_for_en_us() {
        let cases = [
            ('a', 0x04, false),
            ('z', 0x1d, false),
            ('A', 0x04, true),
            ('Z', 0x1d, true),
            ('1', 0x1e, false),
            ('0', 0x27, false),
            ('!', 0x1e, true),
            ('@', 0x1f, true),
            ('#', 0x20, true),
            ('-', 0x2d, false),
            ('_', 0x2d, true),
            ('=', 0x2e, false),
            ('+', 0x2e, true),
            ('[', 0x2f, false),
            ('{', 0x2f, true),
            (']', 0x30, false),
            ('}', 0x30, true),
            ('\\', 0x31, false),
            ('|', 0x31, true),
            (';', 0x33, false),
            (':', 0x33, true),
            ('\'', 0x34, false),
            ('"', 0x34, true),
            ('`', 0x35, false),
            ('~', 0x35, true),
            (',', 0x36, false),
            ('<', 0x36, true),
            ('.', 0x37, false),
            ('>', 0x37, true),
            ('/', 0x38, false),
            ('?', 0x38, true),
            (' ', 0x2c, false),
        ];

        for (character, usage, shift) in cases {
            assert_eq!(
                map_keysym(character as u32).unwrap(),
                MappedKey::Character {
                    usage: KeyboardUsage::new(usage).unwrap(),
                    shift: ShiftRequirement::from_required(shift),
                }
            );
        }
    }

    #[test]
    fn rejects_non_ascii_character_keysyms() {
        assert_eq!(
            map_keysym(0x00e9),
            Err(RfbKeyboardError::UnsupportedKeysym(0x00e9))
        );
        assert_eq!(
            map_keysym(0x0101_f642),
            Err(RfbKeyboardError::UnsupportedKeysym(0x0101_f642))
        );
    }

    #[test]
    fn every_printable_ascii_value_is_supported() {
        for keysym in 0x20..=0x7e {
            assert!(
                matches!(map_keysym(keysym), Ok(MappedKey::Character { .. })),
                "missing keysym {keysym:#x}"
            );
        }
    }

    #[test]
    fn maps_extended_function_keys_to_usb_hid_usages() {
        for keysym in 0xffca..=0xffd1 {
            assert_eq!(
                map_keysym(keysym).unwrap(),
                MappedKey::Direct(KeyboardUsage::new(0x68 + (keysym - 0xffca) as u8).unwrap())
            );
        }
    }
}
