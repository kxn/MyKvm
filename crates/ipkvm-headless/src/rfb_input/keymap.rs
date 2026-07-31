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
        0xffe1 => return Ok(MappedKey::Direct(usage(0xe1))),
        0xffe2 => return Ok(MappedKey::Direct(usage(0xe5))),
        0xff7f | 0xffe5 => return Ok(MappedKey::IgnoredLock),
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
    Ok(MappedKey::Character {
        usage: usage(usage_value),
        shift,
    })
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
}
