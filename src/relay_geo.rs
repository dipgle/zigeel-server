//! Relay choice by client location (Zigeel). Glue between hbbs and relay_pick.rs:
//!   client IP -> (lat, lon) from a MaxMind-format city database (GEOIP_DB, e.g. DB-IP "IP to City Lite", CC BY 4.0);
//!   relay "host:port" -> (lat, lon) from RELAY_GEO overrides, else the same database on the relay's IP.
//! No database, no located end, or no located relay => None, and hbbs keeps upstream round-robin.
//!
//! Env (read once at start):
//!   GEOIP_DB=/root/geoip/dbip-city-lite.mmdb
//!   RELAY_GEO="46.250.231.130:22117@1.29,103.85;207.180.221.2:22117@49.45,11.08"   optional, wins over GEOIP_DB
//!   RELAY_GEO_TOL_KM=500   relays whose path cost is within this of the best share the load round-robin
//! Admin: `relay-geo` / `rgeo` on the loopback command port prints the state below; `tg <ip> <ip>` shows a pick.

use crate::relay_pick::{best_relays, LatLon};
use hbb_common::log;
use maxminddb::{geoip2, Mmap, Reader};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

static DB: Lazy<Option<Reader<Mmap>>> = Lazy::new(|| {
    let path = std::env::var("GEOIP_DB").unwrap_or_default();
    if path.is_empty() {
        log::warn!("GEOIP_DB not set: relays are round-robined, not chosen by client location");
        return None;
    }
    match Reader::open_mmap(&path) {
        Ok(r) => {
            log::info!("GEOIP_DB={} loaded: {} built {}", path, r.metadata.database_type, r.metadata.build_epoch);
            Some(r)
        }
        Err(e) => {
            log::error!("GEOIP_DB={} unusable ({}): relays are round-robined", path, e);
            None
        }
    }
});

static OVERRIDES: Lazy<HashMap<String, LatLon>> =
    Lazy::new(|| parse_overrides(&std::env::var("RELAY_GEO").unwrap_or_default()));

static TOL_KM: Lazy<f64> = Lazy::new(|| {
    std::env::var("RELAY_GEO_TOL_KM").ok().and_then(|v| v.parse().ok()).unwrap_or(500.0)
});

fn parse_overrides(s: &str) -> HashMap<String, LatLon> {
    s.split(';')
        .filter_map(|e| {
            let (relay, ll) = e.trim().split_once('@')?;
            let (lat, lon) = ll.split_once(',')?;
            Some((relay.trim().to_owned(), (lat.trim().parse().ok()?, lon.trim().parse().ok()?)))
        })
        .collect()
}

/// The address to look up, or None for addresses no database can place (private, loopback, CGNAT, ...).
fn public(ip: IpAddr) -> Option<IpAddr> {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    };
    let local = match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 carrier-grade NAT
        }
        IpAddr::V6(v6) => {
            let s0 = v6.segments()[0];
            v6.is_loopback() || v6.is_unspecified() || (s0 & 0xfe00) == 0xfc00 || (s0 & 0xffc0) == 0xfe80
        }
    };
    if local {
        None
    } else {
        Some(ip)
    }
}

/// Location of a client address; None when private, absent from the database, or no database.
pub fn locate(ip: IpAddr) -> Option<LatLon> {
    let ip = public(ip)?;
    let city: geoip2::City = DB.as_ref()?.lookup(ip).ok()?;
    let loc = city.location?;
    Some((loc.latitude?, loc.longitude?))
}

/// Relay locations never change while hbbs runs (the database and RELAY_GEO are read once), so each relay
/// string is looked up once instead of on every punch.
static RELAY_LOC: Lazy<Mutex<HashMap<String, (Option<LatLon>, &'static str)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Location of a relay ("host", "host:port", "[v6]:port" or bare "v6") and where it came from.
pub fn relay_location(relay: &str) -> (Option<LatLon>, &'static str) {
    if let Some(hit) = RELAY_LOC.lock().ok().and_then(|m| m.get(relay).copied()) {
        return hit;
    }
    let found = if let Some(ll) = OVERRIDES.get(relay) {
        (Some(*ll), "RELAY_GEO")
    } else {
        let host = if relay.parse::<IpAddr>().is_ok() {
            relay // bare address, including an unbracketed IPv6 with no port
        } else {
            relay
                .rsplit_once(':')
                .map(|(h, _)| h)
                .unwrap_or(relay)
                .trim_matches(|c| c == '[' || c == ']')
        };
        match host.parse::<IpAddr>().ok().and_then(locate) {
            Some(ll) => (Some(ll), "GEOIP_DB"),
            None => (None, "unknown"),
        }
    };
    if let Ok(mut m) = RELAY_LOC.lock() {
        m.insert(relay.to_owned(), found);
    }
    found
}

/// Indices into `relays` to round-robin over for a session between `pa` and `pb`; None = use them all.
pub fn choose(relays: &[String], pa: IpAddr, pb: IpAddr) -> Option<Vec<usize>> {
    let located: Vec<Option<LatLon>> = relays.iter().map(|r| relay_location(r).0).collect();
    best_relays(&located, locate(pa), locate(pb), *TOL_KM)
}

/// State for the log and the `relay-geo` admin command: database, tolerance, one line per relay.
pub fn describe(relays: &[String]) -> String {
    let db = match DB.as_ref() {
        Some(r) => format!("{} built {}", r.metadata.database_type, r.metadata.build_epoch),
        None => "NOT LOADED (round-robin over every relay)".to_owned(),
    };
    let mut out = format!("geoip: {}\ntol_km: {}\n", db, *TOL_KM);
    for r in relays {
        match relay_location(r) {
            (Some((lat, lon)), src) => out += &format!("{} {:.2},{:.2} ({})\n", r, lat, lon, src),
            (None, src) => out += &format!("{} - ({}: never chosen while another relay is located)\n", r, src),
        }
    }
    out
}
