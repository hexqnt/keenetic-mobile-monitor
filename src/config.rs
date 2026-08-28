use std::{
    net::SocketAddr,
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use keenetic_rci::{
    InterfaceId, Iperf3, IperfDirection, IperfIpVersion, IperfLimit, IperfProtocol,
    NetworkTestSource, Ping, PingIpv6,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
enum RawIperfSource {
    Address(RawIperfSourceAddress),
    Interface(RawIperfSourceInterface),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum IperfProtocolConfig {
    #[default]
    Tcp,
    Udp,
}

impl IperfProtocolConfig {
    const fn rci(self) -> IperfProtocol {
        match self {
            Self::Tcp => IperfProtocol::Tcp,
            Self::Udp => IperfProtocol::Udp,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum IperfDirectionConfig {
    #[default]
    Upload,
    Download,
}

impl IperfDirectionConfig {
    const fn rci(self) -> IperfDirection {
        match self {
            Self::Upload => IperfDirection::Upload,
            Self::Download => IperfDirection::Download,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Download => "download",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum IpVersionConfig {
    Ipv4,
    Ipv6,
}

impl IpVersionConfig {
    const fn rci(self) -> IperfIpVersion {
        match self {
            Self::Ipv4 => IperfIpVersion::Ipv4,
            Self::Ipv6 => IperfIpVersion::Ipv6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlotlySource {
    #[default]
    Cdn,
    Local,
}

pub enum PingRequest {
    Ipv4(Ping),
    Ipv6(PingIpv6),
}

impl PingRequest {
    pub fn with_interface(self, interface: InterfaceId) -> Result<Self> {
        match self {
            Self::Ipv4(request) => request
                .source(interface)
                .map(Self::Ipv4)
                .context("ping source interface is incompatible with IPv4"),
            Self::Ipv6(request) => request
                .source(interface)
                .map(Self::Ipv6)
                .context("ping source interface is incompatible with IPv6"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    router: RawRouterConfig,
    web: Option<RawWebConfig>,
    iperf3: Option<RawIperf3Config>,
    ping: Option<RawPingConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRouterConfig {
    id: String,
    url: String,
    login: String,
    password: String,
    poll_interval: String,
    interface: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWebConfig {
    #[serde(default = "default_bind")]
    bind: SocketAddr,
    output_dir: Option<PathBuf>,
    #[serde(default)]
    api_base_url: String,
    cors_origin: Option<String>,
    #[serde(default)]
    plotly_source: PlotlySource,
    #[serde(default)]
    debug: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIperf3Config {
    server: String,
    #[serde(default = "default_iperf_port")]
    port: u16,
    #[serde(default = "default_iperf_duration_seconds")]
    duration_seconds: u32,
    #[serde(default)]
    protocol: IperfProtocolConfig,
    #[serde(default)]
    direction: IperfDirectionConfig,
    ip_version: Option<IpVersionConfig>,
    bitrate: Option<u64>,
    streams: Option<u16>,
    source: Option<RawIperfSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPingConfig {
    host: String,
    interval: String,
    count: u16,
    ip_version: Option<IpVersionConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIperfSourceAddress {
    address: std::net::IpAddr,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIperfSourceInterface {
    interface: String,
}

pub struct Config {
    pub router: RouterConfig,
    pub web: Option<WebConfig>,
    pub iperf3: Option<Iperf3Config>,
    pub ping: Option<PingConfig>,
}

impl Config {
    pub async fn load(path: &Path, serve: bool) -> Result<Self> {
        let text = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read configuration from {}", path.display()))?;
        let raw: RawConfig = toml::from_str(&text).map_err(|error: toml::de::Error| {
            let location = error
                .span()
                .map_or_else(String::new, |span| format!(" at byte {}", span.start));
            anyhow!(
                "invalid configuration in {}: {}{}",
                path.display(),
                error.message(),
                location
            )
        })?;
        Self::parse(raw, serve)
    }

    fn parse(raw: RawConfig, serve: bool) -> Result<Self> {
        if raw.router.id.trim().is_empty() {
            bail!("router.id must not be empty");
        }
        if raw.router.login.is_empty() {
            bail!("router.login must not be empty");
        }
        let poll_interval =
            parse_positive_duration(&raw.router.poll_interval, "router.poll_interval")?;
        let interface = raw
            .router
            .interface
            .map(InterfaceId::new)
            .transpose()
            .context("router.interface is invalid")?;
        let web = raw.web.map(|web| WebConfig {
            bind: web.bind,
            output_dir: web.output_dir,
            api_base_url: web.api_base_url,
            cors_origin: web.cors_origin,
            plotly_source: web.plotly_source,
            debug: web.debug,
        });
        let iperf3 = raw.iperf3.map(Iperf3Config::parse).transpose()?;
        let ping = raw.ping.map(PingConfig::parse).transpose()?;
        if serve && web.is_none() {
            bail!("the web section is required in serve mode");
        }
        Ok(Self {
            router: RouterConfig {
                id: raw.router.id.into(),
                url: raw.router.url,
                login: raw.router.login,
                password: Password(raw.router.password),
                poll_interval,
                interface,
            },
            web,
            iperf3,
            ping,
        })
    }
}

pub struct RouterConfig {
    pub id: Arc<str>,
    pub url: String,
    pub login: String,
    pub password: Password,
    pub poll_interval: Duration,
    pub interface: Option<InterfaceId>,
}

pub struct Password(String);

impl Password {
    pub fn into_inner(self) -> String {
        self.0
    }
}

pub struct WebConfig {
    pub bind: SocketAddr,
    pub output_dir: Option<PathBuf>,
    pub api_base_url: String,
    pub cors_origin: Option<String>,
    pub plotly_source: PlotlySource,
    pub debug: bool,
}

#[derive(Clone)]
pub struct Iperf3Config {
    pub request: Iperf3,
    pub summary: Arc<str>,
}

impl Iperf3Config {
    fn parse(raw: RawIperf3Config) -> Result<Self> {
        let limit =
            IperfLimit::time(raw.duration_seconds).context("iperf3.duration_seconds is invalid")?;
        let mut request = Iperf3::new(&raw.server, limit).context("iperf3.server is invalid")?;
        if let Some(ip_version) = raw.ip_version {
            request = request
                .ip_version(ip_version.rci())
                .context("iperf3.ip_version conflicts with the server address")?;
        }
        request = request
            .protocol(raw.protocol.rci())
            .direction(raw.direction.rci())
            .port(raw.port)
            .context("iperf3.port is invalid")?;
        if let Some(bitrate) = raw.bitrate {
            request = request
                .bitrate(bitrate)
                .context("iperf3.bitrate is invalid")?;
        }
        if let Some(streams) = raw.streams {
            request = request
                .streams(streams)
                .context("iperf3.streams is invalid")?;
        }
        let source_label = if let Some(source) = raw.source {
            let source = match source {
                RawIperfSource::Address(source) => NetworkTestSource::from(source.address),
                RawIperfSource::Interface(source) => NetworkTestSource::interface(source.interface)
                    .context("iperf3.source.interface is invalid")?,
            };
            let label = source.to_string();
            request = request
                .source(source)
                .context("iperf3.source conflicts with iperf3.ip_version")?;
            Some(label)
        } else {
            None
        };
        let inferred_ipv6 = request
            .host()
            .address()
            .is_some_and(|address| address.is_ipv6());
        let server = if inferred_ipv6 {
            format!("[{}]", raw.server)
        } else {
            raw.server
        };
        let ip_version = match raw.ip_version {
            Some(IpVersionConfig::Ipv6) => "IPv6",
            None if inferred_ipv6 => "IPv6",
            Some(IpVersionConfig::Ipv4) | None => "IPv4",
        };
        let mut summary = vec![
            format!("{server}:{}", raw.port),
            format!("{} {}", raw.protocol.label(), raw.direction.label()),
            format!("{}s", raw.duration_seconds),
            ip_version.to_owned(),
        ];
        if let Some(bitrate) = raw.bitrate {
            summary.push(format!("{bitrate} bit/s"));
        }
        if let Some(streams) = raw.streams {
            summary.push(format!(
                "{streams} stream{}",
                if streams == 1 { "" } else { "s" }
            ));
        }
        if let Some(source) = source_label {
            summary.push(format!("source {source}"));
        }
        Ok(Self {
            request,
            summary: summary.join(" · ").into(),
        })
    }
}

pub struct PingConfig {
    pub request: PingRequest,
    pub host: Arc<str>,
    pub interval: Duration,
    pub count: NonZeroU16,
}

impl PingConfig {
    fn parse(raw: RawPingConfig) -> Result<Self> {
        let interval = parse_positive_duration(&raw.interval, "ping.interval")?;
        let count = NonZeroU16::new(raw.count).context("ping.count must be greater than zero")?;

        let address = raw.host.parse::<std::net::IpAddr>().ok();
        let ipv6 = match (address, raw.ip_version) {
            (Some(address), Some(version))
                if address.is_ipv6() != (version == IpVersionConfig::Ipv6) =>
            {
                bail!("ping.ip_version conflicts with ping.host")
            }
            (Some(address), _) => address.is_ipv6(),
            (None, Some(IpVersionConfig::Ipv6)) => true,
            (None, Some(IpVersionConfig::Ipv4) | None) => false,
        };
        let request = if ipv6 {
            PingIpv6::new(&raw.host)
                .and_then(|request| request.count(count.get()))
                .map(PingRequest::Ipv6)
        } else {
            Ping::new(&raw.host)
                .and_then(|request| request.count(count.get()))
                .map(PingRequest::Ipv4)
        }
        .context("ping configuration is invalid")?;
        Ok(Self {
            request,
            host: raw.host.into(),
            interval,
            count,
        })
    }

    pub fn with_interface(mut self, interface: InterfaceId) -> Result<Self> {
        self.request = self.request.with_interface(interface)?;
        Ok(self)
    }
}

const fn default_iperf_port() -> u16 {
    5201
}

const fn default_iperf_duration_seconds() -> u32 {
    10
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:3000"
        .parse()
        .expect("static socket address is valid")
}

fn parse_positive_duration(value: &str, field: &str) -> Result<Duration> {
    let duration = humantime::parse_duration(value)
        .with_context(|| format!("{field} is not a valid duration"))?;
    if duration.is_zero() {
        bail!("{field} must be greater than zero");
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str, serve: bool) -> Result<Config> {
        Config::parse(toml::from_str(text)?, serve)
    }

    const BASE: &str = r#"
        [router]
        id = "home"
        url = "http://192.0.2.1"
        login = "monitor"
        password = "secret"
        poll_interval = "5s"
    "#;

    #[test]
    fn collect_accepts_router_only() {
        assert!(parse(BASE, false).is_ok());
    }

    #[test]
    fn serve_requires_web_section() {
        assert!(parse(BASE, true).is_err());
    }

    #[test]
    fn serve_parses_defaults() {
        let config = parse(&format!("{BASE}\n[web]"), true).unwrap();
        let web = config.web.unwrap();
        assert_eq!(web.bind, default_bind());
        assert_eq!(web.plotly_source, PlotlySource::Cdn);
        assert!(!web.debug);
    }

    #[test]
    fn serve_parses_debug_mode() {
        let config = parse(&format!("{BASE}\n[web]\ndebug = true"), true).unwrap();
        assert!(config.web.unwrap().debug);
    }

    #[test]
    fn rejects_zero_interval() {
        assert!(parse(&BASE.replace("5s", "0s"), false).is_err());
    }

    #[test]
    fn rejects_invalid_interface_and_unknown_fields() {
        assert!(parse(&format!("{BASE}\ninterface = \"bad\\nname\""), false).is_err());
        assert!(parse(&format!("{BASE}\nunknown = true"), false).is_err());
    }

    #[test]
    fn parses_iperf3_into_a_typed_request() {
        let config = parse(
            &format!(
                r#"{BASE}
                [iperf3]
                server = "2001:db8::2"
                port = 5202
                duration_seconds = 30
                protocol = "udp"
                direction = "download"
                ip_version = "ipv6"
                bitrate = 10000000
                streams = 2
                source = {{ address = "2001:db8::1" }}
                "#
            ),
            false,
        )
        .unwrap();
        let iperf3 = config.iperf3.unwrap();
        assert_eq!(
            serde_json::to_value(iperf3.request).unwrap(),
            serde_json::json!({
                "host": "2001:db8::2",
                "ipv6": true,
                "udp": true,
                "reverse": true,
                "port": 5202,
                "bitrate": 10_000_000,
                "streams": 2,
                "time": 30,
                "source-address": "2001:db8::1"
            })
        );
        assert_eq!(
            &*iperf3.summary,
            "[2001:db8::2]:5202 · UDP download · 30s · IPv6 · 10000000 bit/s · 2 streams · source 2001:db8::1"
        );
    }

    #[test]
    fn rejects_invalid_iperf3_options() {
        for options in [
            "server = \"example.com\"\nport = 0",
            "server = \"example.com\"\nduration_seconds = 0",
            "server = \"192.0.2.1\"\nip_version = \"ipv6\"",
            "server = \"example.com\"\nsource = { interface = \"bad\\nname\" }",
        ] {
            assert!(parse(&format!("{BASE}\n[iperf3]\n{options}"), false).is_err());
        }
    }

    fn ping_request_json(request: PingRequest) -> serde_json::Value {
        match request {
            PingRequest::Ipv4(request) => serde_json::to_value(request).unwrap(),
            PingRequest::Ipv6(request) => serde_json::to_value(request).unwrap(),
        }
    }

    #[test]
    fn parses_ipv4_ipv6_and_hostname_ping_requests() {
        for (options, expected) in [
            (
                "host = \"1.1.1.1\"\ninterval = \"5s\"\ncount = 3",
                serde_json::json!({"host": "1.1.1.1", "count": 3}),
            ),
            (
                "host = \"2001:4860:4860::8888\"\ninterval = \"10s\"\ncount = 2",
                serde_json::json!({"host": "2001:4860:4860::8888", "count": 2}),
            ),
            (
                "host = \"example.com\"\ninterval = \"1m\"\ncount = 1\nip_version = \"ipv6\"",
                serde_json::json!({"host": "example.com", "count": 1}),
            ),
        ] {
            let config = parse(&format!("{BASE}\n[ping]\n{options}"), false).unwrap();
            let ping = config.ping.unwrap();
            assert_eq!(ping_request_json(ping.request), expected);
        }
    }

    #[test]
    fn ping_uses_the_selected_mobile_interface() {
        let config = parse(
            &format!("{BASE}\n[ping]\nhost = \"example.com\"\ninterval = \"5s\"\ncount = 3"),
            false,
        )
        .unwrap();
        let ping = config
            .ping
            .unwrap()
            .with_interface(InterfaceId::new("UsbLte0").unwrap())
            .unwrap();
        assert_eq!(
            ping_request_json(ping.request),
            serde_json::json!({
                "host": "example.com",
                "count": 3,
                "source": "UsbLte0"
            })
        );
    }

    #[test]
    fn rejects_invalid_ping_options() {
        for options in [
            "host = \"example.com\"\ninterval = \"5s\"",
            "host = \"example.com\"\ncount = 1",
            "interval = \"5s\"\ncount = 1",
            "host = \"example.com\"\ninterval = \"0s\"\ncount = 1",
            "host = \"example.com\"\ninterval = \"5s\"\ncount = 0",
            "host = \"192.0.2.1\"\ninterval = \"5s\"\ncount = 1\nip_version = \"ipv6\"",
            "host = \"2001:db8::1\"\ninterval = \"5s\"\ncount = 1\nip_version = \"ipv4\"",
            "host = \"example.com\"\ninterval = \"5s\"\ncount = 1\nunknown = true",
        ] {
            assert!(parse(&format!("{BASE}\n[ping]\n{options}"), false).is_err());
        }
    }

    #[tokio::test]
    async fn syntax_errors_do_not_echo_the_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        tokio::fs::write(
            &path,
            BASE.replace("password = \"secret\"", "password = \"secret-without-end"),
        )
        .await
        .unwrap();

        let error = Config::load(&path, false).await.err().unwrap();
        assert!(!format!("{error:#}").contains("secret-without-end"));
    }
}
