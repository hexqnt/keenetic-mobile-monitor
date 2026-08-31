use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::config::PlotlySource;

pub(super) const PLOTLY_CDN: &str = "https://cdn.plot.ly/plotly-3.0.1.min.js";

const STYLE: &str = include_str!("dashboard.css");
const APP_JS: &str = include_str!("dashboard.js");

pub(super) fn render_page(
    source: PlotlySource,
    api_base_url: &str,
    iperf3_summary: Option<&str>,
    ping_host: Option<&str>,
    debug: bool,
) -> String {
    let plotly_url = if source == PlotlySource::Cdn {
        PLOTLY_CDN
    } else {
        "plotly.min.js"
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width,initial-scale=1";
                title { "Keenetic Mobile Monitor" }
                script src=(plotly_url) {}
                style { (PreEscaped(STYLE)) }
            }
            body data-api-base=(api_base_url) data-ping-host=[ping_host] {
                main {
                    header {
                        div {
                            h1 { "Keenetic Mobile Monitor" }
                            p { "Live signal quality in radio context" }
                        }
                        span id="connection" class="status pending" { "Connecting…" }
                    }

                    section class="cards summary" aria-label="Current radio context" {
                        div class="card" { span { "State / RAT" } strong id="network" { "—" } }
                        div class="card" { span { "Signal level" } strong id="signal-level" { "—" } }
                        div class="card" { span { "Operator" } strong id="operator" { "—" } }
                        div class="card" { span { "Site" } strong id="site" { "—" } }
                        div class="card" { span { "Sector / TAC" } strong id="sector" { "—" } }
                    }

                    (carrier_table())

                    section class="panel" aria-label="Signal history" {
                        div id="signal-chart" class="chart signal-chart" {}
                    }

                    section class="panel" aria-label="Traffic history" {
                        div id="traffic-chart" class="chart traffic-chart" {}
                    }

                    section class="panel" aria-label="Modem temperature history" {
                        div id="temperature-chart" class="chart temperature-chart" {}
                    }

                    @if let Some(summary) = iperf3_summary {
                        section class="panel iperf-panel" aria-label="Router traffic load test" {
                            div class="panel-heading" {
                                div {
                                    h2 { "Router traffic load" }
                                    p { "iPerf3 via Keenetic RCI · " (summary) }
                                }
                                div class="iperf-controls" {
                                    span id="iperf-status" class="status pending" aria-live="polite" { "Loading…" }
                                    button id="iperf-start" type="button" disabled { "Start iPerf3" }
                                }
                            }
                            pre id="iperf-output" class="iperf-output" aria-live="polite" { "The router output will appear here after the test." }
                        }
                    }

                    section class="panel context-panel" {
                        div class="panel-heading" {
                            div { h2 { "Radio context timeline" } p { "Synchronized with signal and traffic charts; gaps denote missing samples" } }
                        }
                        div id="context-chart" class="chart context-chart" {}
                    }

                    (analysis_tables())

                    @if debug {
                        details class="panel metadata" {
                            summary { "Latest raw mobile sample" }
                            pre id="metadata" { "Waiting for the first sample…" }
                        }
                    }
                }
                script { (PreEscaped(APP_JS)) }
            }
        }
    }
    .into_string()
}

fn carrier_table() -> Markup {
    html! {
        section class="panel table-panel" {
            div class="panel-heading" {
                div { h2 { "Current carriers" } p { "Primary and reported secondary component carriers" } }
                button id="copy-carriers" class="panel-action" type="button" disabled
                    title="Copy active carrier aggregation" { "Copy carriers" }
            }
            div class="table-scroll" {
                table id="carrier-table" {
                    thead { tr {
                        th { "Role" } th { "State" } th { "RAT" } th { "Band" }
                        th class="numeric" { "EARFCN" } th class="numeric" { "PCI" }
                        th class="numeric" title="Downlink / uplink frequency" { "Frequency DL / UL" }
                        th class="numeric" { "Bandwidth" } th class="numeric" { "DL layers" }
                        th title="Downlink / uplink modulation" { "Modulation DL / UL" }
                    } }
                    tbody { tr { td colspan="10" class="empty" { "Waiting for the first successful sample…" } } }
                }
            }
        }
    }
}

fn analysis_tables() -> Markup {
    html! {
        section class="panel table-panel" {
            div class="panel-heading" {
                div {
                    h2 { "Carrier radio statistics" }
                    p { "All traffic levels · share of successful observations for each PCell or active SCell; missing values are included as Unknown once a metric is reported" }
                }
            }
            div class="table-scroll tall" {
                table id="carrier-statistics-table" {
                    thead { tr {
                        th { "Carrier" } th { "Cell" } th { "Role" }
                        th class="numeric" { "Observed" } th class="numeric" { "Samples" }
                        th { "DL modulation" } th { "UL modulation" } th { "DL MIMO layers" }
                    } }
                    tbody { tr { td colspan="8" class="empty" { "No carrier radio measurements yet." } } }
                }
            }
        }

        section class="panel table-panel" {
            div class="panel-heading" {
                div {
                    h2 { "Context comparison" }
                    p { "Grouped by site, sector, primary carrier, and active aggregation set" }
                }
            }
            div class="table-scroll tall" {
                table id="context-table" {
                    thead { tr {
                        th { "Radio context" } th class="numeric" { "Observed" }
                        th class="numeric" { "Samples" } th class="numeric" { "Entries" }
                        th class="numeric" { "RSSI med / p10" br; "dBm" }
                        th class="numeric" { "RSRP med / p10" br; "dBm" }
                        th class="numeric" { "RSRQ med / p10" br; "dB" }
                        th class="numeric" { "SINR med / p10 · <5" br; "dB" }
                        th class="numeric" { "Max DL / UL" br; "Mbit/s" }
                    } }
                    tbody { tr { td colspan="9" class="empty" { "No successful samples yet." } } }
                }
            }
        }

        section class="panel table-panel" {
            div class="panel-heading" {
                div { h2 { "Radio events" } p { "Latest transitions, handovers, aggregation changes, and polling gaps" } }
            }
            div class="table-scroll events-scroll" {
                table id="event-table" {
                    thead { tr { th { "Time" } th { "Event" } th { "Details" } } }
                    tbody { tr { td colspan="3" class="empty" { "No events yet." } } }
                }
            }
        }
    }
}
