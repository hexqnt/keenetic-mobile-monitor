use anyhow::{Context, Result};
use keenetic_rci::{CliCommand, CliReply, InterfaceId, KeeneticClient, LteModem, RadioBand};

use crate::sample::{
    CarrierRadioStatisticsSupport, CarrierSnapshot, DownlinkLayers, MobileSnapshot, Modulation,
};

mod fm350;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CarrierBand {
    Lte(u16),
    Nr(u16),
}

impl CarrierBand {
    const fn matches(self, band: &RadioBand) -> bool {
        match (self, band) {
            (Self::Lte(expected), RadioBand::Number(actual))
            | (Self::Nr(expected), RadioBand::Nr(actual)) => expected == *actual,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CarrierIdentity {
    band: CarrierBand,
    channel: u64,
    physical_cell_id: u16,
}

impl CarrierIdentity {
    const fn matches(self, carrier: &CarrierSnapshot) -> bool {
        let mut compared = false;
        if let Some(band) = &carrier.band {
            compared = true;
            if !self.band.matches(band) {
                return false;
            }
        }
        if let Some(channel) = carrier.earfcn {
            compared = true;
            if channel != self.channel {
                return false;
            }
        }
        if let Some(physical_cell_id) = carrier.physical_cell_id {
            compared = true;
            if physical_cell_id != self.physical_cell_id {
                return false;
            }
        }
        compared
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CarrierRadioDetails {
    identity: CarrierIdentity,
    downlink_layers: Option<DownlinkLayers>,
    downlink_modulation: Option<Modulation>,
    uplink_modulation: Option<Modulation>,
}

impl CarrierRadioDetails {
    const fn apply_to(self, carrier: &mut CarrierSnapshot) {
        carrier.downlink_layers = self.downlink_layers;
        carrier.downlink_modulation = self.downlink_modulation;
        carrier.uplink_modulation = self.uplink_modulation;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarrierDetails {
    primary: CarrierRadioDetails,
    secondary: Vec<CarrierRadioDetails>,
}

impl CarrierDetails {
    pub fn apply_to(self, snapshot: &mut MobileSnapshot) {
        if self.primary.identity.matches(&snapshot.primary_carrier) {
            self.primary.apply_to(&mut snapshot.primary_carrier);
        }
        for details in self.secondary {
            if let Some(carrier) = snapshot
                .secondary_carriers
                .iter_mut()
                .find(|carrier| details.identity.matches(carrier))
            {
                details.apply_to(carrier);
            }
        }
    }
}

trait CarrierDetailsDriver: Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, modem: &LteModem) -> bool;
    fn command(&self, interface: &InterfaceId) -> CliCommand;
    fn parse(&self, reply: &CliReply) -> Result<CarrierDetails>;
}

// Adding modem support only requires implementing the driver and registering it here.
static DRIVERS: &[&dyn CarrierDetailsDriver] = &[&fm350::DRIVER];

struct ConfiguredDriver {
    driver: &'static dyn CarrierDetailsDriver,
    command: CliCommand,
}

/// Selects and runs a model-specific carrier-details command.
pub struct CarrierDetailsPoller {
    drivers: Box<[ConfiguredDriver]>,
}

impl CarrierDetailsPoller {
    pub fn new(interface: &InterfaceId) -> Self {
        let drivers = DRIVERS
            .iter()
            .map(|driver| ConfiguredDriver {
                driver: *driver,
                command: driver.command(interface),
            })
            .collect();
        Self { drivers }
    }

    #[must_use]
    pub fn support(&self, modem: Option<&LteModem>) -> CarrierRadioStatisticsSupport {
        if modem.is_some_and(|modem| {
            self.drivers
                .iter()
                .any(|query| query.driver.supports(modem))
        }) {
            CarrierRadioStatisticsSupport::Supported
        } else {
            CarrierRadioStatisticsSupport::Unsupported
        }
    }

    pub async fn poll(
        &self,
        client: &KeeneticClient,
        modem: Option<&LteModem>,
    ) -> Result<Option<CarrierDetails>> {
        let Some(query) = modem.and_then(|modem| {
            self.drivers
                .iter()
                .find(|query| query.driver.supports(modem))
        }) else {
            return Ok(None);
        };
        let reply = client
            .execute_cli(&query.command)
            .await
            .with_context(|| format!("{} carrier-details command failed", query.driver.name()))?;
        let details = query.driver.parse(&reply).with_context(|| {
            format!(
                "{} carrier-details response is invalid",
                query.driver.name()
            )
        })?;
        Ok(Some(details))
    }
}

#[cfg(test)]
mod tests {
    use keenetic_rci::MobileStatus;

    use super::*;

    #[test]
    fn registry_selects_only_a_supported_modem() {
        let supported: MobileStatus = serde_json::from_value(serde_json::json!({
            "ati": { "model": "FM350-GL" }
        }))
        .unwrap();
        let other: MobileStatus = serde_json::from_value(serde_json::json!({
            "ati": { "model": "FM-1000" }
        }))
        .unwrap();

        let supported = supported.modem.as_ref().unwrap();
        let other = other.modem.as_ref().unwrap();
        let poller = CarrierDetailsPoller::new(&InterfaceId::new("UsbLte0").unwrap());
        assert_eq!(
            DRIVERS
                .iter()
                .filter(|driver| driver.supports(supported))
                .count(),
            1
        );
        assert!(!DRIVERS.iter().any(|driver| driver.supports(other)));
        assert_eq!(
            poller.support(Some(supported)),
            CarrierRadioStatisticsSupport::Supported
        );
        assert_eq!(
            poller.support(Some(other)),
            CarrierRadioStatisticsSupport::Unsupported
        );
        assert_eq!(
            poller.support(None),
            CarrierRadioStatisticsSupport::Unsupported
        );
    }
}
