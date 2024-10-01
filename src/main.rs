mod aranet4;

use anyhow::{bail, Error, Result};
use bluez_async::{AdapterEvent, BluetoothEvent, BluetoothSession, DeviceEvent, DeviceId};
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use scroll::Pread;
use serde::Serialize;
use std::{cmp::max, collections::HashMap, io::Write};
use tokio::select;
use tokio_stream::{Stream, StreamExt};
use tracing::{debug, error, instrument};

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
    #[arg(long, short, default_value = "60")]
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

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_logging(&args);

    // Connect to BlueZ over DBus.
    let (bt_join_handle, session) = BluetoothSession::new().await?;

    // Spawn a background task that processes Bluetooth events.
    let events = session.event_stream().await?;
    tokio::spawn(async move { run(&args, events).await });

    // Start discovery.
    session.start_discovery().await?;

    // Bail out and hopefully restart if the session goes away, for some reason.
    bt_join_handle.await?;
    bail!("Bluetooth Session terminated!");
}

#[derive(Debug, Default)]
struct State {
    aranet4: HashMap<DeviceId, (DateTime<Utc>, aranet4::Announcement)>,
}

fn print_state(args: &Args, state: &State) -> Result<()> {
    #[derive(Debug, Default, Serialize)]
    struct OutputAranet4 {
        pub co2: Option<u16>,
        pub temperature: Option<f64>,
        pub pressure: Option<f64>,
        pub humidity: u8,
    }
    #[derive(Debug, Default, Serialize)]
    struct Output {
        #[serde(skip_serializing_if = "Option::is_none")]
        aranet4: Option<OutputAranet4>,
    }

    // Accumulate output data.
    let now = Utc::now();
    let stale = Duration::from_std(std::time::Duration::from_secs_f64(args.stale)).unwrap();
    let output = Output {
        aranet4: state
            .aranet4
            .iter()
            .filter(|(_, (ts, _))| now - ts < stale)
            .map(|(_, (_, ann))| ann)
            .fold(None, |out_, ann| {
                let mut out = out_.unwrap_or_default();
                // Use the highest currently observable readings.
                if let Some(co2) = ann.co2 {
                    out.co2 = Some(max(out.co2.unwrap_or_default(), co2));
                }
                if let Some(temp) = ann.temperature {
                    out.temperature = Some(f64::max(out.temperature.unwrap_or_default(), temp));
                }
                if let Some(press) = ann.pressure {
                    out.pressure = Some(f64::max(out.pressure.unwrap_or_default(), press));
                }
                out.humidity = max(out.humidity, ann.humidity);
                Some(out)
            }),
    };

    // Write to stdout.
    let mut stdout = std::io::stdout();
    serde_json::to_writer(&mut stdout, &output)?;
    write!(&mut stdout, "\n")?;
    stdout.flush()?;

    Ok(())
}

#[instrument(skip_all)]
async fn run(args: &Args, mut events: impl Stream<Item = BluetoothEvent> + Unpin) {
    let mut state = State::default();
    let mut output_ticker =
        tokio::time::interval(std::time::Duration::from_secs_f64(args.interval));
    loop {
        enum Poll {
            OutputTick,
            Event(BluetoothEvent),
        }
        match select! {
            _ = output_ticker.tick() => Poll::OutputTick,
            Some(event) = events.next() => Poll::Event(event),
        } {
            Poll::OutputTick => {
                if let Err(err) = print_state(&args, &state) {
                    error!(?err, "Couldn't print state");
                }
            }
            Poll::Event(event) => {
                if let Err(err) = process_event(&mut state, &event).await {
                    error!(?event, ?err, "Event Error");
                };
            }
        }
    }
}

#[instrument(skip_all)]
async fn process_event(state: &mut State, event: &BluetoothEvent) -> Result<()> {
    match event {
        BluetoothEvent::Adapter { id, event } => match event {
            AdapterEvent::Powered { powered } => {
                debug!(adp = format!("{}", id), powered, "🔌 Adapter (Un)Powered")
            }
            AdapterEvent::Discovering { discovering } => {
                debug!(
                    adp = format!("{}", id),
                    discovering, "🔍 Adapter Discovery Status"
                )
            }
            _ => {}
        },
        BluetoothEvent::Device { id, event } => match event {
            DeviceEvent::ManufacturerData { manufacturer_data } => {
                for (key, value) in manufacturer_data {
                    match *key {
                        aranet4::MANUFACTURER_ID => {
                            let ann = value.pread::<aranet4::Announcement>(0)?;
                            debug!(
                                dev = format!("{}", id),
                                co2 = ann.co2,
                                temp = ann.temperature,
                                press = ann.pressure,
                                humid = ann.humidity,
                                bat = ann.battery,
                                status = ann.status,
                                "🌬️ Aranet4 announcement"
                            );
                            state.aranet4.insert(id.clone(), (Utc::now(), ann));
                        }
                        0x004C => debug!(
                            dev = format!("{}", id),
                            value = hex::encode(value),
                            "🍏 Apple Data"
                        ),
                        _ => debug!(
                            dev = format!("{}", id),
                            key = format!("{:04X}", key),
                            value = hex::encode(value),
                            "🔢 Manufacturer Data"
                        ),
                    }
                }
            }
            DeviceEvent::ServiceData { service_data } => {
                for (svc, value) in service_data {
                    debug!(
                        dev = format!("{}", id),
                        svc = format!("{}", svc),
                        value = hex::encode(value),
                        "⚙️ Service Data"
                    );
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}
