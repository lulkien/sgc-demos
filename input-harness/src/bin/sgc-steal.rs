//! Take one @sgc resource for N ms, then let go — the revoke/re-grant driver.
//!
//! FairQueue (the daemon's default policy) makes a newcomer PREEMPT the current
//! owner, so acquiring a resource the app under test holds revokes it from that
//! app; dropping the connection releases it and the daemon re-grants the queued
//! owner (the app). Point it at an Input to drive input revoke/re-grant cycles
//! while the app keeps its DRM lease and keeps rendering — that is the cycle the
//! input resume path has to survive.
//!
//! Usage: sgc-steal <resource> [hold_ms] [cycles] [gap_ms]
//!   resource: mouse:N | keyboard:N | touch:N | drm:N
//!
//! Find N and the device behind it in the app's log: `acquiring
//! Input(Keyboard(0))` / `libinput device added: Input(Keyboard(0)) at
//! /dev/input/eventN (<name>)`, and in the daemon's `Opened /dev/input/eventN
//! (<name>)`. Match virtual devices by the NAME uinput-inject was given.

use std::os::fd::AsRawFd;
use std::time::Duration;

use libsgc_rs::{InputResource, Resource, SgcClient};

fn parse(spec: &str) -> Option<Resource> {
    let (kind, index) = spec.split_once(':')?;
    let index: u8 = index.parse().ok()?;
    Some(match kind {
        "mouse" => Resource::Input(InputResource::Mouse(index)),
        "keyboard" => Resource::Input(InputResource::Keyboard(index)),
        "touch" => Resource::Input(InputResource::Touch(index)),
        "drm" => Resource::Drm { card: index },
        _ => return None,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let spec = args.next().unwrap_or_else(|| "mouse:0".into());
    let hold_ms: u64 = args.next().unwrap_or_else(|| "2000".into()).parse()?;
    let cycles: u32 = args.next().unwrap_or_else(|| "1".into()).parse()?;
    let gap_ms: u64 = args.next().unwrap_or_else(|| "1000".into()).parse()?;

    let resource = parse(&spec).ok_or("resource must be mouse:N|keyboard:N|touch:N|drm:N")?;

    for cycle in 0..cycles {
        let (mut client, available) = SgcClient::connect()?;
        if !available.contains(&resource) {
            eprintln!("[steal] {resource:?} is not advertised; the daemon offers {available:?}");
            return Ok(());
        }
        client.acquire(resource.clone())?;
        let fd = client.fd(&resource)?;
        println!(
            "[steal] cycle {cycle}: acquired {resource:?} (fd {}) — holding {hold_ms}ms",
            fd.as_raw_fd()
        );
        std::thread::sleep(Duration::from_millis(hold_ms));
        // Dropping the client closes the socket: the daemon reclaims the
        // resource and re-grants it to the queued owner (the app under test).
        drop(client);
        println!("[steal] cycle {cycle}: released {resource:?}");
        if cycle + 1 < cycles {
            std::thread::sleep(Duration::from_millis(gap_ms));
        }
    }
    Ok(())
}
