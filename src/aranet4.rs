use crate::{Error, Result};
use scroll::{ctx::TryFromCtx, Endian, Pread};
use serde::Serialize;

pub const MANUFACTURER_ID: u16 = 0x0702;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Announcement {
    pub co2: Option<u16>,
    pub temperature: Option<f64>,
    pub pressure: Option<f64>,
    pub humidity: u8,
    pub battery: u8,
    pub status: u8,
}

impl<'a> TryFromCtx<'a, ()> for Announcement {
    type Error = Error;
    fn try_from_ctx(from: &'a [u8], _: ()) -> Result<(Self, usize)> {
        let mut offset = 8;
        Ok((
            Self {
                co2: from
                    .gread_with::<u16>(&mut offset, Endian::Little)
                    .map(|v| (v >> 15 != 1).then(|| v))?,
                temperature: from
                    .gread_with::<u16>(&mut offset, Endian::Little)
                    .map(|v| ((v >> 14 & 1) != 1).then(|| v as f64 * 0.05))?,
                pressure: from
                    .gread_with::<u16>(&mut offset, Endian::Little)
                    .map(|v| (v >> 15 != 1).then(|| v as f64 * 0.1))?,
                humidity: from.gread(&mut offset)?,
                battery: from.gread(&mut offset)?,
                status: from.gread(&mut offset)?,
            },
            offset,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_announcement() {
        assert_eq!(
            Announcement {
                co2: Some(697),
                temperature: Some(23.650000000000002),
                pressure: Some(1005.9000000000001),
                humidity: 51,
                battery: 20,
                status: 1,
            },
            [
                0x21, 0x13, 0x04, 0x01, 0x00, 0x0c, 0x0f, 0x01, 0xb9, 0x02, 0xd9, 0x01, 0x4b, 0x27,
                0x33, 0x14, 0x01, 0x3c, 0x00, 0x3c, 0x00, 0xc6,
            ]
            .pread(0)
            .unwrap()
        );
    }
}
