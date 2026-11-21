//! The MQTT client. It follows the settings: set a broker and it connects,
//! change it and it reconnects, clear it and it disconnects. Received
//! messages go through `ui::mqtt`, the same code the simulator runs.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration, QoS,
};
use ui::mqtt::{Config, Message, StatePublisher, OFFLINE, ONLINE};

use crate::shared::Shared;

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
    subscribed: bool,
    publisher: StatePublisher,
}

impl MqttLink {
    pub fn new(client_id: String) -> Self {
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
        if let Some((failed, retry_at)) = &self.failed {
            if *failed == config && now < *retry_at {
                return;
            }
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
            buffer_size: 2048,
            ..Default::default()
        };

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
                    if let Err(e) = result {
                        log::warn!("MQTT {topic}: {}", e.message);
                    }
                }
                EventPayload::Received { .. } => {
                    log::warn!("MQTT message larger than the receive buffer ignored");
                }
                EventPayload::Error(e) => log::warn!("MQTT: {e}"),
                _ => {}
            }
        })?;
        Ok(Self {
            config,
            client,
            connected,
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
