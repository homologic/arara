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
