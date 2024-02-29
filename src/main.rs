use anyhow::{anyhow, Error, Result};
use btleplug::api::{Central, CentralEvent, Manager};
use clap::Parser;
use scroll::{ctx::TryFromCtx, Endian, Pread};
use tokio_stream::StreamExt;
use tracing::{debug, instrument, trace};

#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Increase log level.
    #[arg(long, short, action=clap::ArgAction::Count)]
    verbose: u8,

    /// Decrease log level.
    #[arg(long, short, action=clap::ArgAction::Count)]
    quiet: u8,
}

#[instrument(skip_all)]
fn init_logging(args: &Args) {
    use tracing_subscriber::prelude::*;

    tracing_subscriber::Registry::default()
        .with(match args.verbose as i8 - args.quiet as i8 {
            ..=-2 => tracing_subscriber::filter::LevelFilter::ERROR,
            -1 => tracing_subscriber::filter::LevelFilter::WARN,
            0 => tracing_subscriber::filter::LevelFilter::INFO,
            1 => tracing_subscriber::filter::LevelFilter::DEBUG,
            2.. => tracing_subscriber::filter::LevelFilter::TRACE,
        })
        .with(tracing_subscriber::fmt::layer())
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(&args);

    let btman = btleplug::platform::Manager::new().await?;
    let adapter = btman
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No adapter found"))?;
    adapter
        .start_scan(btleplug::api::ScanFilter::default())
        .await?;

    let mut events = adapter.events().await?;
    while let Some(event) = events.next().await {
        match event {
            CentralEvent::ManufacturerDataAdvertisement {
                id,
                manufacturer_data,
            } => {
                trace!(
                    id = format!("{}", id),
                    ?manufacturer_data,
                    "🏭 Manufacturer Data"
                );
                if let Some(raw) = manufacturer_data.get(&0x0702) {
                    let ann: Aranet4Announcement = raw.pread(0)?;
                    debug!(
                        id = format!("{}", id),
                        raw = hex::encode(&raw),
                        co2 = ann.co2,
                        temperature = ann.temperature,
                        pressure = ann.pressure,
                        humidity = ann.humidity,
                        battery = ann.battery,
                        status = ann.status,
                        "📡 Aranet4 Announcement"
                    );
                }
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct Aranet4Announcement {
    pub co2: Option<u16>,
    pub temperature: Option<f64>,
    pub pressure: Option<f64>,
    pub humidity: u8,
    pub battery: u8,
    pub status: u8,
}

impl<'a> TryFromCtx<'a, ()> for Aranet4Announcement {
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
