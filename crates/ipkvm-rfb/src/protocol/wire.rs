pub(crate) fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    Some(u16::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

pub(crate) fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    let end = offset.checked_add(4)?;
    Some(i32::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

pub(crate) fn write_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

pub(crate) fn write_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_rfb_big_endian_integers() {
        let bytes = [0x12, 0x34, 0x89, 0xab, 0xcd, 0xef];
        assert_eq!(read_u16(&bytes, 0), Some(0x1234));
        assert_eq!(read_u32(&bytes, 2), Some(0x89ab_cdef));

        let mut output = Vec::new();
        write_u16(&mut output, 0x1234);
        write_u32(&mut output, 0x89ab_cdef);
        write_i32(&mut output, -223);
        assert_eq!(
            output,
            [0x12, 0x34, 0x89, 0xab, 0xcd, 0xef, 0xff, 0xff, 0xff, 0x21]
        );
    }

    #[test]
    fn short_reads_return_none() {
        assert_eq!(read_u16(&[1], 0), None);
        assert_eq!(read_u32(&[1, 2, 3], 0), None);
        assert_eq!(read_i32(&[1, 2, 3], 0), None);
    }
}
