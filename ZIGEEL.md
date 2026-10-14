# Zigeel modifications to rustdesk-server

This repository is the **complete corresponding source** of the `hbbs` server that the Zigeel service
(https://zigeel.com) runs, published under the same licence as the original: **GNU AGPL-3.0** (see `LICENSE`).

- Upstream: https://github.com/rustdesk-org/rustdesk-server at commit `c3a3e0d1070e29cb28b468cf6ef5b184e65e4051`
  (the source of the `rustdesk/rustdesk-server:1.1.16` image). Branch `main` = that commit + the Zigeel changes below,
  so `git diff c3a3e0d` shows every modification.
- `hbbr` (relay) is **unmodified**.

## What was changed, and why

| File | Change |
|---|---|
| `src/rendezvous_server.rs` | **Account gate.** Every attempt to open a session — a `PunchHoleRequest`, or a `RequestRelay` hbbs would forward — is reported to the account service (`PAID_API`, HTTP `POST /internal/punch`, 3 s timeout), which decides whether it may proceed. Refused ⇒ the client gets the service's message. Service unreachable ⇒ refuse, unless `PAID_FAIL_OPEN=1`. Without `PAID_API` the server behaves like upstream. |
| `src/rendezvous_server.rs`, `src/lib.rs`, `Cargo.toml` | **Relay chosen by client location.** When `GEOIP_DB` points to a MaxMind-format city database, the relay offered to a pair of peers is the one with the shortest great-circle path A → relay → B (ties within `RELAY_GEO_TOL_KM`, default 500 km, are shared round-robin). Unlocatable peers fall back to upstream round-robin. Admin command `relay-geo` / `rgeo` prints the relay locations. Adds the `maxminddb` crate. |
| `src/relay_pick.rs` | Pure selection logic (haversine distance, tolerance grouping) with unit tests. |
| `src/relay_geo.rs` | GeoIP lookup (memory-mapped database, private/loopback addresses ignored, `RELAY_GEO` coordinate overrides, cached relay locations). |

The account service itself is a separate program that talks to hbbs over HTTP; it is not part of this work.
IP geolocation data used in production: DB-IP City Lite (https://db-ip.com, CC BY 4.0) — not included here.

## Build

Same toolchain as upstream CI (Rust 1.88):

```sh
git submodule update --init --recursive
cargo build --release --bin hbbs
```

`Cargo.lock` is the lock file of the production build.

## Runtime switches added

| Variable | Meaning |
|---|---|
| `PAID_API` | Base URL of the account service (e.g. `http://127.0.0.1:22114`). Unset = no account gate. |
| `PAID_FAIL_OPEN` | `1` = allow sessions when the account service is unreachable (default: refuse). |
| `GEOIP_DB` | Path to a MaxMind-format `.mmdb` city database. Unset = upstream round-robin relay choice. |
| `RELAY_GEO_TOL_KM` | Tie tolerance in km (default 500). |
| `RELAY_GEO` | Optional relay coordinate overrides, `host:port@lat,lon;host:port@lat,lon` (win over `GEOIP_DB`). |
