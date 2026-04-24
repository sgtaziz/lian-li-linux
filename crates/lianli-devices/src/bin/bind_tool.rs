//! Interactive bind/unbind tool for wireless Lian Li devices.
//!
//! Opens the TX+RX dongle directly and drives `WirelessController::bind_device`
//! / `unbind_device`. The daemon MUST be stopped first — the USB dongle is
//! exclusive.

use anyhow::{Context, Result};
use lianli_devices::wireless::{DiscoveredDevice, WirelessController};
use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

const DISCOVERY_WINDOW: Duration = Duration::from_secs(5);

fn main() -> Result<()> {
    eprintln!("lianli bind tool — stop the daemon before running");
    eprintln!();

    let mut ctrl = WirelessController::new();
    ctrl.connect().context("connecting to dongle")?;
    ctrl.start_polling().context("starting RF discovery")?;

    eprintln!("Waiting {}s for device discovery…", DISCOVERY_WINDOW.as_secs());
    let deadline = Instant::now() + DISCOVERY_WINDOW;
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(250));
    }

    loop {
        let mut devices = ctrl.devices();
        devices.extend(ctrl.unbound_devices());
        if devices.is_empty() {
            eprintln!("No wireless devices discovered yet.");
        } else {
            print_device_list(&devices);
        }

        match prompt("\n[r]efresh list / [b]ind / [u]nbind / [q]uit: ")?.as_str() {
            "r" => continue,
            "q" | "" => break,
            "b" => prompt_and_run(&ctrl, &devices, Action::Bind)?,
            "u" => prompt_and_run(&ctrl, &devices, Action::Unbind)?,
            other => eprintln!("unknown command: {other}"),
        }

        // Re-read discovery so the state shown reflects the bind/unbind.
        thread::sleep(Duration::from_millis(800));
    }

    Ok(())
}

enum Action {
    Bind,
    Unbind,
}

fn prompt_and_run(
    ctrl: &WirelessController,
    devices: &[DiscoveredDevice],
    action: Action,
) -> Result<()> {
    if devices.is_empty() {
        eprintln!("No devices to act on.");
        return Ok(());
    }
    let idx_str = prompt(&format!(
        "Pick device [1-{}] (or blank to cancel): ",
        devices.len()
    ))?;
    let Ok(idx) = idx_str.parse::<usize>() else {
        eprintln!("cancelled.");
        return Ok(());
    };
    let Some(dev) = idx.checked_sub(1).and_then(|i| devices.get(i)) else {
        eprintln!("index out of range.");
        return Ok(());
    };
    match action {
        Action::Bind => {
            eprintln!("binding {} …", dev.mac_str());
            ctrl.bind_device(&dev.mac)?;
        }
        Action::Unbind => {
            eprintln!("unbinding {} …", dev.mac_str());
            ctrl.unbind_device(&dev.mac)?;
        }
    }
    eprintln!("done.");
    Ok(())
}

fn print_device_list(devices: &[DiscoveredDevice]) {
    println!();
    println!(
        "{:>3}  {:17}  {:17}  {:>3}  {:>3}  {:>3}  {}",
        "#", "MAC", "MASTER", "ch", "rx", "fans", "type"
    );
    println!("{}", "-".repeat(78));
    for (i, d) in devices.iter().enumerate() {
        let master = format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            d.master_mac[0], d.master_mac[1], d.master_mac[2],
            d.master_mac[3], d.master_mac[4], d.master_mac[5],
        );
        let bound = d.master_mac != [0u8; 6];
        println!(
            "{:>3}  {:17}  {:17}  {:>3}  {:>3}  {:>3}  {}{}",
            i + 1,
            d.mac_str(),
            if bound { master } else { "(unbound)".to_string() },
            d.channel,
            d.rx_type,
            d.fan_count,
            d.fan_type.display_name(),
            if bound { "" } else { "  [UNBOUND]" },
        );
    }
}

fn prompt(msg: &str) -> Result<String> {
    eprint!("{msg}");
    io::stderr().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}
