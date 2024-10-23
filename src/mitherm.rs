use crate::{Error, Result};
use scroll::{ctx::TryFromCtx, Endian, Pread};
use serde::Serialize;

// pub const MANUFACTURER_ID: u16 = 0xa4c1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Announcement {
    pub temperature: f64,
    pub humidity: f64,
    pub battery_mv: u16,
	pub battery_percent: u8,
}

impl<'a> TryFromCtx<'a, ()> for Announcement { // this is the pvvx custom firmware format, apparently
    type Error = Error;
    fn try_from_ctx(from: &'a [u8], _: ()) -> Result<(Self, usize)> {
        let mut offset = 6;
        Ok((
            Self {
                temperature: from
                    .gread_with::<u16>(&mut offset, Endian::Little)
                    .map(|v| v as f64 * 0.01)?,
                humidity: from
                    .gread_with::<u16>(&mut offset, Endian::Little)
                    .map(|v| v as f64 * 0.01)?,
                battery_mv: from
                    .gread_with::<u16>(&mut offset, Endian::Little)?,
                battery_percent: from.gread(&mut offset)?
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
                temperature: 22.38,
                humidity: 54.44,
                battery_percent: 100,
                battery_mv: 3004,
            },
            [
				0x80,0x49,0xd8,0x38,0xc1,0xa4,0xbe,0x08,0x44,0x15,0xbc,0x0b,0x64,0xef,0x04
            ]
            .pread(0)
            .unwrap()
        );
    }
}

