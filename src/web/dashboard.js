const PLOT_CONFIG = { responsive: true, displaylogo: false };
const SIGNALS = [
	{ field: "rssi", title: "RSSI", unit: "dBm", axis: "y", color: "#38bdf8" },
	{ field: "rsrp", title: "RSRP", unit: "dBm", axis: "y", color: "#818cf8" },
	{ field: "rsrq", title: "RSRQ", unit: "dB", axis: "y2", color: "#fb7185" },
	{ field: "sinr", title: "SINR", unit: "dB", axis: "y2", color: "#fbbf24" },
];
const TRAFFIC = [
	{ field: "download_bps", title: "Download", color: "#22c55e" },
	{ field: "upload_bps", title: "Upload", color: "#f97316" },
];
const PING_HOST = document.body.dataset.pingHost;
const PING_TRACE_INDEX = TRAFFIC.length;
const BITS_PER_MEGABIT = 1_000_000;
const ALL_TIME_CHART_IDS = [
	"signal-chart",
	"traffic-chart",
	"temperature-chart",
	"context-chart",
];
const BASE_LANES = ["RAT", "Cell", "PCell"];
const PALETTE = [
	"#38bdf8",
	"#818cf8",
	"#fb7185",
	"#fbbf24",
	"#2dd4bf",
	"#c084fc",
	"#f97316",
];
const MODULATION_ORDER = [
	"BPSK",
	"QPSK",
	"16QAM",
	"64QAM",
	"256QAM",
	"1024QAM",
	"Unknown",
];
const MIMO_LAYER_ORDER = [
	"1 layer",
	"2 layers",
	"3 layers",
	"4 layers",
	"Unknown",
];
const MODULATION_RANK = new Map(
	MODULATION_ORDER.map((value, index) => [value, index]),
);
const MIMO_LAYER_RANK = new Map(
	MIMO_LAYER_ORDER.map((value, index) => [value, index]),
);
const CARRIER_MEASUREMENT_FIELDS = [
	"downlink_modulation",
	"uplink_modulation",
	"downlink_layers",
];

const signalLayout = {
	title: { text: "Radio signal", font: { color: "#e8edf7", size: 16 } },
	paper_bgcolor: "transparent",
	plot_bgcolor: "transparent",
	font: { color: "#94a3b8" },
	margin: { l: 58, r: 58, t: 58, b: 42 },
	hovermode: "x unified",
	legend: { orientation: "h", y: 1.08 },
	uirevision: "radio-time",
	xaxis: {
		type: "date",
		gridcolor: "#273451",
		showspikes: true,
		spikemode: "across",
		spikesnap: "cursor",
	},
	yaxis: {
		title: { text: "RSSI / RSRP, dBm", standoff: 8 },
		automargin: true,
		gridcolor: "#273451",
		fixedrange: false,
	},
	yaxis2: {
		title: { text: "RSRQ / SINR, dB", standoff: 8 },
		overlaying: "y",
		side: "right",
		automargin: true,
		showgrid: false,
		fixedrange: false,
	},
};

const trafficLayout = {
	title: {
		text: PING_HOST ? "Interface traffic and ping" : "Interface traffic",
		font: { color: "#e8edf7", size: 16 },
	},
	paper_bgcolor: "transparent",
	plot_bgcolor: "transparent",
	font: { color: "#94a3b8" },
	margin: { l: 92, r: PING_HOST ? 72 : 24, t: 58, b: 42 },
	hovermode: "x unified",
	legend: { orientation: "h", y: 1.08 },
	uirevision: "radio-time",
	xaxis: {
		type: "date",
		gridcolor: "#273451",
		showspikes: true,
		spikemode: "across",
		spikesnap: "cursor",
	},
	yaxis: {
		title: { text: "Mbit/s", standoff: 8 },
		automargin: true,
		gridcolor: "#273451",
		rangemode: "tozero",
		tickformat: ".2~f",
		fixedrange: false,
	},
	...(PING_HOST
		? {
				yaxis2: {
					title: { text: "RTT, ms", standoff: 8 },
					overlaying: "y",
					side: "right",
					automargin: true,
					rangemode: "tozero",
					tickformat: ".2~f",
					showgrid: false,
					fixedrange: false,
				},
			}
		: {}),
};

const temperatureLayout = {
	title: { text: "Modem temperature", font: { color: "#e8edf7", size: 16 } },
	paper_bgcolor: "transparent",
	plot_bgcolor: "transparent",
	font: { color: "#94a3b8" },
	margin: { l: 72, r: 24, t: 58, b: 42 },
	hovermode: "x unified",
	uirevision: "radio-time",
	xaxis: {
		type: "date",
		gridcolor: "#273451",
		showspikes: true,
		spikemode: "across",
		spikesnap: "cursor",
	},
	yaxis: {
		title: { text: "°C", standoff: 8 },
		automargin: true,
		gridcolor: "#273451",
		fixedrange: false,
	},
};

const chartsReady = Promise.all([
	Plotly.newPlot(
		"signal-chart",
		SIGNALS.map((signal) => ({
			name: signal.title,
			x: [],
			y: [],
			yaxis: signal.axis,
			mode: "lines",
			connectgaps: false,
			line: { color: signal.color, width: 1.8 },
			hovertemplate: `%{y:.1f} ${signal.unit}<extra></extra>`,
		})),
		signalLayout,
		PLOT_CONFIG,
	),
	Plotly.newPlot(
		"traffic-chart",
		[
			...TRAFFIC.map((traffic) => ({
				name: traffic.title,
				x: [],
				y: [],
				mode: "lines",
				connectgaps: false,
				line: { color: traffic.color, width: 1.8 },
				hovertemplate: "%{y:.2f} Mbit/s<extra></extra>",
			})),
			...(PING_HOST
				? [
						{
							name: `Ping avg (${PING_HOST})`,
							x: [],
							y: [],
							yaxis: "y2",
							mode: "lines",
							connectgaps: false,
							line: { color: "#c084fc", width: 1.8 },
							hovertemplate: "%{y:.2f} ms<extra></extra>",
						},
					]
				: []),
		],
		trafficLayout,
		PLOT_CONFIG,
	),
	Plotly.newPlot(
		"temperature-chart",
		[
			{
				name: "Temperature",
				x: [],
				y: [],
				mode: "lines",
				connectgaps: false,
				line: { color: "#f97316", width: 1.8 },
				hovertemplate: "%{y:.1f} °C<extra></extra>",
			},
		],
		temperatureLayout,
		PLOT_CONFIG,
	),
	Plotly.newPlot(
		"context-chart",
		[],
		contextLayout(["Events", ...BASE_LANES]),
		PLOT_CONFIG,
	),
]).then(wireRangeSynchronization);

const groups = new Map();
const carrierStatistics = new Map();
const contextCache = new WeakMap();
const timeline = new Map();
const secondaryLanes = new Set();
const radioEvents = [];

let lastId = 0;
let lastSample = null;
let lastSuccess = null;
let lastSuccessfulGroupKey = null;
let contextGroupInterrupted = true;
let estimatedIntervalMs = 1_000;
let outage = null;
let derivedRenderPending = false;
let currentAggregationText = null;
let copyFeedbackTimer = null;
let carrierRadioStatisticsSupported = null;

function present(value) {
	return value !== null && value !== undefined && value !== "";
}

function display(value) {
	return present(value) ? String(value) : "—";
}

function escapeHover(value) {
	return display(value)
		.replaceAll("&", "&amp;")
		.replaceAll("<", "&lt;")
		.replaceAll(">", "&gt;")
		.replaceAll('"', "&quot;")
		.replaceAll("'", "&#39;");
}

function joinPresent(values, separator = " / ") {
	const parts = values.filter(present).map(String);
	return parts.length ? parts.join(separator) : "—";
}

function directionalPair(downlink, uplink) {
	if (!present(downlink) && !present(uplink)) return "—";
	return `${display(downlink)} / ${display(uplink)}`;
}

function formatBand(band) {
	if (!present(band)) return "—";
	const text = String(band);
	return /^\d+$/.test(text) ? `B${text}` : text;
}

function carrierBand(carrier) {
	return formatBand(carrier?.band);
}

function carrierBandAndBandwidth(carrier) {
	const band = carrierBand(carrier);
	if (band === "—" || !present(carrier?.bandwidth)) return band;
	const bandwidth = String(carrier.bandwidth).trim();
	const suffix = /^\d+(?:[.,]\d+)?$/.test(bandwidth) ? " MHz" : "";
	return `${band}@${bandwidth}${suffix}`;
}

function carrierNetworkIdentity(network) {
	return network === "4G+" ? "4G" : (network ?? null);
}

function carrierIdentity(carrier) {
	if (!carrier) return ["—", null, null, null, null];
	return [
		carrierNetworkIdentity(carrier.network),
		carrier.band ?? null,
		carrier.earfcn ?? null,
		carrier.physical_cell_id ?? null,
		carrier.bandwidth ?? null,
	];
}

function carrierLabel(carrier, detailed = false) {
	if (!carrier) return "—";
	const parts = [carrierBandAndBandwidth(carrier)];
	if (present(carrier.earfcn)) parts.push(`EARFCN ${carrier.earfcn}`);
	if (detailed && present(carrier.physical_cell_id))
		parts.push(`PCI ${carrier.physical_cell_id}`);
	return parts.filter((part) => part !== "—").join(" · ") || "—";
}

function secondaryCarriers(mobile) {
	return Array.isArray(mobile.secondary_carriers)
		? mobile.secondary_carriers
		: [];
}

function activeSecondaryCarriers(mobile) {
	return secondaryCarriers(mobile).filter((carrier) => carrier.active === true);
}

function activeCarrierIdentities(mobile) {
	return activeSecondaryCarriers(mobile)
		.map(carrierIdentity)
		.sort((left, right) =>
			JSON.stringify(left).localeCompare(JSON.stringify(right)),
		);
}

function siteLabel(mobile) {
	return joinPresent([mobile.cell?.plmn, mobile.cell?.enb_id]);
}

function sectorLabel(mobile) {
	const sector = present(mobile.cell?.sector_id)
		? `sector ${mobile.cell.sector_id}`
		: null;
	const tac = present(mobile.cell?.tac) ? `TAC ${mobile.cell.tac}` : null;
	return joinPresent([sector, tac]);
}

function sectorValue(mobile) {
	return joinPresent([mobile.cell?.sector_id, mobile.cell?.tac]);
}

function cellLabel(mobile) {
	const site = siteLabel(mobile);
	const sector = sectorLabel(mobile);
	return sector === "—" ? site : `${site} · ${sector}`;
}

function aggregationLabel(mobile) {
	const carriers = [mobile.primary_carrier, ...activeSecondaryCarriers(mobile)];
	const bands = carriers
		.map(carrierBandAndBandwidth)
		.filter((band) => band !== "—");
	if (!bands.length) return "—";
	return bands.length === 1 ? `${bands[0]} · PCell only` : bands.join(" + ");
}

function contextInfo(mobile) {
	const cached = contextCache.get(mobile);
	if (cached) return cached;
	const active = activeCarrierIdentities(mobile);
	const identity = [
		mobile.cell?.plmn ?? null,
		mobile.cell?.enb_id ?? null,
		mobile.cell?.sector_id ?? null,
		mobile.cell?.tac ?? null,
		...carrierIdentity(mobile.primary_carrier),
		active,
	];
	const info = {
		key: JSON.stringify(identity),
		site: siteLabel(mobile),
		sector: sectorLabel(mobile),
		primary: carrierLabel(mobile.primary_carrier, true),
		aggregation: aggregationLabel(mobile),
	};
	contextCache.set(mobile, info);
	return info;
}

function createMetric() {
	return { count: 0, frequencies: new Map(), orderedFrequencies: null };
}

function createDistribution() {
	return { count: 0, observed: 0, frequencies: new Map() };
}

function modulationCategory(value) {
	if (!present(value) || String(value).toLowerCase() === "unknown")
		return "Unknown";
	return String(value);
}

function mimoLayerCategory(value) {
	if (!present(value)) return "Unknown";
	return `${value} ${Number(value) === 1 ? "layer" : "layers"}`;
}

function addDistribution(distribution, value, category) {
	distribution.count += 1;
	distribution.observed += Number(present(value));
	const bucket = category(value);
	distribution.frequencies.set(
		bucket,
		(distribution.frequencies.get(bucket) ?? 0) + 1,
	);
}

function carrierStatisticsKey(mobile, carrier) {
	return JSON.stringify([
		mobile.cell?.plmn ?? null,
		mobile.cell?.enb_id ?? null,
		mobile.cell?.sector_id ?? null,
		mobile.cell?.tac ?? null,
		carrierNetworkIdentity(carrier?.network),
		carrier?.band ?? null,
		carrier?.earfcn ?? null,
		carrier?.physical_cell_id ?? null,
	]);
}

function carrierStatisticsInfo(mobile, carrier) {
	return {
		key: carrierStatisticsKey(mobile, carrier),
		carrier: carrierLabel(carrier, true),
		cell: cellLabel(mobile),
	};
}

function* observedCarriers(mobile) {
	if (mobile.primary_carrier) yield ["PCell", mobile.primary_carrier];
	for (const carrier of secondaryCarriers(mobile)) {
		if (carrier.active === true) yield ["SCell", carrier];
	}
}

function createCarrierGroup(info) {
	return {
		...info,
		roles: new Set(),
		samples: 0,
		durationMs: 0,
		hasMeasurements: false,
		downlinkModulation: createDistribution(),
		uplinkModulation: createDistribution(),
		downlinkLayers: createDistribution(),
	};
}

function updateCarrierGroups(mobile) {
	for (const [role, carrier] of observedCarriers(mobile)) {
		const info = carrierStatisticsInfo(mobile, carrier);
		let group = carrierStatistics.get(info.key);
		if (!group) {
			group = createCarrierGroup(info);
			carrierStatistics.set(info.key, group);
		}
		group.roles.add(role);
		group.samples += 1;
		group.hasMeasurements ||= CARRIER_MEASUREMENT_FIELDS.some((field) =>
			present(carrier[field]),
		);
		addDistribution(
			group.downlinkModulation,
			carrier.downlink_modulation,
			modulationCategory,
		);
		addDistribution(
			group.uplinkModulation,
			carrier.uplink_modulation,
			modulationCategory,
		);
		addDistribution(
			group.downlinkLayers,
			carrier.downlink_layers,
			mimoLayerCategory,
		);
	}
}

function accountCarrierDurations(mobile, elapsed) {
	for (const [, carrier] of observedCarriers(mobile)) {
		const group = carrierStatistics.get(carrierStatisticsKey(mobile, carrier));
		if (group) group.durationMs += elapsed;
	}
}

function createGroup(info) {
	return {
		...info,
		samples: 0,
		entries: 0,
		durationMs: 0,
		lowSinr: 0,
		maxTraffic: { download_bps: null, upload_bps: null },
		metrics: {
			rssi: createMetric(),
			rsrp: createMetric(),
			rsrq: createMetric(),
			sinr: createMetric(),
		},
	};
}

function hasSignalMeasurements(mobile) {
	return SIGNALS.some((signal) =>
		Number.isFinite(mobile.signal?.[signal.field]),
	);
}

function addMetric(metric, value) {
	if (!Number.isFinite(value)) return;
	metric.count += 1;
	metric.frequencies.set(value, (metric.frequencies.get(value) ?? 0) + 1);
	metric.orderedFrequencies = null;
}

function valueAtRank(metric, rank) {
	let seen = 0;
	metric.orderedFrequencies ??= [...metric.frequencies.entries()].sort(
		(a, b) => a[0] - b[0],
	);
	for (const [value, count] of metric.orderedFrequencies) {
		seen += count;
		if (seen > rank) return value;
	}
	return null;
}

function quantile(metric, percentile) {
	if (!metric.count) return null;
	const position = (metric.count - 1) * percentile;
	const lowerRank = Math.floor(position);
	const upperRank = Math.ceil(position);
	const lower = valueAtRank(metric, lowerRank);
	const upper = valueAtRank(metric, upperRank);
	return lower + (upper - lower) * (position - lowerRank);
}

function metricSummary(metric) {
	if (!metric.count) return "—";
	return `${quantile(metric, 0.5).toFixed(1)} / ${quantile(metric, 0.1).toFixed(1)}`;
}

function sinrSummary(group) {
	const metric = group.metrics.sinr;
	if (!metric.count) return "—";
	const lowPercentage = ((group.lowSinr / metric.count) * 100).toFixed(1);
	return `${metricSummary(metric)} · ${lowPercentage}%`;
}

function updateTrafficMaximums(maximums, traffic) {
	for (const { field } of TRAFFIC) {
		const value = traffic?.[field];
		if (
			Number.isFinite(value) &&
			(maximums[field] === null || value > maximums[field])
		)
			maximums[field] = value;
	}
}

function trafficMaximumSummary(maximums) {
	const format = (value) =>
		Number.isFinite(value) ? (value / BITS_PER_MEGABIT).toFixed(2) : "—";
	if (!Object.values(maximums).some(Number.isFinite)) return "—";
	return `${format(maximums.download_bps)} / ${format(maximums.upload_bps)}`;
}

function observationSummary(durationMs, totalDurationMs) {
	const share =
		totalDurationMs > 0 ? percentage(durationMs, totalDurationMs) : "—";
	return `${formatDuration(durationMs)} · ${share}`;
}

function updateGroup(sample) {
	const mobile = sample.mobile;
	if (!hasSignalMeasurements(mobile)) return;
	const info = contextInfo(mobile);
	let group = groups.get(info.key);
	if (!group) {
		group = createGroup(info);
		groups.set(info.key, group);
	}
	group.samples += 1;
	if (contextGroupInterrupted || lastSuccessfulGroupKey !== info.key) {
		group.entries += 1;
	}
	for (const signal of SIGNALS)
		addMetric(group.metrics[signal.field], mobile.signal?.[signal.field]);
	updateTrafficMaximums(group.maxTraffic, sample.traffic);
	if (Number.isFinite(mobile.signal?.sinr) && mobile.signal.sinr < 5)
		group.lowSinr += 1;
	lastSuccessfulGroupKey = info.key;
	contextGroupInterrupted = false;
}

function accountPreviousDuration(sample) {
	if (lastSample?.status !== "success") return;
	const elapsed = sample.collected_at_ms - lastSample.collected_at_ms;
	if (!Number.isFinite(elapsed) || elapsed <= 0) return;
	estimatedIntervalMs = elapsed;
	if (lastSample.mobile.carrier_radio_statistics_support === "supported") {
		accountCarrierDurations(lastSample.mobile, elapsed);
	}
	if (hasSignalMeasurements(lastSample.mobile)) {
		const previous = groups.get(contextInfo(lastSample.mobile).key);
		if (previous) previous.durationMs += elapsed;
	}
}

function eventDetails(before, after) {
	return `${display(before)} → ${display(after)}`;
}

function addRadioEvent(timestamp, kind, title, details) {
	radioEvents.push({ timestamp, kind, title, details });
	if (radioEvents.length > 500) radioEvents.shift();
}

function compareRadioContext(sample) {
	const mobile = sample.mobile;
	if (!lastSuccess) {
		addRadioEvent(
			sample.collected_at_ms,
			"change",
			"Initial context",
			cellLabel(mobile),
		);
		return;
	}
	const previous = lastSuccess.mobile;
	if (previous.connection_state !== mobile.connection_state) {
		addRadioEvent(
			sample.collected_at_ms,
			"change",
			"Connection state",
			eventDetails(previous.connection_state, mobile.connection_state),
		);
	}
	if (previous.primary_carrier?.network !== mobile.primary_carrier?.network) {
		addRadioEvent(
			sample.collected_at_ms,
			"change",
			"RAT changed",
			eventDetails(
				previous.primary_carrier?.network,
				mobile.primary_carrier?.network,
			),
		);
	}
	if (cellLabel(previous) !== cellLabel(mobile)) {
		addRadioEvent(
			sample.collected_at_ms,
			"change",
			"Cell handover",
			eventDetails(cellLabel(previous), cellLabel(mobile)),
		);
	}
	if (
		JSON.stringify(carrierIdentity(previous.primary_carrier)) !==
		JSON.stringify(carrierIdentity(mobile.primary_carrier))
	) {
		addRadioEvent(
			sample.collected_at_ms,
			"change",
			"Primary carrier changed",
			eventDetails(
				carrierLabel(previous.primary_carrier, true),
				carrierLabel(mobile.primary_carrier, true),
			),
		);
	}
	if (
		JSON.stringify(activeCarrierIdentities(previous)) !==
		JSON.stringify(activeCarrierIdentities(mobile))
	) {
		addRadioEvent(
			sample.collected_at_ms,
			"change",
			"Aggregation changed",
			eventDetails(
				contextInfo(previous).aggregation,
				contextInfo(mobile).aggregation,
			),
		);
	}
}

function processFailure(sample) {
	contextGroupInterrupted = true;
	if (!outage) {
		outage = {
			startedAt: sample.collected_at_ms,
			count: 0,
			kind: sample.error?.kind,
		};
		addRadioEvent(
			sample.collected_at_ms,
			"failure",
			"Polling gap",
			display(sample.error?.kind),
		);
	}
	outage.count += 1;
	updateTimelineForFailure(sample.collected_at_ms);
}

function processSuccess(sample) {
	if (outage) {
		const duration = sample.collected_at_ms - outage.startedAt;
		addRadioEvent(
			sample.collected_at_ms,
			"recovered",
			"Polling recovered",
			`${outage.count} failed sample${outage.count === 1 ? "" : "s"} · ${formatDuration(duration)}`,
		);
		outage = null;
	}
	compareRadioContext(sample);
	updateGroup(sample);
	carrierRadioStatisticsSupported =
		sample.mobile.carrier_radio_statistics_support === "supported";
	if (carrierRadioStatisticsSupported) updateCarrierGroups(sample.mobile);
	updateTimelineForSuccess(sample.mobile, sample.collected_at_ms);
	lastSuccess = sample;
}

function processSample(sample) {
	accountPreviousDuration(sample);
	if (sample.status === "success") processSuccess(sample);
	else processFailure(sample);
	lastSample = sample;
}

function timelineStateKey(value) {
	return present(value) ? String(value) : "Unknown";
}

function updateTimelineLane(lane, value, timestamp) {
	const key = timelineStateKey(value);
	let segments = timeline.get(lane);
	if (!segments) {
		segments = [];
		timeline.set(lane, segments);
	}
	const current = segments.at(-1);
	if (current?.key === key) {
		current.end = timestamp;
	} else {
		if (current) current.end = timestamp;
		segments.push({ key, start: timestamp, end: timestamp });
	}
}

function updateTimelineForFailure(timestamp) {
	for (const lane of [...BASE_LANES, ...secondaryLanes]) {
		updateTimelineLane(lane, "No data", timestamp);
	}
}

function updateTimelineForSuccess(mobile, timestamp) {
	updateTimelineLane(
		"RAT",
		mobile.primary_carrier?.network ?? "Unknown",
		timestamp,
	);
	updateTimelineLane("Cell", cellLabel(mobile), timestamp);
	updateTimelineLane("PCell", carrierLabel(mobile.primary_carrier), timestamp);

	const reported = new Map(
		secondaryCarriers(mobile).map((carrier, index) => [
			String(index + 1),
			carrier,
		]),
	);
	for (const id of reported.keys()) secondaryLanes.add(`SCell ${id}`);
	for (const lane of secondaryLanes) {
		const id = lane.slice("SCell ".length);
		const carrier = reported.get(id);
		let value = "Absent";
		if (carrier)
			value = `${carrierLabel(carrier)} · ${carrier.active === true ? "active" : "inactive"}`;
		updateTimelineLane(lane, value, timestamp);
	}
}

function contextLayout(lanes) {
	return {
		paper_bgcolor: "transparent",
		plot_bgcolor: "transparent",
		font: { color: "#94a3b8" },
		margin: { l: 105, r: 18, t: 15, b: 42 },
		hovermode: "closest",
		showlegend: false,
		uirevision: "radio-time",
		xaxis: {
			type: "date",
			gridcolor: "#273451",
			showspikes: true,
			spikemode: "across",
		},
		yaxis: {
			type: "category",
			categoryorder: "array",
			categoryarray: lanes,
			autorange: "reversed",
			gridcolor: "#273451",
			fixedrange: true,
		},
	};
}

function stringHash(text) {
	let hash = 0;
	for (let index = 0; index < text.length; index += 1)
		hash = ((hash << 5) - hash + text.charCodeAt(index)) | 0;
	return Math.abs(hash);
}

function timelineColor(lane, value) {
	const normalized = value.toLowerCase();
	if (normalized === "no data" || normalized.includes("failed"))
		return "#e11d48";
	if (
		normalized === "absent" ||
		normalized.includes("inactive") ||
		normalized === "unknown"
	)
		return "#475569";
	return PALETTE[stringHash(`${lane}:${value}`) % PALETTE.length];
}

function renderTimeline() {
	const lanes = ["Events", ...BASE_LANES, ...[...secondaryLanes].sort()];
	const traces = [];
	for (const lane of lanes.filter((name) => name !== "Events")) {
		const segments = timeline.get(lane) ?? [];
		for (let index = 0; index < segments.length; index += 1) {
			const segment = segments[index];
			const isCurrent = index === segments.length - 1;
			const end = isCurrent
				? Math.max(segment.end, segment.start + estimatedIntervalMs)
				: segment.end;
			const duration = Math.max(0, end - segment.start);
			const hover = `${escapeHover(segment.key)}<br>${formatDuration(duration)}`;
			traces.push({
				x: [new Date(segment.start), new Date(end)],
				y: [lane, lane],
				mode: "lines",
				line: { color: timelineColor(lane, segment.key), width: 17 },
				customdata: [hover, hover],
				hovertemplate: `<b>${escapeHover(lane)}</b><br>%{customdata}<extra></extra>`,
				showlegend: false,
			});
		}
	}
	if (radioEvents.length) {
		traces.push({
			x: radioEvents.map((event) => new Date(event.timestamp)),
			y: radioEvents.map(() => "Events"),
			mode: "markers",
			marker: {
				size: 9,
				symbol: "diamond",
				color: radioEvents.map((event) =>
					event.kind === "failure"
						? "#fb7185"
						: event.kind === "recovered"
							? "#86efac"
							: "#38bdf8",
				),
			},
			customdata: radioEvents.map(
				(event) =>
					`<b>${escapeHover(event.title)}</b><br>${escapeHover(event.details)}`,
			),
			hovertemplate: "%{customdata}<extra></extra>",
			showlegend: false,
		});
	}
	return Plotly.react(
		"context-chart",
		traces,
		contextLayout(lanes),
		PLOT_CONFIG,
	);
}

function wireRangeSynchronization() {
	let synchronizing = false;
	for (const chartId of ALL_TIME_CHART_IDS) {
		document.getElementById(chartId).on("plotly_relayout", (event) => {
			if (synchronizing) return;
			let update = null;
			if (
				present(event["xaxis.range[0]"]) &&
				present(event["xaxis.range[1]"])
			) {
				update = {
					"xaxis.range[0]": event["xaxis.range[0]"],
					"xaxis.range[1]": event["xaxis.range[1]"],
				};
			} else if (event["xaxis.autorange"]) {
				update = { "xaxis.autorange": true };
			}
			if (!update) return;
			synchronizing = true;
			Promise.all(
				ALL_TIME_CHART_IDS.filter((id) => id !== chartId).map((id) =>
					Plotly.relayout(id, update),
				),
			).finally(() => {
				synchronizing = false;
			});
		});
	}
}

function tableCell(row, text, className = "") {
	const cell = document.createElement("td");
	cell.textContent = display(text);
	if (className) cell.className = className;
	row.appendChild(cell);
	return cell;
}

function statusCell(row, text, active) {
	const cell = document.createElement("td");
	const badge = document.createElement("span");
	badge.className = `carrier-state ${active ? "active" : "inactive"}`;
	badge.textContent = text;
	cell.appendChild(badge);
	row.appendChild(cell);
}

function renderCarrierTable(mobile) {
	const body = document.querySelector("#carrier-table tbody");
	body.replaceChildren();
	const entries = [
		["PCell", mobile.primary_carrier, true],
		...secondaryCarriers(mobile).map((carrier, index) => [
			`SCell ${index + 1}`,
			carrier,
			carrier.active === true,
		]),
	];
	for (const [role, carrier, active] of entries) {
		const row = document.createElement("tr");
		tableCell(row, role);
		statusCell(
			row,
			role === "PCell" ? "primary" : active ? "active" : "inactive",
			active,
		);
		tableCell(row, carrier?.network);
		tableCell(row, carrierBand(carrier));
		tableCell(row, carrier?.earfcn, "numeric");
		tableCell(row, carrier?.physical_cell_id, "numeric");
		tableCell(
			row,
			directionalPair(carrier?.downlink_frequency, carrier?.uplink_frequency),
			"numeric",
		);
		tableCell(row, carrier?.bandwidth, "numeric");
		tableCell(row, carrier?.downlink_layers, "numeric");
		tableCell(
			row,
			directionalPair(carrier?.downlink_modulation, carrier?.uplink_modulation),
		);
		body.appendChild(row);
	}
}

function setText(id, value) {
	document.getElementById(id).textContent = display(value);
}

function showLatest(sample) {
	if (sample.status === "success") {
		const mobile = sample.mobile;
		setText(
			"network",
			joinPresent(
				[mobile.connection_state, mobile.primary_carrier?.network],
				" · ",
			),
		);
		setText("signal-level", mobile.signal?.level);
		setText("operator", mobile.operator);
		setText("site", siteLabel(mobile));
		setText("sector", sectorValue(mobile));
		const aggregation = aggregationLabel(mobile);
		currentAggregationText = aggregation === "—" ? null : aggregation;
		carrierCopyButton.disabled = !currentAggregationText;
		renderCarrierTable(mobile);
	} else {
		setText("network", `Poll failed · ${display(sample.error?.kind)}`);
	}
	const metadata = document.getElementById("metadata");
	if (metadata) metadata.textContent = JSON.stringify(sample, null, 2);
}

function formatDuration(milliseconds) {
	if (!Number.isFinite(milliseconds) || milliseconds < 1_000) return "<1s";
	const seconds = Math.round(milliseconds / 1_000);
	if (seconds < 60) return `${seconds}s`;
	const minutes = Math.floor(seconds / 60);
	if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return `${hours}h ${minutes % 60}m`;
	const days = Math.floor(hours / 24);
	return `${days}d ${hours % 24}h`;
}

function contextDescription(group) {
	return `${group.site} · ${group.sector}\n${group.primary}\n${group.aggregation}`;
}

function percentage(value, total) {
	return `${Number(((value / total) * 100).toFixed(1))}%`;
}

function distributionSummary(distribution, rank) {
	if (!distribution.observed) return "—";
	return [...distribution.frequencies.entries()]
		.sort(
			([left], [right]) =>
				(rank.get(left) ?? rank.size) - (rank.get(right) ?? rank.size) ||
				left.localeCompare(right),
		)
		.map(
			([value, count]) => `${value} ${percentage(count, distribution.count)}`,
		)
		.join("\n");
}

function renderCarrierStatisticsTable() {
	const body = document.querySelector("#carrier-statistics-table tbody");
	body.replaceChildren();
	if (carrierRadioStatisticsSupported === false) {
		const row = document.createElement("tr");
		const cell = tableCell(
			row,
			"The installed equipment does not support collecting carrier radio statistics.",
			"empty",
		);
		cell.colSpan = 8;
		body.appendChild(row);
		return;
	}
	const ordered = [...carrierStatistics.values()]
		.filter((group) => group.hasMeasurements)
		.sort(
			(left, right) =>
				right.durationMs - left.durationMs || right.samples - left.samples,
		);
	if (!ordered.length) {
		const row = document.createElement("tr");
		const cell = tableCell(row, "No carrier radio measurements yet.", "empty");
		cell.colSpan = 8;
		body.appendChild(row);
		return;
	}
	for (const group of ordered) {
		const row = document.createElement("tr");
		tableCell(row, group.carrier, "carrier-statistics-identity");
		tableCell(row, group.cell, "carrier-statistics-cell");
		tableCell(row, [...group.roles].sort().join(" / "));
		tableCell(row, formatDuration(group.durationMs), "numeric");
		tableCell(row, group.samples, "numeric");
		tableCell(
			row,
			distributionSummary(group.downlinkModulation, MODULATION_RANK),
			"distribution-cell",
		);
		tableCell(
			row,
			distributionSummary(group.uplinkModulation, MODULATION_RANK),
			"distribution-cell",
		);
		tableCell(
			row,
			distributionSummary(group.downlinkLayers, MIMO_LAYER_RANK),
			"distribution-cell",
		);
		body.appendChild(row);
	}
}

function renderContextTable() {
	const body = document.querySelector("#context-table tbody");
	body.replaceChildren();
	const ordered = [...groups.values()].sort(
		(left, right) =>
			right.durationMs - left.durationMs || right.samples - left.samples,
	);
	if (!ordered.length) {
		const row = document.createElement("tr");
		const cell = tableCell(row, "No successful samples yet.", "empty");
		cell.colSpan = 9;
		body.appendChild(row);
		return;
	}
	const totalDurationMs = ordered.reduce(
		(total, group) => total + group.durationMs,
		0,
	);
	for (const group of ordered) {
		const row = document.createElement("tr");
		tableCell(row, contextDescription(group), "context-cell");
		tableCell(
			row,
			observationSummary(group.durationMs, totalDurationMs),
			"numeric",
		);
		tableCell(row, group.samples, "numeric");
		tableCell(row, group.entries, "numeric");
		tableCell(row, metricSummary(group.metrics.rssi), "numeric");
		tableCell(row, metricSummary(group.metrics.rsrp), "numeric");
		tableCell(row, metricSummary(group.metrics.rsrq), "numeric");
		tableCell(row, sinrSummary(group), "numeric");
		tableCell(row, trafficMaximumSummary(group.maxTraffic), "numeric");
		body.appendChild(row);
	}
}

function renderEventTable() {
	const body = document.querySelector("#event-table tbody");
	body.replaceChildren();
	if (!radioEvents.length) {
		const row = document.createElement("tr");
		const cell = tableCell(row, "No events yet.", "empty");
		cell.colSpan = 3;
		body.appendChild(row);
		return;
	}
	for (const event of radioEvents.slice(-100).reverse()) {
		const row = document.createElement("tr");
		tableCell(row, new Date(event.timestamp).toLocaleString());
		const kindCell = document.createElement("td");
		const badge = document.createElement("span");
		badge.className = `event-kind ${event.kind}`;
		badge.textContent = event.title;
		kindCell.appendChild(badge);
		row.appendChild(kindCell);
		tableCell(row, event.details);
		body.appendChild(row);
	}
}

function renderDerivedViews() {
	derivedRenderPending = false;
	renderCarrierStatisticsTable();
	renderContextTable();
	renderEventTable();
	renderTimeline();
}

function scheduleDerivedRender() {
	if (derivedRenderPending) return;
	derivedRenderPending = true;
	setTimeout(() => chartsReady.then(renderDerivedViews), 200);
}

function appendSamples(samples) {
	const fresh = samples
		.filter((sample) => sample.id > lastId)
		.sort((left, right) => left.id - right.id);
	if (!fresh.length) return;
	const pingSamples = [];
	for (const sample of fresh) {
		if (sample.ping) pingSamples.push(sample.ping);
	}
	const x = fresh.map((sample) => new Date(sample.collected_at_ms));
	const mobile = fresh.map((sample) =>
		sample.status === "success" ? sample.mobile : null,
	);
	Plotly.extendTraces(
		"signal-chart",
		{
			x: SIGNALS.map(() => x),
			y: SIGNALS.map((signal) =>
				mobile.map((snapshot) => snapshot?.signal?.[signal.field] ?? null),
			),
		},
		SIGNALS.map((_, index) => index),
	);
	Plotly.extendTraces(
		"traffic-chart",
		{
			x: TRAFFIC.map(() => x),
			y: TRAFFIC.map((traffic) =>
				fresh.map((sample) => {
					const bitsPerSecond = sample.traffic?.[traffic.field];
					return Number.isFinite(bitsPerSecond)
						? bitsPerSecond / BITS_PER_MEGABIT
						: null;
				}),
			),
		},
		TRAFFIC.map((_, index) => index),
	);
	Plotly.extendTraces(
		"temperature-chart",
		{
			x: [x],
			y: [mobile.map((snapshot) => snapshot?.temperature_c ?? null)],
		},
		[0],
	);
	if (PING_HOST && pingSamples.length) {
		Plotly.extendTraces(
			"traffic-chart",
			{
				x: [pingSamples.map((sample) => new Date(sample.collected_at_ms))],
				y: [
					pingSamples.map((sample) =>
						sample.status === "replies" &&
						Number.isFinite(sample.average_rtt_ms)
							? sample.average_rtt_ms
							: null,
					),
				],
			},
			[PING_TRACE_INDEX],
		);
	}
	for (const sample of fresh) processSample(sample);
	lastId = fresh.at(-1).id;
	showLatest(fresh.at(-1));
	scheduleDerivedRender();
}

const base = document.body.dataset.apiBase.replace(/\/$/, "");
const connection = document.getElementById("connection");
const carrierCopyButton = document.getElementById("copy-carriers");
const iperfButton = document.getElementById("iperf-start");
let iperfStarting = false;
let iperfPollTimer = null;

async function copyText(text) {
	if (window.isSecureContext && navigator.clipboard?.writeText) {
		await navigator.clipboard.writeText(text);
		return;
	}
	const textarea = document.createElement("textarea");
	textarea.value = text;
	textarea.style.position = "fixed";
	textarea.style.opacity = "0";
	document.body.appendChild(textarea);
	textarea.select();
	const copied = document.execCommand("copy");
	textarea.remove();
	if (!copied) throw new Error("Clipboard copy is unavailable");
}

carrierCopyButton.addEventListener("click", async () => {
	if (!currentAggregationText) return;
	clearTimeout(copyFeedbackTimer);
	try {
		await copyText(currentAggregationText);
		carrierCopyButton.textContent = "Copied";
	} catch {
		carrierCopyButton.textContent = "Copy failed";
	}
	copyFeedbackTimer = setTimeout(() => {
		carrierCopyButton.textContent = "Copy carriers";
	}, 1_500);
});

function errorMessage(error) {
	return error instanceof Error ? error.message : String(error);
}

function renderIperfStatus(snapshot) {
	if (!iperfButton) return;
	if (iperfStarting && snapshot.status === "idle") return;
	const status = document.getElementById("iperf-status");
	const output = document.getElementById("iperf-output");
	const labels = {
		idle: "Ready",
		running: "Running…",
		finished: "Finished",
		failed: "Failed",
		cancelled: "Cancelled",
	};
	status.textContent = labels[snapshot.status] ?? display(snapshot.status);
	status.className = `status ${snapshot.status}`;
	iperfButton.disabled = iperfStarting || snapshot.status === "running";
	if (snapshot.status === "running") {
		output.textContent = `Running ${snapshot.summary}…`;
	} else if (snapshot.error) {
		output.textContent = snapshot.error;
	} else if (snapshot.output?.length) {
		output.textContent = snapshot.output.join("\n");
	} else if (snapshot.status === "finished") {
		output.textContent = "The test finished without router output.";
	}
}

async function fetchIperfStatus() {
	const response = await fetch(`${base}/api/iperf3`);
	if (!response.ok)
		throw new Error(`iPerf3 status request failed: HTTP ${response.status}`);
	const snapshot = await response.json();
	renderIperfStatus(snapshot);
	return snapshot;
}

function scheduleIperfPoll(delay = 1_000) {
	clearTimeout(iperfPollTimer);
	iperfPollTimer = setTimeout(pollIperfStatus, delay);
}

async function pollIperfStatus() {
	try {
		const snapshot = await fetchIperfStatus();
		if (snapshot.status === "running") scheduleIperfPoll();
	} catch (error) {
		document.getElementById("iperf-status").textContent = "Unavailable";
		document.getElementById("iperf-status").className = "status failed";
		document.getElementById("iperf-output").textContent = errorMessage(error);
		iperfButton.disabled = true;
		scheduleIperfPoll(3_000);
	}
}

if (iperfButton) {
	iperfButton.addEventListener("click", async () => {
		iperfStarting = true;
		iperfButton.disabled = true;
		document.getElementById("iperf-status").textContent = "Starting…";
		document.getElementById("iperf-status").className = "status pending";
		try {
			const response = await fetch(`${base}/api/iperf3`, { method: "POST" });
			const snapshot = await response.json();
			iperfStarting = false;
			renderIperfStatus(snapshot);
			if (!response.ok && response.status !== 409) {
				throw new Error(
					snapshot.error ?? `iPerf3 start failed: HTTP ${response.status}`,
				);
			}
			if (snapshot.status === "running") scheduleIperfPoll();
		} catch (error) {
			iperfStarting = false;
			document.getElementById("iperf-status").textContent = "Failed";
			document.getElementById("iperf-status").className = "status failed";
			document.getElementById("iperf-output").textContent = errorMessage(error);
			iperfButton.disabled = true;
			scheduleIperfPoll();
		}
	});
	pollIperfStatus();
}

const events = new EventSource(`${base}/api/events`);
events.addEventListener("snapshot", (event) => {
	const snapshotId = Number(event.lastEventId);
	if (Number.isSafeInteger(snapshotId) && snapshotId < lastId) {
		window.location.reload();
		return;
	}
	chartsReady.then(() => appendSamples(JSON.parse(event.data)));
});
events.addEventListener("sample", (event) =>
	chartsReady.then(() => appendSamples([JSON.parse(event.data)])),
);
events.onopen = () => {
	connection.textContent = "Live";
	connection.className = "status online";
};
events.onerror = () => {
	connection.textContent = "Reconnecting…";
	connection.className = "status offline";
};
