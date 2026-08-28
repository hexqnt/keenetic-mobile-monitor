use anyhow::{Context, Result, bail};
use keenetic_rci::{CliCommand, CliReply, InterfaceId, LteModem};

use super::{
    CarrierBand, CarrierDetails, CarrierDetailsDriver, CarrierIdentity, CarrierRadioDetails,
};
use crate::sample::{DownlinkLayers, Modulation};

const MODEL: &str = "FM350-GL";
const UNAVAILABLE: u16 = 255;
const PCC_DOCUMENTED: FieldLayout = FieldLayout::new(0, 1, 2, 4, 6, 7);
const PCC_WITH_UL_BANDWIDTH: FieldLayout = FieldLayout::new(0, 1, 2, 5, 7, 8);
const SCC: FieldLayout = FieldLayout::new(2, 3, 4, 7, 9, 10);

pub(super) static DRIVER: Driver = Driver;

pub(super) struct Driver;

impl CarrierDetailsDriver for Driver {
    fn name(&self) -> &'static str {
        MODEL
    }

    fn supports(&self, modem: &LteModem) -> bool {
        modem
            .model
            .as_deref()
            .is_some_and(|model| model.eq_ignore_ascii_case(MODEL))
    }

    fn command(&self, interface: &InterfaceId) -> CliCommand {
        CliCommand::new(format!("interface {interface} tty send AT+GTCAINFO?"))
            .expect("a validated interface ID produces a valid CLI command")
    }

    fn parse(&self, reply: &CliReply) -> Result<CarrierDetails> {
        parse(reply.tty_output())
    }
}

fn parse<'a>(lines: impl IntoIterator<Item = &'a str>) -> Result<CarrierDetails> {
    let mut primary = None;
    let mut secondary = Vec::new();
    for line in lines {
        let line = line.trim();
        if let Some(fields) = line.strip_prefix("PCC:") {
            primary = Some(parse_primary(fields).context("invalid GTCAINFO PCC")?);
        } else if let Some((_, fields)) = line
            .strip_prefix("SCC")
            .and_then(|line| line.split_once(':'))
        {
            secondary.push(parse_secondary(fields).context("invalid GTCAINFO SCC")?);
        }
    }
    Ok(CarrierDetails {
        primary: primary.context("GTCAINFO response contains no PCC")?,
        secondary,
    })
}

fn parse_primary(fields: &str) -> Result<CarrierRadioDetails> {
    let fields = NumericFields::parse(fields)?;
    let layout = match fields.len {
        // The command manual documents a shorter PCC form without UL bandwidth.
        9 => PCC_DOCUMENTED,
        // FM350 firmware in the field includes UL bandwidth after DL bandwidth.
        10 => PCC_WITH_UL_BANDWIDTH,
        _ => bail!("expected 9 or 10 comma-separated fields"),
    };
    parse_carrier(&fields.values, layout)
}

fn parse_secondary(fields: &str) -> Result<CarrierRadioDetails> {
    let fields = NumericFields::parse(fields)?;
    if !(11..=14).contains(&fields.len) {
        bail!("expected between 11 and 14 comma-separated fields");
    }
    parse_carrier(&fields.values, SCC)
}

#[derive(Clone, Copy)]
struct FieldLayout {
    band: usize,
    physical_cell_id: usize,
    channel: usize,
    downlink_layers: usize,
    downlink_modulation: usize,
    uplink_modulation: usize,
}

impl FieldLayout {
    const fn new(
        band: usize,
        physical_cell_id: usize,
        channel: usize,
        downlink_layers: usize,
        downlink_modulation: usize,
        uplink_modulation: usize,
    ) -> Self {
        Self {
            band,
            physical_cell_id,
            channel,
            downlink_layers,
            downlink_modulation,
            uplink_modulation,
        }
    }
}

struct NumericFields {
    values: [i32; 14],
    len: usize,
}

impl NumericFields {
    fn parse(fields: &str) -> Result<Self> {
        let mut parsed = Self {
            values: [0; 14],
            len: 0,
        };
        for field in fields.split(',') {
            let Some(slot) = parsed.values.get_mut(parsed.len) else {
                bail!("expected at most 14 comma-separated fields");
            };
            *slot = field
                .trim()
                .parse()
                .with_context(|| format!("invalid numeric field {field:?}"))?;
            parsed.len += 1;
        }
        Ok(parsed)
    }
}

fn parse_carrier(fields: &[i32], layout: FieldLayout) -> Result<CarrierRadioDetails> {
    Ok(CarrierRadioDetails {
        identity: CarrierIdentity {
            band: parse_band(
                u32::try_from(fields[layout.band]).context("band code cannot be negative")?,
            )?,
            channel: u64::try_from(fields[layout.channel]).context("channel cannot be negative")?,
            physical_cell_id: u16::try_from(fields[layout.physical_cell_id])
                .context("physical cell ID is out of range")?,
        },
        downlink_layers: parse_downlink_layers(fields[layout.downlink_layers])?,
        downlink_modulation: parse_modulation(
            u16::try_from(fields[layout.downlink_modulation])
                .context("DL modulation cannot be negative")?,
        )?,
        uplink_modulation: parse_modulation(
            u16::try_from(fields[layout.uplink_modulation])
                .context("UL modulation cannot be negative")?,
        )?,
    })
}

fn parse_band(value: u32) -> Result<CarrierBand> {
    match value {
        101..=171 => Ok(CarrierBand::Lte(
            u16::try_from(value - 100).expect("LTE band encoding fits u16"),
        )),
        501..=509 => Ok(CarrierBand::Nr(
            u16::try_from(value - 500).expect("short NR band encoding fits u16"),
        )),
        5_010..=50_512 => Ok(CarrierBand::Nr(
            u16::try_from(value - 5_000).expect("NR band encoding fits u16"),
        )),
        _ => bail!("unsupported FM350 band code {value}"),
    }
}

fn parse_downlink_layers(value: i32) -> Result<Option<DownlinkLayers>> {
    if value == 0 || value == i32::from(UNAVAILABLE) {
        return Ok(None);
    }
    let value = u8::try_from(value).context("downlink layer count is out of range")?;
    DownlinkLayers::new(value)
        .map(Some)
        .context("downlink layer count must be between 1 and 4")
}

fn parse_modulation(value: u16) -> Result<Option<Modulation>> {
    Ok(match value {
        UNAVAILABLE => None,
        0 => Some(Modulation::Bpsk),
        1 => Some(Modulation::Qpsk),
        2 => Some(Modulation::Qam16),
        3 => Some(Modulation::Qam64),
        4 => Some(Modulation::Qam256),
        5 => Some(Modulation::Qam1024),
        6 => Some(Modulation::Unknown),
        _ => bail!("unsupported FM350 modulation code {value}"),
    })
}

#[cfg(test)]
mod tests {
    use keenetic_rci::MobileStatus;

    use super::*;
    use crate::sample::MobileSnapshot;

    const RESPONSE: [&str; 7] = [
        "+GTCAINFO:",
        "PCC:101,84,200,25,25,1,1,3,3,-98",
        "SCC 1:2,0,107,84,2850,100,255,1,255,2,255,-98",
        "SCC 2:2,0,107,84,3048,100,255,1,255,2,255,-98",
        "SCC 3:2,0,103,84,1596,50,255,1,255,2,255,-98",
        "OK",
        "",
    ];

    #[test]
    fn builds_command_for_the_selected_interface() {
        let interface = InterfaceId::new("UsbLte1").unwrap();
        assert_eq!(
            DRIVER.command(&interface).as_str(),
            "interface UsbLte1 tty send AT+GTCAINFO?"
        );
    }

    #[test]
    fn parses_carrier_details() {
        let details = parse(RESPONSE).unwrap();

        assert_eq!(details.primary.downlink_layers.unwrap().get(), 1);
        assert_eq!(details.primary.downlink_modulation, Some(Modulation::Qam64));
        assert_eq!(details.primary.uplink_modulation, Some(Modulation::Qam64));
        assert_eq!(details.secondary.len(), 3);
        assert_eq!(details.secondary[0].downlink_layers.unwrap().get(), 1);
        assert_eq!(
            details.secondary[0].downlink_modulation,
            Some(Modulation::Qam16)
        );
        assert_eq!(details.secondary[0].uplink_modulation, None);
    }

    #[test]
    fn enriches_secondary_carriers_by_radio_identity() {
        let status: MobileStatus = serde_json::from_value(serde_json::json!({
            "band": "1", "earfcn": 200, "phy-cell-id": "84",
            "carrier": {
                "2": { "band": "3", "earfcn": 1596, "phy-cell-id": "84" },
                "3": { "band": "7", "earfcn": 2850, "phy-cell-id": "84" }
            }
        }))
        .unwrap();
        let mut snapshot = MobileSnapshot::from(status);

        parse([
            "+GTCAINFO:",
            "PCC:101,84,200,25,25,1,1,3,3,-98",
            "SCC 1:2,0,107,84,2850,100,255,1,255,2,255,-98",
            "SCC 2:2,0,103,84,1596,50,255,1,255,4,255,-98",
            "OK",
        ])
        .unwrap()
        .apply_to(&mut snapshot);

        assert_eq!(snapshot.primary_carrier.downlink_layers.unwrap().get(), 1);
        assert_eq!(
            snapshot.secondary_carriers[0].downlink_modulation,
            Some(Modulation::Qam256)
        );
        assert_eq!(
            snapshot.secondary_carriers[1].downlink_modulation,
            Some(Modulation::Qam16)
        );
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json["primary_carrier"]["downlink_layers"], 1);
        assert_eq!(json["primary_carrier"]["downlink_modulation"], "64QAM");
        assert_eq!(json["primary_carrier"]["uplink_modulation"], "64QAM");
        assert!(json["secondary_carriers"][0]["uplink_modulation"].is_null());
    }

    #[test]
    fn ignores_primary_details_from_a_different_radio_context() {
        let status: MobileStatus = serde_json::from_value(serde_json::json!({
            "band": "3", "earfcn": 1596, "phy-cell-id": "84"
        }))
        .unwrap();
        let mut snapshot = MobileSnapshot::from(status);

        parse(["PCC:101,56,200,25,25,1,1,3,3,-98", "OK"])
            .unwrap()
            .apply_to(&mut snapshot);

        assert_eq!(snapshot.primary_carrier.downlink_layers, None);
        assert_eq!(snapshot.primary_carrier.downlink_modulation, None);
        assert_eq!(snapshot.primary_carrier.uplink_modulation, None);
    }

    #[test]
    fn rejects_malformed_relevant_lines() {
        let error = parse(["+GTCAINFO:", "PCC:101,84,bad"]).unwrap_err();
        assert!(error.to_string().contains("invalid GTCAINFO PCC"));
        assert!(parse(["+GTCAINFO:", "OK"]).is_err());
    }

    #[test]
    fn accepts_documented_optional_scc_measurements() {
        let details = parse([
            "+GTCAINFO:",
            "PCC:101,84,200,25,25,1,1,3,3,-98",
            "SCC 1:2,0,107,84,2850,100,255,1,255,2,255,42,-98,-10",
            "OK",
        ])
        .unwrap();

        assert_eq!(details.secondary.len(), 1);
        assert_eq!(
            details.secondary[0].downlink_modulation,
            Some(Modulation::Qam16)
        );
    }

    #[test]
    fn parses_documented_pcc_layout_without_ul_bandwidth() {
        let details = parse(["+GTCAINFO:", "PCC:101,84,200,25,1,1,3,3,-98", "OK"]).unwrap();

        assert_eq!(details.primary.downlink_layers.unwrap().get(), 1);
        assert_eq!(details.primary.downlink_modulation, Some(Modulation::Qam64));
        assert_eq!(details.primary.uplink_modulation, Some(Modulation::Qam64));
    }
}
