mod aranet4;

use anyhow::{anyhow, Error, Result};
use btleplug::api::{Central, CentralEvent, Manager};
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use scroll::Pread;
use serde::Serialize;
use std::io::Write;
use tokio::select;
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

    /// Interval (in seconds) to bundle output up into.
    #[arg(long, short, default_value = "2")]
    interval: f64,

    /// Duration (in seconds) after which data is considered stale.
    #[arg(long, short, default_value = "10")]
    stale: f64,
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

#[derive(Debug, Clone)]
pub struct Datapoint<T>(DateTime<Utc>, Option<T>);

impl<T> Default for Datapoint<T> {
    fn default() -> Self {
        Self(DateTime::default(), None)
    }
}

impl<T> Datapoint<T> {
    fn now(v: T) -> Self {
        Self(Utc::now(), Some(v))
    }

    fn as_ref_current(&self, stale_after: Duration) -> Option<&T> {
        if Utc::now() - self.0 <= stale_after {
            self.1.as_ref()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Datapoints {
    pub aranet4: Datapoint<aranet4::Announcement>,
}

impl Datapoints {
    fn current<'a>(&'a self, stale_after: Duration) -> Output<'a> {
        Output {
            aranet4: self.aranet4.as_ref_current(stale_after),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Output<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aranet4: Option<&'a aranet4::Announcement>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(&args);

    // Stream BTLE events.
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

    // Tick every -i/--interval and print the latest data.
    let mut output_ticker =
        tokio::time::interval(std::time::Duration::from_secs_f64(args.interval));

    // Datapoints kept.
    let stale_after = Duration::from_std(std::time::Duration::from_secs_f64(args.stale))?;
    let mut datapoints = Datapoints::default();
    loop {
        enum Poll {
            OutputTick,
            CentralEvent(CentralEvent),
        }
        match select! {
            _ = output_ticker.tick() => Poll::OutputTick,
            Some(event) = events.next() => Poll::CentralEvent(event),
        } {
            Poll::OutputTick => {
                let mut stdout = std::io::stdout();
                serde_json::to_writer(&mut stdout, &datapoints.current(stale_after))?;
                write!(&mut stdout, "\n")?;
                stdout.flush()?;
            }
            Poll::CentralEvent(event) => match event {
                CentralEvent::ManufacturerDataAdvertisement {
                    id,
                    manufacturer_data,
                } => {
                    for (key, raw) in manufacturer_data {
                        match key {
                            aranet4::MANUFACTURER_ID => {
                                let ann: aranet4::Announcement = raw.pread(0)?;
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
                                datapoints.aranet4 = Datapoint::now(ann);
                            }
                            _ => trace!(
                                id = format!("{}", id),
                                key = format!("{:x}", key),
                                "❔ Unknown Manufacturer Data"
                            ),
                        }
                    }
                }
                _ => {}
            },
        }
    }
}
