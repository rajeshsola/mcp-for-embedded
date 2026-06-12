use anyhow::{Context, Result};
use rumqttc::{AsyncClient, MqttOptions, QoS};
use socketcan::{CanFrame, CanSocket, EmbeddedFrame, Id, Socket};
use std::time::Duration;
use tokio::sync::mpsc;

// ── CAN frame decoding ────────────────────────────────────────────────────────

fn id_to_u32(id: Id) -> u32 {
    match id {
        Id::Standard(s) => s.as_raw() as u32,
        Id::Extended(e) => e.as_raw(),
    }
}

/// Decode a raw CAN frame into structured vehicle telemetry.
/// Returns `None` for unrecognised frame IDs so the agent can skip them.
fn decode_frame(frame_id: u32, data: &[u8]) -> Option<serde_json::Value> {
    let v = match frame_id {
        // 0x100: combined frame — speed (2B) + rpm (2B) + fuel (1B) + mode (1B)
        0x100 => serde_json::json!({
            "speed":        (data.len() >= 2).then(|| u16::from_be_bytes([data[0], data[1]])),
            "rpm":          (data.len() >= 4).then(|| u16::from_be_bytes([data[2], data[3]])),
            "fuel_level":   data.get(4).copied(),
            "driving_mode": data.get(5).copied(),
        }),
        0x101 => serde_json::json!({
            "speed": (data.len() >= 2).then(|| u16::from_be_bytes([data[0], data[1]])),
        }),
        0x102 => serde_json::json!({
            "rpm": (data.len() >= 2).then(|| u16::from_be_bytes([data[0], data[1]])),
        }),
        0x103 => serde_json::json!({ "fuel_level":   data.first().copied() }),
        0x104 => serde_json::json!({ "driving_mode": data.first().copied() }),
        _ => return None,
    };
    Some(v)
}

// ── Reasoning & enrichment ────────────────────────────────────────────────────

const TELEMETRY_FIELDS: &[&str] = &["speed", "rpm", "fuel_level", "driving_mode"];

struct FrameAnalysis {
    topic: String,
    payload: serde_json::Value,
}

/// Inspect decoded telemetry, decide the correct MQTT topic, and enrich the
/// payload with a timestamp and any alert flags.
fn reason(frame_id: u32, decoded: serde_json::Value) -> FrameAnalysis {
    let obj = decoded.as_object().expect("decode_frame always returns an object");

    // Collect fields that carry a real value (not JSON null).
    let present: Vec<&str> = TELEMETRY_FIELDS
        .iter()
        .copied()
        .filter(|&k| obj.get(k).is_some_and(|v| !v.is_null()))
        .collect();

    // Multiple readings → aggregate topic; single reading → specific sub-topic.
    let topic = if present.len() > 1 {
        "vehicletelemetry/all".to_string()
    } else {
        let sub = match present.first().copied().unwrap_or("unknown") {
            "speed"        => "speed",
            "rpm"          => "rpm",
            "fuel_level"   => "fuel",
            "driving_mode" => "driving_mode",
            _              => "unknown",
        };
        format!("vehicletelemetry/{sub}")
    };

    // Build enriched payload.
    let mut enriched = obj.clone();

    enriched.insert(
        "frame_id".to_string(),
        format!("0x{frame_id:X}").into(),
    );
    enriched.insert(
        "timestamp_s".to_string(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .into(),
    );

    // Threshold-based alerts appended as a reasoning annotation.
    let mut alerts: Vec<&str> = Vec::new();

    if let Some(v) = obj.get("speed").and_then(|v| v.as_u64()) {
        if v > 200 {
            alerts.push("OVERSPEED");
        }
    }
    if let Some(v) = obj.get("rpm").and_then(|v| v.as_u64()) {
        if v > 6000 {
            alerts.push("HIGH_RPM");
        }
    }
    if let Some(v) = obj.get("fuel_level").and_then(|v| v.as_u64()) {
        if v < 20 {
            alerts.push("LOW_FUEL");
        }
    }
    if let Some(v) = obj.get("driving_mode").and_then(|v| v.as_u64()) {
        let label = match v {
            0 => "Normal",
            1 => "Sport",
            2 => "Eco",
            _ => "Unknown",
        };
        enriched.insert("driving_mode_label".to_string(), label.into());
    }

    if !alerts.is_empty() {
        enriched.insert("alerts".to_string(), serde_json::json!(alerts));
    }

    FrameAnalysis {
        topic,
        payload: serde_json::Value::Object(enriched),
    }
}

// ── Agent ─────────────────────────────────────────────────────────────────────

pub struct TelemetryAgent {
    can_interface: String,
    mqtt_host: String,
    mqtt_port: u16,
}

impl TelemetryAgent {
    pub fn new(
        can_interface: impl Into<String>,
        mqtt_host: impl Into<String>,
        mqtt_port: u16,
    ) -> Self {
        Self {
            can_interface: can_interface.into(),
            mqtt_host: mqtt_host.into(),
            mqtt_port,
        }
    }

    pub async fn run(self) -> Result<()> {
        // ── MQTT connection ───────────────────────────────────────────────────
        let mut opts =
            MqttOptions::new("socketcan-telemetry-agent", &self.mqtt_host, self.mqtt_port);
        opts.set_keep_alive(Duration::from_secs(30));

        let (client, mut eventloop) = AsyncClient::new(opts, 64);

        // Drive the MQTT event loop; reconnect automatically on transient errors.
        tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("[agent] MQTT event loop error: {e}");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });

        eprintln!(
            "[agent] connected to MQTT broker {}:{}",
            self.mqtt_host, self.mqtt_port
        );

        // ── CAN reader (blocking I/O on a dedicated OS thread) ────────────────
        let (tx, mut rx) = mpsc::channel::<CanFrame>(64);
        let iface = self.can_interface.clone();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let socket = CanSocket::open(&iface)
                .with_context(|| format!("failed to open CAN interface '{iface}'"))?;
            // Short read timeout lets the thread notice when the channel is closed.
            socket
                .set_read_timeout(Duration::from_millis(200))
                .context("failed to set CAN read timeout")?;

            eprintln!("[agent] listening on CAN interface '{iface}'");

            loop {
                match socket.read_frame() {
                    Ok(frame) => {
                        if tx.blocking_send(frame).is_err() {
                            break; // main task exited — clean shutdown
                        }
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        continue // normal poll timeout
                    }
                    Err(e) => {
                        eprintln!("[agent] CAN read error: {e}");
                        break;
                    }
                }
            }
            Ok(())
        });

        // ── Processing loop ───────────────────────────────────────────────────
        while let Some(frame) = rx.recv().await {
            // Only process data frames; skip remote/error frames.
            let CanFrame::Data(data_frame) = frame else {
                continue;
            };

            let frame_id = id_to_u32(data_frame.id());

            // Decode — skip unknown frame IDs.
            let Some(decoded) = decode_frame(frame_id, data_frame.data()) else {
                eprintln!("[agent] unknown frame 0x{frame_id:X} — skipping");
                continue;
            };

            // Reason about the decoded data and determine the MQTT topic.
            let analysis = reason(frame_id, decoded);

            let payload_str =
                serde_json::to_string(&analysis.payload).context("payload serialisation failed")?;

            eprintln!("[agent] → {}  {}", analysis.topic, payload_str);

            client
                .publish(
                    &analysis.topic,
                    QoS::AtLeastOnce,
                    false,
                    payload_str.as_bytes(),
                )
                .await
                .context("MQTT publish failed")?;
        }

        eprintln!("[agent] CAN reader exited — shutting down");
        Ok(())
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let can_iface = args.next().ok_or_else(|| {
        anyhow::anyhow!(
            "usage: can-mqtt-agent <can_interface> [mqtt_host] [mqtt_port]\n\
             example: can-mqtt-agent vcan0 localhost 1883"
        )
    })?;

    let mqtt_host = args.next().unwrap_or_else(|| "localhost".to_string());

    let mqtt_port = args
        .next()
        .map(|s| s.parse::<u16>().context("MQTT port must be a number 1–65535"))
        .transpose()?
        .unwrap_or(1883);

    TelemetryAgent::new(can_iface, mqtt_host, mqtt_port).run().await
}
