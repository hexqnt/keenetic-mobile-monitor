use std::{
    fmt::Display,
    num::{NonZeroU8, NonZeroU16},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use keenetic_rci::{
    ComponentCarrier, ComponentCarrierId, Error as KeeneticError, Iccid, Imei, Imsi, InterfaceStat,
    MobileConnectionState, MobileStatus, Plmn, RadioAccessTechnology, RadioBand, Reported,
    SimState,
};
use serde::{Serialize, Serializer};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollOutcome {
    Success { mobile: Box<MobileSnapshot> },
    Failure { error: PollFailure },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Request,
    Transport,
    Authentication,
    Http,
    Rci,
    InvalidResponse,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PingOutcome {
    Replies {
        transmitted: NonZeroU16,
        received: NonZeroU16,
        average_rtt_ms: f64,
    },
    NoReply {
        transmitted: NonZeroU16,
    },
    Failure {
        error: PollFailure,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct LteSample {
    id: u64,
    collected_at_ms: u64,
    poll_duration_ms: u64,
    router_id: Arc<str>,
    interface: Arc<str>,
    traffic: Option<TrafficSnapshot>,
    ping: Option<Arc<PingSnapshot>>,
    #[serde(flatten)]
    outcome: PollOutcome,
}

impl LteSample {
    pub const fn id(&self) -> u64 {
        self.id
    }
}

pub struct PendingSample {
    pub collected_at_ms: u64,
    pub poll_duration_ms: u64,
    pub router_id: Arc<str>,
    pub interface: Arc<str>,
    pub traffic: Option<TrafficSnapshot>,
    pub ping: Option<Arc<PingSnapshot>>,
    pub outcome: PollOutcome,
}

impl PendingSample {
    pub fn sequence(self, id: u64) -> LteSample {
        LteSample {
            id,
            collected_at_ms: self.collected_at_ms,
            poll_duration_ms: self.poll_duration_ms,
            router_id: self.router_id,
            interface: self.interface,
            traffic: self.traffic,
            ping: self.ping,
            outcome: self.outcome,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PingSnapshot {
    collected_at_ms: u64,
    probe_duration_ms: u64,
    host: Arc<str>,
    #[serde(flatten)]
    outcome: PingOutcome,
}

impl PingSnapshot {
    pub const fn new(
        collected_at_ms: u64,
        probe_duration_ms: u64,
        host: Arc<str>,
        outcome: PingOutcome,
    ) -> Self {
        Self {
            collected_at_ms,
            probe_duration_ms,
            host,
            outcome,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PollFailure {
    pub kind: FailureKind,
    pub message: String,
}

impl PollFailure {
    pub fn from_error(error: &KeeneticError) -> Self {
        let kind = match error {
            KeeneticError::QuerySerialization(_) | KeeneticError::JsonSerialization(_) => {
                FailureKind::Request
            }
            KeeneticError::Transport(_) => FailureKind::Transport,
            KeeneticError::Authentication(_) => FailureKind::Authentication,
            KeeneticError::Http(_) => FailureKind::Http,
            KeeneticError::Rci(_) => FailureKind::Rci,
            KeeneticError::ResponseJson(_) | KeeneticError::ResponseDeserialization(_) => {
                FailureKind::InvalidResponse
            }
            _ => FailureKind::InvalidResponse,
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }

    pub fn invalid_response(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::InvalidResponse,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MobileSnapshot {
    pub plugged: Option<Box<str>>,
    #[serde(serialize_with = "serialize_optional_display")]
    pub connection_state: Option<MobileConnectionState>,
    pub operator: Option<Box<str>>,
    pub apn: Option<Box<str>>,
    #[serde(serialize_with = "serialize_optional_display")]
    pub sim_state: Option<SimState>,
    pub pin_attempts: Option<u8>,
    pub embedded: Option<bool>,
    pub roaming: Option<bool>,
    pub imei: Option<Reported<Imei>>,
    pub imsi: Option<Reported<Imsi>>,
    pub iccid: Option<Reported<Iccid>>,
    pub phone_number: Option<Box<str>>,
    pub signal: SignalSnapshot,
    pub temperature_c: Option<f32>,
    pub cell: CellSnapshot,
    pub primary_carrier: CarrierSnapshot,
    pub secondary_carriers: Vec<CarrierSnapshot>,
    pub carrier_radio_statistics_support: CarrierRadioStatisticsSupport,
    pub modem: Option<ModemSnapshot>,
    pub sim_service_provider: Option<Box<str>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierRadioStatisticsSupport {
    Supported,
    Unsupported,
}

impl From<MobileStatus> for MobileSnapshot {
    fn from(status: MobileStatus) -> Self {
        let mut primary_carrier = CarrierSnapshot::from(status.primary_carrier);
        let mut reported_carriers = status.reported_carriers;
        if let Some(reported_primary) = reported_carriers.remove(&ComponentCarrierId::PRIMARY) {
            primary_carrier.fill_missing_from(CarrierSnapshot::from(reported_primary));
        }
        let secondary_carriers = reported_carriers
            .into_values()
            .map(CarrierSnapshot::from)
            .collect();
        Self {
            plugged: status.plugged,
            connection_state: status.connection_state,
            operator: status.operator,
            apn: status.apn,
            sim_state: status.sim_state,
            pin_attempts: status.pin_attempts,
            embedded: status.embedded,
            roaming: status.roaming,
            imei: status.imei,
            imsi: status.imsi,
            iccid: status.iccid,
            phone_number: status.phone_number,
            signal: SignalSnapshot {
                level: status.signal.level,
                rssi: status.signal.rssi.map(keenetic_rci::Dbm::get),
                rsrp: status.signal.rsrp.map(keenetic_rci::Dbm::get),
                rsrq: status.signal.rsrq.map(keenetic_rci::Db::get),
                sinr: status.signal.cinr.map(keenetic_rci::Db::get),
            },
            temperature_c: status.modem_temperature.map(keenetic_rci::Celsius::get),
            cell: CellSnapshot {
                plmn: status.cell.plmn,
                enb_id: status.cell.enb_id,
                sector_id: status.cell.sector_id,
                tac: status.cell.tac,
            },
            primary_carrier,
            secondary_carriers,
            carrier_radio_statistics_support: CarrierRadioStatisticsSupport::Unsupported,
            modem: status.modem.map(|modem| ModemSnapshot {
                manufacturer: modem.manufacturer,
                model: modem.model,
                revision: modem.revision,
            }),
            sim_service_provider: status.sim.and_then(|sim| sim.service_provider),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct SignalSnapshot {
    pub level: Option<i64>,
    pub rssi: Option<f32>,
    pub rsrp: Option<f32>,
    pub rsrq: Option<f32>,
    /// Keenetic calls this value CINR; it is exposed as SINR for the dashboard.
    pub sinr: Option<f32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct TrafficSnapshot {
    pub download_bps: Option<u64>,
    pub upload_bps: Option<u64>,
}

impl From<InterfaceStat> for TrafficSnapshot {
    fn from(stat: InterfaceStat) -> Self {
        Self {
            download_bps: stat
                .receive_rate
                .map(keenetic_rci::DataRate::bits_per_second),
            upload_bps: stat
                .transmit_rate
                .map(keenetic_rci::DataRate::bits_per_second),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CellSnapshot {
    pub plmn: Option<Plmn>,
    pub enb_id: Option<u32>,
    pub sector_id: Option<u16>,
    pub tac: Option<u32>,
}

/// Modulation reported for a component carrier by a modem-specific query.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Modulation {
    #[serde(rename = "BPSK")]
    Bpsk,
    #[serde(rename = "QPSK")]
    Qpsk,
    #[serde(rename = "16QAM")]
    Qam16,
    #[serde(rename = "64QAM")]
    Qam64,
    #[serde(rename = "256QAM")]
    Qam256,
    #[serde(rename = "1024QAM")]
    Qam1024,
    #[serde(rename = "unknown")]
    Unknown,
}

/// Number of downlink MIMO layers, restricted to the cellular range of 1–4.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DownlinkLayers(NonZeroU8);

impl DownlinkLayers {
    #[must_use]
    pub const fn new(value: u8) -> Option<Self> {
        if value <= 4 {
            match NonZeroU8::new(value) {
                Some(value) => Some(Self(value)),
                None => None,
            }
        } else {
            None
        }
    }

    #[cfg(test)]
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CarrierSnapshot {
    pub active: Option<bool>,
    #[serde(serialize_with = "serialize_optional_display")]
    pub network: Option<RadioAccessTechnology>,
    #[serde(serialize_with = "serialize_optional_display")]
    pub band: Option<RadioBand>,
    pub earfcn: Option<u64>,
    pub downlink_frequency: Option<u64>,
    pub uplink_frequency: Option<u64>,
    pub bandwidth: Option<f32>,
    pub physical_cell_id: Option<u16>,
    pub downlink_layers: Option<DownlinkLayers>,
    pub downlink_modulation: Option<Modulation>,
    pub uplink_modulation: Option<Modulation>,
}

impl CarrierSnapshot {
    fn fill_missing_from(&mut self, other: Self) {
        self.active = self.active.or(other.active);
        self.network = self.network.take().or(other.network);
        self.band = self.band.take().or(other.band);
        self.earfcn = self.earfcn.or(other.earfcn);
        self.downlink_frequency = self.downlink_frequency.or(other.downlink_frequency);
        self.uplink_frequency = self.uplink_frequency.or(other.uplink_frequency);
        self.bandwidth = self.bandwidth.take().or(other.bandwidth);
        self.physical_cell_id = self.physical_cell_id.take().or(other.physical_cell_id);
        self.downlink_layers = self.downlink_layers.or(other.downlink_layers);
        self.downlink_modulation = self.downlink_modulation.or(other.downlink_modulation);
        self.uplink_modulation = self.uplink_modulation.or(other.uplink_modulation);
    }
}

impl From<ComponentCarrier> for CarrierSnapshot {
    fn from(carrier: ComponentCarrier) -> Self {
        Self {
            active: carrier.active,
            network: carrier.network,
            band: carrier.band,
            earfcn: carrier.earfcn,
            downlink_frequency: carrier.downlink_frequency,
            uplink_frequency: carrier.uplink_frequency,
            bandwidth: carrier
                .bandwidth
                .map(keenetic_rci::CarrierBandwidth::as_megahertz),
            physical_cell_id: carrier.physical_cell_id,
            downlink_layers: None,
            downlink_modulation: None,
            uplink_modulation: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ModemSnapshot {
    pub manufacturer: Option<Box<str>>,
    pub model: Option<Box<str>>,
    pub revision: Option<Box<str>>,
}

#[expect(
    clippy::ref_option,
    reason = "serde serialize_with passes a shared reference to the field"
)]
fn serialize_optional_display<T, S>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Display,
    S: Serializer,
{
    match value {
        Some(value) => serializer.collect_str(value),
        None => serializer.serialize_none(),
    }
}

pub fn unix_time_ms(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downlink_layers_encode_the_supported_range() {
        assert_eq!(DownlinkLayers::new(0), None);
        assert_eq!(DownlinkLayers::new(1).map(DownlinkLayers::get), Some(1));
        assert_eq!(DownlinkLayers::new(4).map(DownlinkLayers::get), Some(4));
        assert_eq!(DownlinkLayers::new(5), None);
    }

    fn interface_stat(rxspeed: Option<u64>, txspeed: Option<u64>) -> InterfaceStat {
        let mut value = serde_json::json!({
            "rxpackets": 0,
            "rx-multicast-packets": 0,
            "rx-broadcast-packets": 0,
            "rxbytes": 0,
            "rxerrors": 0,
            "rxdropped": 0,
            "txpackets": 0,
            "tx-multicast-packets": 0,
            "tx-broadcast-packets": 0,
            "txbytes": 0,
            "txerrors": 0,
            "txdropped": 0,
            "timestamp": "1.0",
            "last-overflow": "0.0"
        });
        let object = value.as_object_mut().unwrap();
        if let Some(rate) = rxspeed {
            object.insert("rxspeed".to_owned(), rate.into());
        }
        if let Some(rate) = txspeed {
            object.insert("txspeed".to_owned(), rate.into());
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn maps_cinr_to_sinr_and_preserves_missing_values() {
        let status: MobileStatus = serde_json::from_value(serde_json::json!({
            "rssi": "-62.5",
            "rsrp": -91,
            "rsrq": "-8.5",
            "cinr": "17",
            "temperature": "43.5",
            "mobile": "4G",
            "band": "B3",
            "phy-cell-id": "42",
            "imei": "000000000000000",
            "imsi": "001010000000001",
            "iccid": "890100000000000001",
            "plmn": "00101"
        }))
        .unwrap();
        let snapshot = MobileSnapshot::from(status);
        assert_eq!(snapshot.signal.rssi, Some(-62.5));
        assert_eq!(snapshot.signal.sinr, Some(17.0));
        assert_eq!(snapshot.temperature_c, Some(43.5));
        assert!(matches!(
            snapshot.iccid.as_ref(),
            Some(Reported::Unrecognized(_))
        ));
        assert_eq!(
            snapshot
                .primary_carrier
                .network
                .as_ref()
                .map(RadioAccessTechnology::as_str),
            Some("4G")
        );
        assert_eq!(snapshot.operator, None);

        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(json["imei"], "000000000000000");
        assert_eq!(json["imsi"], "001010000000001");
        assert_eq!(json["iccid"], "890100000000000001");
        assert_eq!(json["cell"]["plmn"], "00101");
        assert_eq!(json["primary_carrier"]["network"], "4G");
        assert_eq!(json["primary_carrier"]["band"], "B3");
        assert_eq!(json["carrier_radio_statistics_support"], "unsupported");
    }

    #[test]
    fn separates_primary_carrier_and_sequential_secondary_carriers() {
        let status: MobileStatus = serde_json::from_value(serde_json::json!({
            "mobile": "4G",
            "band": "7",
            "earfcn": 3048,
            "phy-cell-id": "56",
            "carrier": {
                "1": {
                    "active": true,
                    "mobile": "4G",
                    "band": "7",
                    "earfcn": 3048,
                    "phy-cell-id": "56",
                    "bandwidth": "20"
                },
                "2": {
                    "active": true,
                    "mobile": "4G",
                    "band": "1",
                    "earfcn": 150,
                    "phy-cell-id": "56",
                    "bandwidth": "10"
                },
                "3": {
                    "active": true,
                    "mobile": "4G",
                    "band": "7",
                    "earfcn": 3050,
                    "phy-cell-id": "57",
                    "bandwidth": "5"
                }
            }
        }))
        .unwrap();

        let snapshot = MobileSnapshot::from(status);

        assert_eq!(snapshot.primary_carrier.bandwidth, Some(20.0));
        assert_eq!(snapshot.secondary_carriers.len(), 2);
        assert_eq!(snapshot.secondary_carriers[0].earfcn, Some(150));
        assert_eq!(snapshot.secondary_carriers[1].physical_cell_id, Some(57));
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json["secondary_carriers"].is_array());
        assert!(json.get("additional_carriers").is_none());
    }

    #[test]
    fn merges_repeated_primary_carrier_with_stale_physical_cell_id() {
        let status: MobileStatus = serde_json::from_value(serde_json::json!({
            "mobile": "4G+",
            "band": "1",
            "earfcn": 200,
            "dl-freq": 2_130_000,
            "bandwidth": "10",
            "phy-cell-id": "84",
            "carrier": {
                "1": {
                    "active": true,
                    "mobile": "4G",
                    "band": "1",
                    "earfcn": 200,
                    "dl-freq": 2_130_000,
                    "bandwidth": "10",
                    "phy-cell-id": "56"
                }
            }
        }))
        .unwrap();

        assert!(
            status.primary_carrier.has_radio_identity_conflict(
                &status.reported_carriers[&ComponentCarrierId::PRIMARY]
            )
        );
        let snapshot = MobileSnapshot::from(status);
        assert!(snapshot.secondary_carriers.is_empty());
        assert_eq!(snapshot.primary_carrier.active, Some(true));
        assert_eq!(snapshot.primary_carrier.physical_cell_id, Some(84));
    }

    #[test]
    fn keeps_secondary_carriers_ordered_across_reported_id_gaps() {
        let status: MobileStatus = serde_json::from_value(serde_json::json!({
            "band": "7",
            "earfcn": 3048,
            "phy-cell-id": "56",
            "carrier": {
                "2": { "band": "1", "earfcn": 150 },
                "4": { "band": "20", "earfcn": 6300 }
            }
        }))
        .unwrap();

        let snapshot = MobileSnapshot::from(status);
        assert_eq!(snapshot.secondary_carriers.len(), 2);
        assert_eq!(snapshot.secondary_carriers[0].earfcn, Some(150));
        assert_eq!(snapshot.secondary_carriers[1].earfcn, Some(6300));
    }

    #[test]
    fn maps_interface_rates_and_serializes_independent_traffic() {
        let traffic = TrafficSnapshot::from(interface_stat(Some(7_158_920), Some(80_949)));
        assert_eq!(traffic.download_bps, Some(7_158_920));
        assert_eq!(traffic.upload_bps, Some(80_949));
        let ping = Arc::new(PingSnapshot::new(
            900,
            15,
            Arc::from("one.one.one.one"),
            PingOutcome::Replies {
                transmitted: NonZeroU16::new(3).unwrap(),
                received: NonZeroU16::new(2).unwrap(),
                average_rtt_ms: 12.5,
            },
        ));

        let sample = PendingSample {
            collected_at_ms: 1,
            poll_duration_ms: 2,
            router_id: Arc::from("test"),
            interface: Arc::from("UsbLte0"),
            traffic: Some(traffic),
            ping: Some(ping),
            outcome: PollOutcome::Failure {
                error: PollFailure {
                    kind: FailureKind::Transport,
                    message: "radio unavailable".to_owned(),
                },
            },
        }
        .sequence(3);
        let json = serde_json::to_value(sample).unwrap();
        assert_eq!(json["traffic"]["download_bps"], 7_158_920);
        assert_eq!(json["traffic"]["upload_bps"], 80_949);
        assert_eq!(json["ping"]["host"], "one.one.one.one");
        assert_eq!(json["ping"]["collected_at_ms"], 900);
        assert_eq!(json["ping"]["status"], "replies");
        assert_eq!(json["ping"]["average_rtt_ms"], 12.5);

        let missing = TrafficSnapshot::from(interface_stat(None, None));
        assert_eq!(
            missing,
            TrafficSnapshot {
                download_bps: None,
                upload_bps: None,
            }
        );

        let sample_without_traffic = PendingSample {
            collected_at_ms: 1,
            poll_duration_ms: 2,
            router_id: Arc::from("test"),
            interface: Arc::from("UsbLte0"),
            traffic: None,
            ping: None,
            outcome: PollOutcome::Failure {
                error: PollFailure {
                    kind: FailureKind::Transport,
                    message: "statistics unavailable".to_owned(),
                },
            },
        }
        .sequence(4);
        let json = serde_json::to_value(sample_without_traffic).unwrap();
        assert!(json["traffic"].is_null());

        let partial = TrafficSnapshot::from(interface_stat(Some(1_000_000), None));
        assert_eq!(partial.download_bps, Some(1_000_000));
        assert_eq!(partial.upload_bps, None);
    }
}
