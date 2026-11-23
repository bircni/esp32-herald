//! The MQTT client. It follows the settings: set a broker and it connects,
//! change it and it reconnects, clear it and it disconnects. Received
//! messages go through `ui::mqtt`, the same code the simulator runs.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration, QoS,
};
use ui::mqtt::{Config, Message, OFFLINE, ONLINE, StatePublisher, error_payload};

use crate::shared::Shared;

/// Error reports waiting to be published; more are dropped.
const MAX_QUEUED_ERRORS: usize = 8;
/// After a client could not be created, wait this long before trying again.
const RETRY_MS: u64 = 30_000;

pub struct MqttLink {
    client_id: String,
    running: Option<Running>,
    /// The configuration that failed to start, and when to try again.
    failed: Option<(Config, u64)>,
}

struct Running {
    config: Config,
    client: EspMqttClient<'static>,
    connected: Arc<AtomicBool>,
    /// Reports of rejected messages, published from `poll`: the client's
    /// callback cannot publish itself.
    errors: Arc<Mutex<Vec<String>>>,
    subscribed: bool,
    publisher: StatePublisher,
}

impl MqttLink {
    pub const fn new(client_id: String) -> Self {
        Self {
            client_id,
            running: None,
            failed: None,
        }
    }

    /// Call regularly from the network thread: applies settings changes,
    /// subscribes after (re)connecting and publishes the state.
    pub fn poll(&mut self, shared: &Arc<Shared>) {
        let wanted = Config::from_settings(shared.app().settings());
        if self.running.as_ref().map(|r| &r.config) != wanted.as_ref() {
            // Dropping the client disconnects it.
            self.running = None;
            shared.set_mqtt(wanted.as_ref().map(|_| false));
            if let Some(config) = wanted {
                self.start(shared, config);
            }
        }
        let Some(r) = self.running.as_mut() else {
            return;
        };
        let connected = r.connected.load(Ordering::Relaxed);
        shared.set_mqtt(Some(connected));
        if !connected {
            r.subscribed = false;
            return;
        }
        if !r.subscribed {
            match r.subscribe() {
                Ok(()) => {
                    log::info!("MQTT connected to {}", r.config.url());
                    r.subscribed = true;
                    r.publisher.reset();
                }
                Err(e) => log::warn!("MQTT subscribe failed: {e:#}"),
            }
        }
        for (name, payload) in shared.take_outbox() {
            let topic = r.config.screen_topic(&name);
            let payload = payload.unwrap_or_default();
            if let Err(e) = r
                .client
                .enqueue(&topic, QoS::AtLeastOnce, true, payload.as_bytes())
            {
                log::warn!("MQTT publish of screen {name} failed: {e}");
            }
        }
        for line in ui::logbuf::take_pending() {
            // Not logged on failure: that would queue another line.
            let _ = r.client.enqueue(
                &r.config.log_topic(),
                QoS::AtMostOnce,
                false,
                line.as_bytes(),
            );
        }
        let reports = std::mem::take(
            &mut *r
                .errors
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for report in reports {
            let topic = r.config.error_topic();
            if let Err(e) = r
                .client
                .enqueue(&topic, QoS::AtMostOnce, false, report.as_bytes())
            {
                log::warn!("MQTT error report failed: {e}");
            }
        }
        let env = shared.env();
        let payload = r.publisher.due(&shared.app(), &env);
        if let Some(payload) = payload {
            let topic = r.config.state_topic();
            if let Err(e) = r
                .client
                .enqueue(&topic, QoS::AtMostOnce, true, payload.as_bytes())
            {
                log::warn!("MQTT publish failed: {e}");
            }
        }
    }

    fn start(&mut self, shared: &Arc<Shared>, config: Config) {
        let now = shared.now_ms();
        if let Some((failed, retry_at)) = &self.failed
            && *failed == config
            && now < *retry_at
        {
            return;
        }
        match Running::new(shared, config.clone(), &self.client_id) {
            Ok(running) => {
                log::info!("MQTT connecting to {}", config.url());
                self.running = Some(running);
                self.failed = None;
            }
            Err(e) => {
                log::warn!("MQTT client for {} failed: {e:#}", config.url());
                self.failed = Some((config, now + RETRY_MS));
            }
        }
    }
}

impl Running {
    fn new(shared: &Arc<Shared>, config: Config, client_id: &str) -> Result<Self> {
        let connected = Arc::new(AtomicBool::new(false));
        let status_topic = config.status_topic();
        let mqtt_conf = MqttClientConfiguration {
            client_id: Some(client_id),
            username: (!config.user.is_empty()).then_some(config.user.as_str()),
            password: (!config.password.is_empty()).then_some(config.password.as_str()),
            keep_alive_interval: Some(Duration::from_secs(30)),
            lwt: Some(LwtConfiguration {
                topic: &status_topic,
                payload: OFFLINE.as_bytes(),
                qos: QoS::AtLeastOnce,
                retain: true,
            }),
            // Messages go through JSON parsing in the client's task.
            task_stack: 8 * 1024,
            // Custom screens may be up to 4 KB.
            buffer_size: ui::mqtt::MAX_PAYLOAD + 256,
            ..Default::default()
        };

        let errors = Arc::new(Mutex::new(Vec::new()));
        let reports = Arc::clone(&errors);
        let flag = Arc::clone(&connected);
        let shared = Arc::clone(shared);
        let transport = config.transport();
        let client = EspMqttClient::new_cb(&config.url(), &mqtt_conf, move |event| {
            match event.payload() {
                EventPayload::Connected(_) => flag.store(true, Ordering::Relaxed),
                EventPayload::Disconnected => flag.store(false, Ordering::Relaxed),
                EventPayload::Received {
                    topic: Some(topic),
                    data,
                    details: Details::Complete,
                    ..
                } => {
                    let env = shared.env();
                    let mut app = shared.app();
                    let message = Message {
                        topic,
                        payload: data,
                    };
                    let result = ui::command::handle(&transport, &mut app, &env, &message);
                    shared.apply_effects(&mut app);
                    drop(app);
                    match result {
                        Ok(_) => log::info!("MQTT {topic}: {} bytes, applied", data.len()),
                        Err(e) => {
                            let start = String::from_utf8_lossy(&data[..data.len().min(120)]);
                            log::warn!("MQTT {topic}: {} (payload starts {start:?})", e.message);
                            let mut queue = reports
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if queue.len() < MAX_QUEUED_ERRORS {
                                queue.push(error_payload(&message, &e));
                            }
                        }
                    }
                }
                EventPayload::Received { .. } => {
                    log::warn!("MQTT message larger than the receive buffer ignored");
                }
                EventPayload::Error(e) => log::warn!("MQTT: {e}"),
                EventPayload::BeforeConnect
                | EventPayload::Subscribed(_)
                | EventPayload::Unsubscribed(_)
                | EventPayload::Published(_)
                | EventPayload::Deleted(_) => {}
            }
        })?;
        Ok(Self {
            config,
            client,
            connected,
            errors,
            subscribed: false,
            publisher: StatePublisher::default(),
        })
    }

    fn subscribe(&mut self) -> Result<()> {
        for topic in self.config.subscriptions() {
            self.client.subscribe(&topic, QoS::AtLeastOnce)?;
        }
        self.client.enqueue(
            &self.config.status_topic(),
            QoS::AtLeastOnce,
            true,
            ONLINE.as_bytes(),
        )?;
        Ok(())
    }
}
