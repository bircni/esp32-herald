//! `WiFi`, the HTTP server, firmware updates, host name, time sync and MQTT.
//! Everything here runs on its own thread so the UI never waits for the
//! network.

use std::{
    ffi::CString,
    net::{Ipv4Addr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::WifiModemPeripheral,
    handle::RawHandle,
    http::{
        Method,
        server::{Configuration as ServerConfig, EspHttpConnection, EspHttpServer, Request},
    },
    io::Write,
    mdns::EspMdns,
    nvs::EspDefaultNvsPartition,
    ota::EspOta,
    sntp::{EspSntp, SntpConf},
    sys,
    wifi::{
        AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration,
        EspWifi,
    },
};
use ui::{NetState, api};

use crate::{mqtt::MqttLink, shared::Shared, storage::WifiCredentials};

const RETRY_DELAY: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Chunk size for firmware uploads; also the stack buffer, so kept small.
const OTA_CHUNK: usize = 2048;

pub fn spawn<M>(
    shared: Arc<Shared>,
    modem: M,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> Result<()>
where
    M: WifiModemPeripheral + Send + 'static,
{
    thread::Builder::new()
        .name("net".into())
        .stack_size(12 * 1024)
        .spawn(move || {
            if let Err(e) = run(&shared, modem, sysloop, nvs) {
                log::error!("network stopped: {e:#}");
                shared.set_net(NetState::Offline);
            }
        })?;
    Ok(())
}

/// The firmware slot running now, e.g. `ota_0`.
pub fn running_slot() -> Option<String> {
    let ota = EspOta::new().ok()?;
    Some(ota.get_running_slot().ok()?.label.to_string())
}

fn run<M>(
    shared: &Arc<Shared>,
    modem: M,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> Result<()>
where
    M: WifiModemPeripheral + 'static,
{
    let mut wifi = BlockingWifi::wrap(EspWifi::new(modem, sysloop.clone(), Some(nvs))?, sysloop)?;
    match shared.wifi_credentials() {
        Some(credentials) => run_station(shared, &mut wifi, &credentials),
        None => run_setup(shared, &mut wifi),
    }
}

/// No credentials yet: open an access point and wait for them to arrive
/// through `POST /api/wifi`, which restarts the device. Every DNS name
/// points at the device, so phones open the setup page by themselves.
fn run_setup(shared: &Arc<Shared>, wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    let (ssid, password) = setup_network_name();
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid: ssid
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("ssid too long"))?,
        password: password
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("password too long"))?,
        auth_method: AuthMethod::WPA2Personal,
        channel: 6,
        max_connections: 2,
        ..Default::default()
    }))?;
    wifi.start()?;
    let ip = wifi.wifi().ap_netif().get_ip_info()?.ip;
    log::info!("setup access point {ssid} is up, open http://{ip}");
    shared.set_net(NetState::Setup {
        ssid,
        password,
        ip: ip.to_string(),
    });
    let _server = serve(shared)?;
    thread::Builder::new()
        .name("dns".into())
        .stack_size(4 * 1024)
        .spawn(move || {
            if let Err(e) = captive_dns(ip) {
                log::warn!("captive portal DNS stopped: {e:#}");
            }
        })?;
    loop {
        thread::sleep(POLL_INTERVAL);
    }
}

/// Answers every DNS query with the device's address.
fn captive_dns(ip: Ipv4Addr) -> Result<()> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 53))?;
    let mut buf = [0u8; 512];
    loop {
        let (len, from) = socket.recv_from(&mut buf)?;
        if let Some(reply) = ui::dns::answer(&buf[..len], ip) {
            let _ = socket.send_to(&reply, from);
        }
    }
}

fn run_station(
    shared: &Arc<Shared>,
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    credentials: &WifiCredentials,
) -> Result<()> {
    let ssid = credentials.ssid.clone();
    wifi.set_configuration(&Configuration::Client(ClientConfiguration {
        ssid: ssid
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("ssid too long"))?,
        password: credentials
            .password
            .as_str()
            .try_into()
            .map_err(|_| anyhow!("password too long"))?,
        auth_method: if credentials.password.is_empty() {
            AuthMethod::None
        } else {
            AuthMethod::WPA2Personal
        },
        ..Default::default()
    }))?;
    let mut naming = Naming::default();
    naming.apply(shared, wifi);
    keep_radio_awake()?;
    wifi.start()?;
    let _server = serve(shared)?;
    let mut time = TimeSync::default();
    let mut mqtt = MqttLink::new(format!("herald-{}", mac_suffix()));
    let mut marked_valid = false;

    loop {
        if !wifi.is_connected()? {
            shared.set_net(NetState::Connecting { ssid: ssid.clone() });
            shared.set_rssi(None);
            if let Err(e) = wifi.connect().and_then(|()| wifi.wait_netif_up()) {
                log::warn!("WiFi connection to {ssid:?} failed: {e}");
                // A failed attempt can leave the driver half-connected.
                let _ = wifi.disconnect();
                thread::sleep(RETRY_DELAY);
                continue;
            }
            let ip = wifi.wifi().sta_netif().get_ip_info()?.ip.to_string();
            log::info!("connected to {ssid:?}, open http://{ip}");
            shared.set_net(NetState::Connected { ip });
        }
        if !marked_valid {
            // Connected once: a freshly updated firmware works, so the
            // bootloader must not roll it back.
            marked_valid = true;
            if let Err(e) = EspOta::new().and_then(|mut ota| ota.mark_running_slot_valid()) {
                log::warn!("marking the firmware valid failed: {e}");
            }
        }
        shared.set_rssi(rssi());
        time.apply(shared);
        naming.apply(shared, wifi);
        mqtt.poll(shared);
        thread::sleep(POLL_INTERVAL);
    }
}

/// Modem power saving lets the radio sleep between beacons and delays
/// incoming requests by up to seconds. The board runs on USB power, so the
/// radio stays awake, set before `WiFi` starts.
#[expect(unsafe_code, reason = "power saving is set through the ESP-IDF C API")]
fn keep_radio_awake() -> Result<()> {
    // SAFETY: called after the WiFi driver was initialised by `EspWifi::new`.
    sys::esp!(unsafe { sys::esp_wifi_set_ps(sys::wifi_ps_type_t_WIFI_PS_NONE) })?;
    Ok(())
}

/// Signal strength of the access point in dBm, while connected.
#[expect(unsafe_code, reason = "the RSSI comes from the ESP-IDF C API")]
fn rssi() -> Option<i8> {
    let mut info = sys::wifi_ap_record_t::default();
    // SAFETY: `info` is a valid, writable record for the duration of the call.
    sys::esp!(unsafe { sys::esp_wifi_sta_get_ap_info(&raw mut info) }).ok()?;
    Some(info.rssi)
}

/// Runs SNTP against the configured server, restarting it when the setting
/// changes. Once running it resyncs hourly.
#[derive(Default)]
struct TimeSync {
    server: String,
    sntp: Option<EspSntp<'static>>,
}

impl TimeSync {
    fn apply(&mut self, shared: &Shared) {
        let wanted = shared.app().settings().ntp_server.clone();
        // A server that failed is not retried until the setting changes.
        if wanted == self.server {
            return;
        }
        // Only one SNTP client can exist: drop the old one first.
        self.sntp = None;
        // ESP-IDF is configured for a single SNTP server.
        let conf = SntpConf {
            servers: [wanted.as_str()],
            ..Default::default()
        };
        match EspSntp::new(&conf) {
            Ok(sntp) => {
                log::info!("time from {wanted}");
                self.sntp = Some(sntp);
            }
            Err(e) => log::warn!("SNTP with {wanted} failed: {e}"),
        }
        self.server = wanted;
    }
}

/// Keeps the DHCP host name and the mDNS name (`<hostname>.local`) in line
/// with the settings.
#[derive(Default)]
struct Naming {
    applied: String,
    mdns: Option<EspMdns>,
}

impl Naming {
    fn apply(&mut self, shared: &Shared, wifi: &BlockingWifi<EspWifi<'static>>) {
        let wanted = shared.app().settings().hostname.clone();
        if wanted == self.applied {
            return;
        }
        // Takes effect with the next DHCP request: at connect or lease renewal.
        if let Err(e) = set_dhcp_hostname(wifi, &wanted) {
            log::warn!("setting the DHCP host name failed: {e:#}");
        }
        if let Err(e) = self.announce(&wanted) {
            log::warn!("mDNS failed: {e:#}");
        }
        log::info!("host name {wanted}");
        self.applied = wanted;
    }

    fn announce(&mut self, name: &str) -> Result<()> {
        if let Some(mdns) = &mut self.mdns {
            mdns.set_hostname(name)?;
            mdns.set_instance_name(name)?;
            return Ok(());
        }
        // The host name must be set before any service is added.
        let mut mdns = EspMdns::take()?;
        mdns.set_hostname(name)?;
        mdns.set_instance_name(name)?;
        mdns.add_service(None, "_http", "_tcp", 80, &[])?;
        self.mdns = Some(mdns);
        Ok(())
    }
}

#[expect(
    unsafe_code,
    reason = "the DHCP host name is set through the ESP-IDF C API"
)]
fn set_dhcp_hostname(wifi: &BlockingWifi<EspWifi<'static>>, name: &str) -> Result<()> {
    let name = CString::new(name)?;
    // SAFETY: the netif handle is valid while `wifi` lives, and ESP-IDF
    // copies the name.
    sys::esp!(unsafe {
        sys::esp_netif_set_hostname(wifi.wifi().sta_netif().handle(), name.as_ptr())
    })?;
    Ok(())
}

/// The last two bytes of the MAC address, e.g. `67EC`.
#[expect(unsafe_code, reason = "the MAC address comes from the ESP-IDF C API")]
fn mac_suffix() -> String {
    let mut mac = [0u8; 6];
    // SAFETY: the buffer has the six bytes the function writes.
    unsafe { sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) };
    format!("{:02X}{:02X}", mac[4], mac[5])
}

/// `Herald-XXXX` from the MAC address, and a random 8-character password.
#[expect(unsafe_code, reason = "random numbers come from the ESP-IDF C API")]
fn setup_network_name() -> (String, String) {
    // No characters that are easily confused on a small screen.
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let ssid = format!("Herald-{}", mac_suffix());
    let password = (0..8)
        .map(|_| {
            // SAFETY: esp_random has no preconditions.
            let r = unsafe { sys::esp_random() } as usize;
            char::from(ALPHABET[r % ALPHABET.len()])
        })
        .collect();
    (ssid, password)
}

/// HTTP load since the last log line, to see what keeps the device busy.
pub struct HttpStats {
    requests: AtomicU32,
    bytes: AtomicU32,
    worst_ms: AtomicU32,
}

pub static HTTP_STATS: HttpStats = HttpStats {
    requests: AtomicU32::new(0),
    bytes: AtomicU32::new(0),
    worst_ms: AtomicU32::new(0),
};

impl HttpStats {
    fn record(&self, bytes: usize, ms: u32) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes as u32, Ordering::Relaxed);
        self.worst_ms.fetch_max(ms, Ordering::Relaxed);
    }

    /// A summary for the log, and starts counting again.
    pub fn take(&self) -> String {
        format!(
            "http: {} requests, {} bytes sent, {} ms slowest",
            self.requests.swap(0, Ordering::Relaxed),
            self.bytes.swap(0, Ordering::Relaxed),
            self.worst_ms.swap(0, Ordering::Relaxed),
        )
    }
}

type HttpRequest<'a, 'b> = Request<&'a mut EspHttpConnection<'b>>;

/// Writes the new firmware from the request body into the other slot and
/// restarts into it. The bootloader rolls back if it never connects.
fn update_firmware(shared: &Shared, request: &mut HttpRequest<'_, '_>) -> Result<usize> {
    let token = request.header(api::TOKEN_HEADER);
    if !api::authorized(&shared.app(), token) {
        bail!("missing or wrong {} header", api::TOKEN_HEADER);
    }
    let mut ota = EspOta::new()?;
    let mut update = ota.initiate_update()?;
    let mut buf = [0u8; OTA_CHUNK];
    let mut total = 0;
    loop {
        let n = request.read(&mut buf)?;
        if n == 0 {
            break;
        }
        update.write(&buf[..n])?;
        total += n;
    }
    if total == 0 {
        bail!("the request body is empty; send the firmware .bin");
    }
    // Checks the image before the slot is switched.
    update.complete()?;
    log::info!("firmware update of {total} bytes written, restarting");
    shared.schedule_restart();
    Ok(total)
}

/// Starts the HTTP server. All routes but firmware updates go through
/// `ui::api`, the same code the simulator serves.
fn serve(shared: &Arc<Shared>) -> Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&ServerConfig {
        stack_size: 10 * 1024,
        max_sessions: 4,
        max_open_sockets: 4,
        uri_match_wildcard: true,
        lru_purge_enable: true,
        ..Default::default()
    })?;
    for (method, name) in [
        (Method::Get, "GET"),
        (Method::Post, "POST"),
        (Method::Put, "PUT"),
        (Method::Delete, "DELETE"),
    ] {
        let shared = Arc::clone(shared);
        server.fn_handler::<anyhow::Error, _>("/*", method, move |mut request| {
            let started = Instant::now();
            let path = request.uri().to_owned();

            if name == "POST" && path == api::OTA_PATH {
                let (status, body) = match update_firmware(&shared, &mut request) {
                    Ok(bytes) => (200, format!(r#"{{"ok":true,"bytes":{bytes}}}"#)),
                    Err(e) => {
                        log::warn!("firmware update failed: {e:#}");
                        let message = serde_json::json!({ "error": format!("{e:#}") });
                        (400, message.to_string())
                    }
                };
                request
                    .into_response(status, None, &[("Content-Type", "application/json")])?
                    .write_all(body.as_bytes())?;
                return Ok(());
            }

            let token = request.header(api::TOKEN_HEADER).map(str::to_owned);
            let if_none_match = request.header("If-None-Match").map(str::to_owned);
            let length: usize = request
                .header("Content-Length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            // Read one byte more than allowed so oversized bodies are
            // rejected by the API instead of being silently cut off.
            let mut body = vec![0u8; length.min(api::MAX_BODY + 1)];
            let mut filled = 0;
            while filled < body.len() {
                let n = request.read(&mut body[filled..])?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            body.truncate(filled);

            let response = {
                let env = shared.env();
                let mut app = shared.app();
                let response = api::handle(
                    &mut app,
                    &env,
                    &api::Request {
                        method: name,
                        path: &path,
                        body: &body,
                        token: token.as_deref(),
                        if_none_match: if_none_match.as_deref(),
                    },
                );
                shared.apply_effects(&mut app);
                drop(app);
                response
            };
            let headers: Vec<(&str, &str)> =
                std::iter::once(("Content-Type", response.content_type))
                    .chain(response.headers.iter().map(|(n, v)| (*n, v.as_str())))
                    .collect();
            request
                .into_response(response.status, None, &headers)?
                .write_all(&response.body)?;
            let ms = started.elapsed().as_millis() as u32;
            HTTP_STATS.record(response.body.len(), ms);
            if ms > 2_000 {
                log::warn!("HTTP {name} {path} took {ms} ms");
            }
            Ok(())
        })?;
    }
    Ok(server)
}
