mod aranet4;
mod mitherm;

use anyhow::{anyhow, Error, Result};
use bluez_async::{
    AdapterEvent, BluetoothEvent, BluetoothSession, DeviceEvent, DeviceId, DeviceInfo, uuid_from_u16
};
use chrono::{DateTime, Duration, Utc};
use clap::{Parser, ValueEnum};
use itertools::Itertools;
use scroll::Pread;
use serde::Serialize;
use std::{collections::HashMap, io::Write};
use tokio::select;
use tokio_stream::StreamExt;
use tracing::{debug, error, instrument, warn};

#[derive(Debug, Copy, Clone, ValueEnum)]
enum OutputFormat {
    Json,
    Waybar,
}

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

    /// Format to output in.
    #[arg(long, short = 'F', default_value = "json")]
    output_format: OutputFormat,
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
    tokio::spawn(async move { run(&args, session).await });

    // Bail out and hopefully restart if the session goes away for some reason.
    bt_join_handle.await?;
    Err(anyhow!("Bluetooth Session terminated!"))
}

#[derive(Debug, Default)]
struct State {
    aranet4: HashMap<DeviceId, (DateTime<Utc>, aranet4::Announcement)>,
    devices: HashMap<DeviceId, DeviceInfo>,
}

#[instrument(skip_all)]
async fn run(args: &Args, session: BluetoothSession) {
    let mut state = State::default();
    let mut output_ticker =
        tokio::time::interval(std::time::Duration::from_secs_f64(args.interval));
    let mut events = session.event_stream().await.unwrap();

    session.start_discovery().await.unwrap();
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
                if let Err(err) = print_state(args, &state) {
                    error!(?err, "Couldn't print state");
                }
            }
            Poll::Event(event) => {
                if let Err(err) = process_event(&mut state, &session, &event).await {
                    error!(?event, ?err, "Event Error");
                };
            }
        }
    }
}

fn print_state(args: &Args, state: &State) -> Result<()> {
    #[derive(Debug, Serialize)]
    struct Output<'s> {
        #[serde(skip_serializing_if = "HashMap::is_empty")]
        pub aranet4: HashMap<String, &'s aranet4::Announcement>,
    }

    // Accumulate output data.
    let now = Utc::now();
    let stale = Duration::from_std(std::time::Duration::from_secs_f64(args.stale)).unwrap();
    let output = Output {
        aranet4: state
            .aranet4
            .iter()
            .filter(|(_, (ts, _))| now - ts < stale)
            .map(|(id, (_, ann))| {
                (
                    state
                        .devices
                        .get(id)
                        .and_then(|info| info.name.clone())
                        .unwrap_or_else(|| id.to_string()),
                    ann,
                )
            })
            .collect(),
    };
    // Don't log anything if there's no non-stale data.
    if output.aranet4.is_empty() {
        return Ok(());
    }
    debug!("{:?}", &output);

    // Write to stdout.
    let mut stdout = std::io::stdout();
    match args.output_format {
        OutputFormat::Json => {
            serde_json::to_writer_pretty(&mut stdout, &output)?;
            writeln!(&mut stdout)?;
        }
        OutputFormat::Waybar => {
            // Format and sort the readings by CO2 value.
            let mut aranet4: Vec<(&String, u16, String)> = output
                .aranet4
                .iter()
                .map(|(id, ann)| {
                    let s = format!(
                        "🪟 {} 🌡️ {:.2} ☔ {} 🗜️ {:.0}",
                        ann.co2.map(i32::from).unwrap_or(-1),
                        ann.temperature.unwrap_or(-1.0),
                        ann.humidity,
                        ann.pressure.unwrap_or(-1.0),
                    );
                    (id, ann.co2.unwrap_or_default(), s)
                })
                .collect();
            aranet4.sort_by_key(|(_, co2, _)| -(*co2 as i32)); // hack to sort descending

            // Each line is one reading.
            #[derive(Serialize)]
            struct WaybarOutput<'a> {
                pub text: &'a str,
                pub tooltip: String,
            }
            serde_json::to_writer(
                &mut stdout,
                &WaybarOutput {
                    text: aranet4
                        .first()
                        .map(|(_, _, s)| s.as_str())
                        .unwrap_or_default(),
                    tooltip: aranet4
                        .iter()
                        .map(|(id, _, s)| format!("[{}] {}", id, s))
                        .join("\n"),
                },
            )?;
            writeln!(&mut stdout)?;
        }
    }
    stdout.flush()?;

    Ok(())
}

#[instrument(skip_all)]
async fn process_event(
    state: &mut State,
    session: &BluetoothSession,
    event: &BluetoothEvent,
) -> Result<()> {
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

                            if !state.devices.contains_key(id) {
                                debug!(dev = format!("{}", id), "Getting device info...");
                                match session.get_device_info(id).await {
                                    Ok(info) => {
                                        state.devices.insert(id.clone(), info);
                                    }
                                    Err(err) => warn!(
                                        dev = format!("{}", id),
                                        ?err,
                                        "Couldn't get device info"
                                    ),
                                }
                            }
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
					let uuid = uuid_from_u16(0x181A);
					debug!( svc = format!("{}", svc), uuid = format!("{}", uuid));
					debug!(
						dev = format!("{}", id),
						svc = format!("{}", svc),
						value = hex::encode(value),
						"⚙️ Service Data"
					);					
					if *svc == uuid {
						let ann = value.pread::<mitherm::Announcement>(0)?;
                        debug!(
                                dev = format!("{}", id),
                                temp = ann.temperature,
                                humid = ann.humidity,
                                bat = ann.battery_mv,
                                "🌬️ Mitherm announcement"
                        );
					}
                }
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}
