//! WiFi, the HTTP server, time sync and weather updates. Everything here
//! runs on its own thread so the UI never waits for the network.

use std::{sync::Arc, thread, time::Duration};

use anyhow::{anyhow, bail, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::modem::WifiModemPeripheral,
    http::{
        client::{Configuration as ClientConfig, EspHttpConnection},
        server::{Configuration as ServerConfig, EspHttpServer},
        Method,
    },
    io::Write,
    nvs::EspDefaultNvsPartition,
    sntp::EspSntp,
    sys,
    wifi::{
        AccessPointConfiguration, AuthMethod, BlockingWifi, ClientConfiguration, Configuration,
        EspWifi,
    },
};
use ui::{api, NetState};

use crate::{shared::Shared, storage::WifiCredentials};

const RETRY_DELAY: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// The forecast response is about 1.5 KB.
const WEATHER_MAX_BYTES: usize = 6144;

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
/// through `POST /api/wifi`, which restarts the device.
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
    let ip = wifi.wifi().ap_netif().get_ip_info()?.ip.to_string();
    log::info!("setup access point {ssid} is up, open http://{ip}");
    shared.set_net(NetState::Setup { ssid, password, ip });
    let _server = serve(shared)?;
    loop {
        thread::sleep(POLL_INTERVAL);
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
    wifi.start()?;
    // Modem power saving lets the radio sleep between beacons and delays
    // incoming requests. The board runs on USB power, so keep it awake.
    // SAFETY: called after the WiFi driver was started, as required.
    sys::esp!(unsafe { sys::esp_wifi_set_ps(sys::wifi_ps_type_t_WIFI_PS_NONE) })?;
    let _server = serve(shared)?;
    let mut sntp = None;

    loop {
        if !wifi.is_connected()? {
            shared.set_net(NetState::Connecting { ssid: ssid.clone() });
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
            if sntp.is_none() {
                // Kept alive for as long as the device runs; it resyncs hourly.
                sntp = Some(EspSntp::new_default()?);
            }
        }
        update_weather(shared);
        thread::sleep(POLL_INTERVAL);
    }
}

/// `Buddy-XXXX` from the MAC address, and a random 8-character password.
fn setup_network_name() -> (String, String) {
    let mut mac = [0u8; 6];
    // SAFETY: the buffer has the six bytes the function writes.
    unsafe { sys::esp_efuse_mac_get_default(mac.as_mut_ptr()) };
    let ssid = format!("Buddy-{:02X}{:02X}", mac[4], mac[5]);
    // No characters that are easily confused on a small screen.
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let password = (0..8)
        .map(|_| {
            // SAFETY: esp_random has no preconditions.
            let r = unsafe { sys::esp_random() } as usize;
            ALPHABET[r % ALPHABET.len()] as char
        })
        .collect();
    (ssid, password)
}

fn update_weather(shared: &Shared) {
    let Some(url) = shared.app().weather_due(&shared.env()) else {
        return;
    };
    // The app is not locked during the request, so the UI keeps running.
    let result = http_get(&url).and_then(|body| {
        shared
            .app()
            .set_weather(&body, &shared.env())
            .map_err(|e| anyhow!(e))
    });
    match result {
        Ok(()) => log::info!("weather updated"),
        Err(e) => log::warn!("weather update failed: {e:#}"),
    }
}

fn http_get(url: &str) -> Result<String> {
    let mut connection = EspHttpConnection::new(&ClientConfig {
        timeout: Some(Duration::from_secs(10)),
        ..Default::default()
    })?;
    connection.initiate_request(Method::Get, url, &[])?;
    connection.initiate_response()?;
    if connection.status() != 200 {
        bail!("HTTP status {}", connection.status());
    }
    let mut body = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let n = connection.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > WEATHER_MAX_BYTES {
            bail!("response larger than {WEATHER_MAX_BYTES} bytes");
        }
    }
    Ok(String::from_utf8(body)?)
}

/// Starts the HTTP server. All routes go through `ui::api`, the same code
/// the simulator serves.
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
            let path = request.uri().to_owned();
            let token = request.header(api::TOKEN_HEADER).map(str::to_owned);
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
                    },
                );
                shared.apply_effects(&mut app);
                response
            };
            let headers = [("Content-Type", response.content_type)];
            request
                .into_response(response.status, None, &headers)?
                .write_all(response.body.as_bytes())?;
            Ok(())
        })?;
    }
    Ok(server)
}
