# Keenetic Mobile Monitor

[![CI](https://github.com/hexqnt/keenetic-mobile-monitor/actions/workflows/ci.yml/badge.svg)](https://github.com/hexqnt/keenetic-mobile-monitor/actions/workflows/ci.yml)

An in-memory Mobile/LTE monitor for Keenetic routers. It polls the local RCI API and shows signal quality, traffic rates, radio context, carrier aggregation, handovers, and failures in a live Plotly dashboard.

The project is not affiliated with or endorsed by Keenetic.

## Configuration

Copy [`config.example.toml`](config.example.toml) to `config.toml` and set the router URL and credentials. Keep the file private: it contains a plaintext password.

`poll_interval` accepts non-zero human-readable durations such as `500ms`, `5s`, or `1m`. `interface` is optional; when omitted, the first Mobile interface reported by the router is selected at startup.

The `[web]` section is used only in `serve` mode. The server defaults to <http://127.0.0.1:3000> and a pinned Plotly CDN build. Use `plotly_source = "local"` for offline use and `debug = true` to show the latest raw sample.

The optional `[iperf3]` section adds a dashboard button that starts an iPerf3 client on the router through `keenetic-rci`. All test parameters are fixed in TOML and cannot be overridden by the browser. The router needs the KeeneticOS iPerf3 component and access to the configured server.

The optional `[ping]` section adds average ICMP round-trip time to the traffic chart. Probes run through the selected Mobile interface; IP literals select the matching protocol, while hostnames default to IPv4 unless `ip_version = "ipv6"`. Overlapping runs are skipped, failures appear as gaps, and ping continues during iPerf3 tests.

## Usage

Collect samples until Ctrl+C or SIGTERM:

```bash
cargo run --release -- collect --config config.toml
```

Collect samples and serve the dashboard and SSE API:

```bash
cargo run --release -- serve --config config.toml
```

## Modem drivers

Model-specific carrier queries live under `src/modem/`. To support another modem, add a driver for detection, command construction, and response parsing, then register it in `src/modem.rs`.

## Operational notes

- The HTTP server has no authentication, and the iPerf3 endpoint can generate substantial traffic when enabled. Bind it only to a trusted interface or place it behind an authenticating reverse proxy.
- Samples have no retention limit or persistence: memory usage grows over time, and data is lost on exit.
- Signal measurements describe the whole mobile snapshot; Keenetic does not report separate RSSI, RSRP, RSRQ, or SINR values for each secondary carrier in this response.
- For FM350-GL, the monitor polls `AT+GTCAINFO?` for per-carrier downlink layers and modulation. Historical percentages represent successful observations, not traffic or scheduler resource shares.
- To host the dashboard separately, set `output_dir` and `api_base_url`. If the UI and API use different origins, also set `cors_origin` to the exact UI origin. The monitor will then expose `/api/events` and, when configured, `/api/iperf3`.
