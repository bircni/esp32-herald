# MQTT and HTTP

How other systems talk to Herald: MQTT topics, feed items, weather, the HTTP API and the Claude Code hook.

## MQTT

Set a broker on the web app (host, port, user, password, topic prefix) and
the device connects; changes take effect within a second. HTTP keeps working
alongside; both go through the same `Transport` trait
(`crates/ui/src/command.rs`), so a message means the same thing either way.

| Topic | Payload | Effect |
|---|---|---|
| `<prefix>/item/<id>` | item JSON, or plain text | Adds or replaces the item |
| `<prefix>/item/<id>` | empty | Removes the item |
| `<prefix>/screen/<name>` | screen JSON | Adds or replaces a custom screen; empty removes it |
| `<prefix>/weather` | weather JSON, see [Weather](#weather) | Replaces the weather; empty removes it |
| `<prefix>/view` | `home`, a screen name, or `{"view":"home"}` | Switches view and wakes the display |
| `<prefix>/clear` | anything | Empties the feed |
| `<prefix>/settings` | any subset of the settings | Validates and saves |

The device publishes:

- `<prefix>/state`: retained, the same JSON as `GET /api/state`, on changes
  and at least once a minute;
- `<prefix>/status`: `online`, retained; `offline` as last will;
- `<prefix>/error`: when a message is rejected, `{topic, status, error,
  payload}`, so the sender can see why (`mosquitto_sub -t '<prefix>/error'`);
- `<prefix>/log`: the device's warnings and errors.

The HTTP token does not apply to MQTT; the broker's login does.

### Feed items

- `id`: up to 32 of `a-z A-Z 0-9 - _ .`, e.g. `window.kitchen`.
- `text`: required (or send plain text as the whole payload).
- `color`: `red`, `yellow`, `blue` (default), `green`, `grey`, or any
  `#RRGGBB` / `[r,g,b]`, mapped to the nearest of these. Items are listed in
  that order; a red or yellow item at the top lights the LED.
- `icon`: one of the icons above, drawn instead of the colour dot.
- `detail`: up to 12 characters on the right; without it, how long the item
  has been in its state (`now`, `12m`, `1h 05m`, `2d 3h`).
- `since` (Unix time the state began) or `age` (seconds): sets that age.
  Prefer `since` for retained messages, which may arrive much later; a
  `since` before 2001 is rejected as a mistake.
- `ttl`: seconds until the item removes itself (at most a week).
- `wake`: switch the display on and show the home view. `beep`: also beep.

The feed holds up to 16 items in RAM; retained messages refill it after a
restart.

```sh
mosquitto_pub -h broker.lan -r -t herald/item/door -m 'Front door open'
mosquitto_pub -h broker.lan -r -t herald/item/window.kitchen \
  -m '{"text":"Kitchen window open","color":"#FF3B30","icon":"window","since":1795000000}'
mosquitto_pub -h broker.lan -r -t herald/item/window.kitchen -n   # closed
```

A Home Assistant automation for a window sensor:

```yaml
triggers:
  - trigger: state
    entity_id: binary_sensor.kitchen_window
actions:
  - action: mqtt.publish
    data:
      topic: herald/item/window.kitchen
      retain: true
      payload: >-
        {% if trigger.to_state.state == 'on' %}
        {"text":"Kitchen window open","color":"red","icon":"window",
         "since":{{ as_timestamp(trigger.to_state.last_changed) | int }}}
        {% endif %}
```

### Weather

The device fetches no weather itself; it shows what arrives on
`<prefix>/weather` (or `PUT /api/weather`). Only `temp` and `condition` are
required; conditions are Home Assistant's weather states (`sunny`,
`clear-night`, `partlycloudy`, `rainy`, `lightning-rainy`, …). Fields the
device does not use are ignored. Weather that is not refreshed for three hours
disappears instead of going stale.

```json
{
  "temp": 4.2, "condition": "rainy", "min": 1, "max": 6, "is_day": true,
  "sunrise": "07:38", "sunset": "16:05",
  "hourly": [{"hour": 17, "temp": 3.9, "rain": 40, "condition": "rainy", "is_day": false}],
  "daily": [{"date": "2026-11-22", "min": 1, "max": 6, "condition": "cloudy"}]
}
```

`hourly` (up to 12, local hours) feeds the forecast curve, `daily` (up to 4,
starting today) the day cards, `sunrise`/`sunset` the calendar view. A Home
Assistant automation that sends a weather entity every 15 minutes:

```yaml
triggers:
  - trigger: time_pattern
    minutes: "/15"
actions:
  - action: weather.get_forecasts
    target: { entity_id: weather.home }
    data: { type: hourly }
    response_variable: hourly
  - action: weather.get_forecasts
    target: { entity_id: weather.home }
    data: { type: daily }
    response_variable: daily
  - action: mqtt.publish
    data:
      topic: herald/weather
      retain: true
      payload: >-
        {% set w = 'weather.home' %}
        {% set ns = namespace(h=[], d=[]) %}
        {% for f in hourly[w].forecast[:12] %}
          {% set ns.h = ns.h + [{"hour": (f.datetime | as_datetime | as_local).hour,
            "temp": f.temperature, "rain": f.precipitation_probability or 0,
            "condition": f.condition}] %}
        {% endfor %}
        {% for f in daily[w].forecast[:4] %}
          {% set ns.d = ns.d + [{"date": (f.datetime | as_datetime | as_local).date() | string,
            "min": f.get('templow', f.temperature), "max": f.temperature,
            "condition": f.condition}] %}
        {% endfor %}
        {{ {"temp": state_attr(w, 'temperature'), "condition": states(w),
            "is_day": is_state('sun.sun', 'above_horizon'),
            "sunrise": as_timestamp(state_attr('sun.sun', 'next_rising')) | timestamp_custom('%H:%M'),
            "sunset": as_timestamp(state_attr('sun.sun', 'next_setting')) | timestamp_custom('%H:%M'),
            "hourly": ns.h, "daily": ns.d} | to_json }}
```

## HTTP API

Plain HTTP on the local network. If an access token is set, every request
except `GET /` and `GET /api/state` needs the header `X-Herald-Token`. The
token guards against accidents on a shared network; it is not encryption.
Errors come back as `{"error": "..."}`.

| Request | Body | Effect |
|---|---|---|
| `GET /api/state` | | Everything the device is doing right now (with `ETag`; unchanged polls get `304`) |
| `GET`, `PUT /api/settings` | any subset of the settings | Reads, or validates and saves |
| `PUT`, `DELETE /api/items/<id>` | item JSON or plain text | Adds/replaces, or removes an item |
| `GET`, `DELETE /api/items` | | The feed, or empties it |
| `GET /api/screens` | | All custom screens by name |
| `PUT`, `DELETE /api/screens/<name>` | screen JSON | Adds/replaces or removes a screen, also on MQTT (retained) |
| `PUT`, `DELETE /api/weather` | weather JSON | Replaces or removes the weather |
| `POST /api/view` | `{"view":"home"}` | Shows a view or screen and wakes the display |
| `POST /api/tap`, `/api/swipe` | `{"x","y","long"}`, `{"dir"}` | A gesture as if made on the screen |
| `POST /api/display` | `{"on":false}` | Sends the display to sleep, or wakes it |
| `GET /api/log` | | The last 80 log lines |
| `POST /api/ota` | the firmware `.bin` | Updates and restarts (device only) |
| `POST /api/restart` | | Restarts |
| `POST /api/wifi` | `{"ssid","password"}` | Saves credentials and restarts |

## Claude Code hook

`tools/claude-hook.sh working|waiting|done|clear` keeps a `claude` item in the
feed up to date (`HERALD_ITEM` changes the id, e.g. one per machine). To wire
it up, add hooks to `~/.claude/settings.json` (adjust the path and URL):

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "HERALD_URL=http://herald /path/to/tools/claude-hook.sh working" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "HERALD_URL=http://herald /path/to/tools/claude-hook.sh waiting" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "HERALD_URL=http://herald /path/to/tools/claude-hook.sh done" }] }]
  }
}
```

The script gives up after three seconds and always exits successfully, so a
switched-off display does not slow Claude Code down.
