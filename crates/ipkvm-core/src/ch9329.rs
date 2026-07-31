use thiserror::Error;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum Ch9329Error {
    #[error("CH9329 frame data is too long: {0} bytes")]
    DataTooLong(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ch9329Frame {
    bytes: Vec<u8>,
}

impl Ch9329Frame {
    pub fn new(addr: u8, command: u8, data: &[u8]) -> Result<Self, Ch9329Error> {
        let len = u8::try_from(data.len()).map_err(|_| Ch9329Error::DataTooLong(data.len()))?;
        let mut bytes = Vec::with_capacity(6 + data.len());
        bytes.extend_from_slice(&[0x57, 0xAB, addr, command, len]);
        bytes.extend_from_slice(data);
        let checksum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        bytes.push(checksum);

        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
