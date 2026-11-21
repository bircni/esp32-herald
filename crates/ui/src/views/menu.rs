use embedded_graphics::{prelude::*, primitives::Rectangle};

use super::{contains, region, Region};
use crate::{
    app::App,
    gfx::{self, Align, Canvas, Font},
    model::{screen, Effect, Env, NetState, ThemeKind},
    theme::Theme,
};

const BRIGHTNESS: Rectangle = gfx::rect(12, 78, 456, 50);
const THEME: Rectangle = gfx::rect(12, 138, 222, 50);
const CALIBRATE: Rectangle = gfx::rect(246, 138, 222, 50);
const ROTATE: Rectangle = gfx::rect(12, 198, 222, 50);
const WIFI: Rectangle = gfx::rect(246, 198, 222, 50);
const CLOSE: Rectangle = gfx::rect(12, 258, 456, 50);

fn net_line(net: &NetState) -> String {
    match net {
        NetState::Offline => "WiFi: offline".into(),
        NetState::Connecting { ssid } => format!("Connecting to {ssid}"),
        NetState::Connected { ip } => format!("http://{ip}"),
        NetState::Setup { ssid, .. } => format!("Setup network {ssid}"),
    }
}

pub(crate) fn regions(app: &App, env: &Env) -> Vec<Region> {
    vec![region(
        screen(),
        &[
            gfx::str_key(&net_line(&env.net)),
            i64::from(app.settings.brightness),
            app.settings.theme as i64,
            i64::from(app.settings.rotate),
            i64::from(app.menu_confirm_wifi),
        ],
    )]
}

pub(crate) fn paint(app: &App, c: &mut Canvas<'_>, env: &Env, theme: &Theme) {
    gfx::text(
        c,
        Font::Large,
        "Menu",
        Point::new(16, 28),
        Align::Left,
        theme.fg,
    );
    let version = format!("deskbuddy {}", crate::VERSION);
    gfx::text(
        c,
        Font::Small,
        &version,
        Point::new(464, 28),
        Align::Right,
        theme.dim,
    );
    gfx::text(
        c,
        Font::Medium,
        &net_line(&env.net),
        Point::new(240, 58),
        Align::Center,
        theme.dim,
    );

    let entry = |c: &mut Canvas<'_>, r: Rectangle, text: &str| {
        gfx::round_rect(c, r, 14, theme.panel);
        gfx::text(c, Font::Medium, text, r.center(), Align::Center, theme.fg);
    };

    let r = BRIGHTNESS;
    gfx::round_rect(c, r, 14, theme.panel);
    let side = |x: i32| gfx::rect(x, r.top_left.y, 90, r.size.height);
    gfx::button(c, side(r.top_left.x), "-", theme.accent, theme.on_accent);
    gfx::button(
        c,
        side(r.top_left.x + r.size.width as i32 - 90),
        "+",
        theme.accent,
        theme.on_accent,
    );
    gfx::text(
        c,
        Font::Medium,
        &format!("Brightness {}%", app.settings.brightness),
        r.center(),
        Align::Center,
        theme.fg,
    );

    entry(
        c,
        THEME,
        match app.settings.theme {
            ThemeKind::Dark => "Theme: dark",
            ThemeKind::Light => "Theme: light",
        },
    );
    entry(c, CALIBRATE, "Calibrate touch");
    entry(
        c,
        ROTATE,
        if app.settings.rotate {
            "Auto-rotate: on"
        } else {
            "Auto-rotate: off"
        },
    );
    if app.menu_confirm_wifi {
        gfx::round_rect(c, WIFI, 14, theme.warn);
        gfx::text(
            c,
            Font::Small,
            "Tap again to reset WiFi",
            WIFI.center(),
            Align::Center,
            theme.on_accent,
        );
    } else {
        entry(c, WIFI, "WiFi setup");
    }
    entry(c, CLOSE, "Close");
}

pub(crate) fn tap(app: &mut App, p: Point) {
    let confirm = app.menu_confirm_wifi;
    app.menu_confirm_wifi = false;

    if contains(&BRIGHTNESS, p) {
        let step: i16 = match p.x {
            x if x < 120 => -10,
            x if x > 360 => 10,
            _ => 0,
        };
        let new = (i16::from(app.settings.brightness) + step).clamp(10, 100) as u8;
        if new != app.settings.brightness {
            app.settings.brightness = new;
            app.push_effect(Effect::SaveSettings);
        }
    } else if contains(&THEME, p) {
        app.settings.theme = match app.settings.theme {
            ThemeKind::Dark => ThemeKind::Light,
            ThemeKind::Light => ThemeKind::Dark,
        };
        app.push_effect(Effect::SaveSettings);
    } else if contains(&CALIBRATE, p) {
        app.start_calibration();
    } else if contains(&ROTATE, p) {
        app.settings.rotate = !app.settings.rotate;
        app.push_effect(Effect::SaveSettings);
    } else if contains(&WIFI, p) {
        if confirm {
            app.menu_open = false;
            app.push_effect(Effect::StartWifiSetup);
        } else {
            app.menu_confirm_wifi = true;
        }
    } else if contains(&CLOSE, p) {
        app.menu_open = false;
    }
}
