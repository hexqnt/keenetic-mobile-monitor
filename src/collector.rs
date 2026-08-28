use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result, bail};
use keenetic_rci::{
    ComponentCarrier, ComponentCarrierId, InterfaceId, KeeneticClient, MobileStatus,
    request::{ShowInterfaceStat, ShowInterfaces, ShowLteInterface},
};
use tokio::sync::{RwLock, broadcast, watch};
use tokio::time::{MissedTickBehavior, interval_at};

use crate::modem::CarrierDetailsPoller;
use crate::sample::{
    LteSample, MobileSnapshot, PendingSample, PingSnapshot, PollFailure, PollOutcome,
    TrafficSnapshot, duration_ms, unix_time_ms,
};

fn conflicting_reported_primary(status: &MobileStatus) -> Option<&ComponentCarrier> {
    status
        .reported_carriers
        .get(&ComponentCarrierId::PRIMARY)
        .filter(|reported| status.primary_carrier.has_radio_identity_conflict(reported))
}

fn warn_on_primary_carrier_conflict(interface: &str, status: &MobileStatus) {
    let Some(reported) = conflicting_reported_primary(status) else {
        return;
    };
    tracing::warn!(
        interface,
        primary_band = ?status.primary_carrier.band,
        primary_earfcn = ?status.primary_carrier.earfcn,
        primary_pci = ?status.primary_carrier.physical_cell_id,
        carrier_1_band = ?reported.band,
        carrier_1_earfcn = ?reported.earfcn,
        carrier_1_pci = ?reported.physical_cell_id,
        "Keenetic carrier 1 conflicts with the top-level primary carrier"
    );
}

pub struct History {
    samples: RwLock<Vec<Arc<LteSample>>>,
    updates: broadcast::Sender<Arc<LteSample>>,
}

pub enum InitialHistory {
    Snapshot(Vec<Arc<LteSample>>),
    Replay(Vec<Arc<LteSample>>),
}

impl History {
    pub fn new() -> Self {
        let (updates, _) = broadcast::channel(64);
        Self {
            samples: RwLock::new(Vec::new()),
            updates,
        }
    }

    pub async fn push(&self, sample: PendingSample) -> u64 {
        let mut samples = self.samples.write().await;
        let id = u64::try_from(samples.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let sample = Arc::new(sample.sequence(id));
        samples.push(Arc::clone(&sample));
        drop(samples);
        let _ = self.updates.send(sample);
        id
    }

    pub async fn after(&self, id: u64) -> Vec<Arc<LteSample>> {
        let samples = self.samples.read().await;
        let start = usize::try_from(id).unwrap_or(usize::MAX).min(samples.len());
        samples[start..].to_vec()
    }

    pub async fn initial(&self, last_id: Option<u64>) -> InitialHistory {
        let samples = self.samples.read().await;
        let latest_id = samples.last().map_or(0, |sample| sample.id());
        match last_id {
            Some(id) if id <= latest_id => {
                let start = usize::try_from(id).unwrap_or(usize::MAX).min(samples.len());
                InitialHistory::Replay(samples[start..].to_vec())
            }
            Some(_) | None => InitialHistory::Snapshot(samples.clone()),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<LteSample>> {
        self.updates.subscribe()
    }
}

fn take_ping_update(
    receiver: &mut watch::Receiver<Option<Arc<PingSnapshot>>>,
) -> Option<Arc<PingSnapshot>> {
    receiver
        .has_changed()
        .is_ok_and(|changed| changed)
        .then(|| receiver.borrow_and_update().clone())
        .flatten()
}

pub async fn resolve_interface(
    client: &KeeneticClient,
    configured: Option<InterfaceId>,
) -> Result<InterfaceId> {
    if let Some(interface) = configured {
        return Ok(interface);
    }
    let interfaces = client
        .execute(ShowInterfaces)
        .await
        .context("failed to discover Mobile interfaces")?;
    let Some((name, _)) = interfaces
        .into_iter()
        .find(|(_, interface)| interface.is_mobile_broadband())
    else {
        bail!("the router did not report a Mobile interface");
    };
    Ok(name)
}

pub async fn run(
    client: KeeneticClient,
    router_id: Arc<str>,
    poll_interval: Duration,
    interface: InterfaceId,
    history: Arc<History>,
    mut latest_ping: watch::Receiver<Option<Arc<PingSnapshot>>>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut ticker = interval_at(tokio::time::Instant::now(), poll_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let interface_id: Arc<str> = Arc::from(interface.as_str());
    let carrier_details = CarrierDetailsPoller::new(&interface);
    let mobile_request = ShowLteInterface::from_id(interface.clone());
    let traffic_request = ShowInterfaceStat::from_id(interface);
    let shutdown_requested = crate::shutdown::requested(&mut shutdown);
    tokio::pin!(shutdown_requested);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let started = Instant::now();
                let mobile_request = client.execute(&mobile_request);
                let traffic_request = client.execute(&traffic_request);
                let (mobile_result, traffic_result) = tokio::join!(mobile_request, traffic_request);
                let outcome = match mobile_result {
                    Ok(reply) => {
                        let (_, status) = reply.into_interface().into_parts();
                        warn_on_primary_carrier_conflict(&interface_id, &status);
                        let carrier_radio_statistics_support =
                            carrier_details.support(status.modem.as_ref());
                        let details = match carrier_details
                            .poll(&client, status.modem.as_ref())
                            .await
                        {
                            Ok(details) => details,
                            Err(error) => {
                                tracing::warn!(error = ?error, "Modem carrier-details poll failed");
                                None
                            }
                        };
                        let mut mobile = MobileSnapshot::from(status);
                        mobile.carrier_radio_statistics_support =
                            carrier_radio_statistics_support;
                        if let Some(details) = details {
                            details.apply_to(&mut mobile);
                        }
                        PollOutcome::Success {
                            mobile: Box::new(mobile),
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = ?error, "Keenetic poll failed");
                        PollOutcome::Failure { error: PollFailure::from_error(&error) }
                    }
                };
                let traffic = match traffic_result {
                    Ok(stat) => Some(TrafficSnapshot::from(stat)),
                    Err(error) => {
                        tracing::warn!(error = ?error, "Keenetic interface statistics poll failed");
                        None
                    }
                };
                let sample = PendingSample {
                    collected_at_ms: unix_time_ms(SystemTime::now()),
                    poll_duration_ms: duration_ms(started.elapsed()),
                    router_id: Arc::clone(&router_id),
                    interface: Arc::clone(&interface_id),
                    traffic,
                    ping: take_ping_update(&mut latest_ping),
                    outcome,
                };
                let sample_id = history.push(sample).await;
                tracing::debug!(sample_id, "Sample collected");
            }
            () = &mut shutdown_requested => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;
    use crate::sample::PingOutcome;

    #[test]
    fn ping_update_is_attached_to_only_one_sample() {
        let (updates, mut latest) = watch::channel(None);
        assert!(take_ping_update(&mut latest).is_none());

        let ping = Arc::new(PingSnapshot::new(
            1,
            2,
            Arc::from("example.com"),
            PingOutcome::NoReply {
                transmitted: NonZeroU16::new(3).unwrap(),
            },
        ));
        updates.send_replace(Some(Arc::clone(&ping)));

        let attached = take_ping_update(&mut latest).unwrap();
        assert!(Arc::ptr_eq(&attached, &ping));
        assert!(take_ping_update(&mut latest).is_none());
    }

    #[test]
    fn selects_only_a_conflicting_reported_primary_carrier() {
        let matching: MobileStatus = serde_json::from_value(serde_json::json!({
            "band": "7",
            "earfcn": 3048,
            "phy-cell-id": "56",
            "carrier": {
                "1": { "band": "7", "earfcn": 3048, "phy-cell-id": "56" }
            }
        }))
        .unwrap();
        assert!(conflicting_reported_primary(&matching).is_none());

        let conflicting: MobileStatus = serde_json::from_value(serde_json::json!({
            "band": "7",
            "earfcn": 3048,
            "phy-cell-id": "84",
            "carrier": {
                "1": { "band": "7", "earfcn": 3048, "phy-cell-id": "56" }
            }
        }))
        .unwrap();
        assert!(conflicting_reported_primary(&conflicting).is_some());
    }
}
