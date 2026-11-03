//! Small helpers shared across the application.

use std::borrow::Cow;

use crate::i18n::{Locale, gettext, ngettext, pgettext};

/// `3:45` for track lengths, `1:02:03` past an hour.
pub fn format_duration_ms(ms: u32) -> String {
    let total = ms / 1000;
    let hours = total / 3600;
    let minutes = (total / 60) % 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// `2 hr 13 min` for playlist totals, `45 min 12 sec` under an hour.
pub fn format_total_ms(locale: Locale, ms: u64) -> String {
    let total = ms / 1000;
    let hours = total / 3600;
    let minutes = (total / 60) % 60;
    let seconds = total % 60;
    if hours > 0 {
        // Translators: A length of time, abbreviated. Keep {hours} and {minutes}.
        gettext(locale, "{hours} hr {minutes} min")
            .replace("{hours}", &hours.to_string())
            .replace("{minutes}", &minutes.to_string())
    } else if minutes > 0 {
        // Translators: A length of time, abbreviated. Keep {minutes} and {seconds}.
        gettext(locale, "{minutes} min {seconds} sec")
            .replace("{minutes}", &minutes.to_string())
            .replace("{seconds}", &seconds.to_string())
    } else {
        // Translators: A length of time, abbreviated. Keep {seconds}.
        gettext(locale, "{seconds} sec").replace("{seconds}", &seconds.to_string())
    }
}

/// Episode lengths read as `1 hr 12 min` or `38 min`.
pub fn format_episode_ms(locale: Locale, ms: u32) -> String {
    let minutes = ms / 60_000;
    let hours = minutes / 60;
    if hours > 0 {
        gettext(locale, "{hours} hr {minutes} min")
            .replace("{hours}", &hours.to_string())
            .replace("{minutes}", &(minutes % 60).to_string())
    } else {
        // Translators: A length of time, abbreviated. Keep {minutes}.
        gettext(locale, "{minutes} min").replace("{minutes}", &minutes.max(1).to_string())
    }
}

pub fn format_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(character);
    }
    out
}

/// `Jan 5, 2024` from an ISO-8601 timestamp or a bare date.
pub fn format_date(locale: Locale, iso: &str) -> String {
    let date = iso.get(..10).unwrap_or(iso);
    let mut parts = date.split('-');
    let (Some(year), Some(month)) = (parts.next(), parts.next()) else {
        return iso.to_string();
    };
    let day = parts.next();
    let month_name = match month {
        "01" => pgettext(locale, "month", "Jan"),
        "02" => pgettext(locale, "month", "Feb"),
        "03" => pgettext(locale, "month", "Mar"),
        "04" => pgettext(locale, "month", "Apr"),
        "05" => pgettext(locale, "month", "May"),
        "06" => pgettext(locale, "month", "Jun"),
        "07" => pgettext(locale, "month", "Jul"),
        "08" => pgettext(locale, "month", "Aug"),
        "09" => pgettext(locale, "month", "Sep"),
        "10" => pgettext(locale, "month", "Oct"),
        "11" => pgettext(locale, "month", "Nov"),
        "12" => pgettext(locale, "month", "Dec"),
        _ => return iso.to_string(),
    };
    let dated = match day.and_then(|day| day.trim_start_matches('0').parse::<u8>().ok()) {
        Some(day) => {
            // Translators: A date. {month} is an abbreviated month name; reorder as your language writes dates.
            gettext(locale, "{month} {day}, {year}").replace("{day}", &day.to_string())
        }
        // Translators: A month and year. {month} is an abbreviated month name.
        None => gettext(locale, "{month} {year}").into_owned(),
    };
    dated
        .replace("{month}", &month_name)
        .replace("{year}", year)
}

/// The window within which timestamps are rendered as relative dates (30 days).
pub const RELATIVE_DATE_LIMIT_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Returns true when `iso` is an ISO timestamp within the relative date window
/// (the past 30 days up to `now`).
pub fn is_relative_date(iso: &str, now: jiff::Timestamp) -> bool {
    let Ok(added) = iso.parse::<jiff::Timestamp>() else {
        return false;
    };
    let seconds = added.duration_until(now).as_secs_f64().floor() as i64;
    (0..RELATIVE_DATE_LIMIT_SECONDS).contains(&seconds)
}

/// `5 minutes ago` for recent ISO-8601 timestamps, otherwise the usual date.
///
/// Dates are shown relatively for their first 30 days, matching the playlist
/// table's compact, time-aware presentation. `now` is an argument so callers
/// can render against one instant and the boundary behaviour stays testable.
pub fn format_relative_date(locale: Locale, iso: &str, now: jiff::Timestamp) -> String {
    let Ok(added) = iso.parse::<jiff::Timestamp>() else {
        return format_date(locale, iso);
    };
    let seconds = added.duration_until(now).as_secs_f64().floor() as i64;
    if !(0..RELATIVE_DATE_LIMIT_SECONDS).contains(&seconds) {
        return format_date(locale, iso);
    }

    let (count, text) = if seconds < 60 {
        let count = seconds;
        let text = ngettext(
            locale,
            // Translators: How long ago a song was added. Keep {count}.
            "{count} second ago",
            "{count} seconds ago",
            count as u32,
        );
        (count, text)
    } else if seconds < 60 * 60 {
        let count = seconds / 60;
        let text = ngettext(
            locale,
            // Translators: How long ago a song was added. Keep {count}.
            "{count} minute ago",
            "{count} minutes ago",
            count as u32,
        );
        (count, text)
    } else if seconds < 24 * 60 * 60 {
        let count = seconds / (60 * 60);
        let text = ngettext(
            locale,
            // Translators: How long ago a song was added. Keep {count}.
            "{count} hour ago",
            "{count} hours ago",
            count as u32,
        );
        (count, text)
    } else if seconds < 7 * 24 * 60 * 60 {
        let count = seconds / (24 * 60 * 60);
        // Translators: How long ago a song was added. Keep {count}.
        let text = ngettext(locale, "{count} day ago", "{count} days ago", count as u32);
        (count, text)
    } else {
        let count = seconds / (7 * 24 * 60 * 60);
        let text = ngettext(
            locale,
            // Translators: How long ago a song was added. Keep {count}.
            "{count} week ago",
            "{count} weeks ago",
            count as u32,
        );
        (count, text)
    };
    text.replace("{count}", &count.to_string())
}

/// Tears the id out of `spotify:track:abc` and friends.
pub fn uri_id(uri: &str) -> Option<&str> {
    uri.rsplit(':').next().filter(|id| !id.is_empty())
}

pub fn uri_kind(uri: &str) -> Option<&str> {
    let mut parts = uri.split(':');
    parts.next()?;
    parts.next()
}

/// Spotify's radio station seeded by a song, playlist, album, or artist.
pub fn station_uri(seed: &str) -> Option<String> {
    let kind = uri_kind(seed)?;
    let id = uri_id(seed)?;
    (seed == format!("spotify:{kind}:{id}")
        && matches!(kind, "track" | "playlist" | "album" | "artist"))
    .then(|| format!("spotify:station:{kind}:{id}"))
}

/// The song, playlist, album, or artist a radio station is seeded by.
pub fn station_seed(station: &str) -> Option<String> {
    let seed = format!("spotify:{}", station.strip_prefix("spotify:station:")?);
    station_uri(&seed).is_some().then_some(seed)
}

pub fn open_spotify_url(uri: &str) -> Option<String> {
    let kind = uri_kind(uri)?;
    let id = uri_id(uri)?;
    Some(format!("https://open.spotify.com/{kind}/{id}"))
}

/// The menu-bar shape for macOS: the circle with the play triangle punched
/// out. macOS template images use only the alpha channel and paint the
/// shape themselves, black in a light menu bar and white in a dark one.
pub fn tray_template_rgba(size: usize) -> Vec<u8> {
    let mut rgba = mark_rgba(size, false);
    for pixel in rgba.as_chunks_mut::<4>().0 {
        // The triangle is the dark colour; make it a hole instead.
        if pixel[1] < 128 {
            pixel[3] = 0;
        }
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
    }
    rgba
}

/// The mark rasterised to pixels: the window icon, the trays and the logo
/// drawn in the app (`theme::logo`) all use this one picture.
///
/// It is the polished, black-backed spectrum mark of `packaging/icons` at
/// every size, with a small play glyph.
pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    mark_rgba(size, true)
}

/// Mixes two colours, `t` of the way from `a` to `b`.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// How far `p` is from the triangle `a`, `b`, `c`: zero inside it.
fn triangle_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    let edge = |a: (f32, f32), b: (f32, f32)| {
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let (px, py) = (p.0 - a.0, p.1 - a.1);
        let along = ((px * ex + py * ey) / (ex * ex + ey * ey)).clamp(0.0, 1.0);
        let (dx, dy) = (px - ex * along, py - ey * along);
        ((dx * dx + dy * dy).sqrt(), ex * py - ey * px)
    };
    let (d1, s1) = edge(a, b);
    let (d2, s2) = edge(b, c);
    let (d3, s3) = edge(c, a);
    let inside = (s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0);
    if inside { 0.0 } else { d1.min(d2).min(d3) }
}

/// The mark on a 128-unit square, matching `packaging/icons/tumlo.svg`:
/// rainbow spectrum bars across a black square with a small play glyph.
fn mark_rgba(size: usize, polished: bool) -> Vec<u8> {
    const GREEN: [f32; 3] = [30.0, 215.0, 96.0];
    const BARS: [([f32; 3], f32); 11] = [
        ([255.0, 75.0, 95.0], 60.0),
        ([255.0, 139.0, 61.0], 52.0),
        ([255.0, 211.0, 78.0], 40.0),
        ([185.0, 232.0, 87.0], 45.0),
        ([85.0, 223.0, 154.0], 25.0),
        ([53.0, 216.0, 199.0], 8.0),
        ([69.0, 190.0, 255.0], 25.0),
        ([103.0, 140.0, 255.0], 45.0),
        ([162.0, 123.0, 255.0], 40.0),
        ([214.0, 111.0, 227.0], 52.0),
        ([255.0, 102.0, 139.0], 60.0),
    ];
    const PUDDLE_TOPS: [f32; 11] = [
        96.0, 95.5, 95.0, 94.5, 93.5, 92.0, 93.5, 94.5, 95.0, 95.5, 96.0,
    ];
    let mut rgba = vec![0u8; size * size * 4];
    let unit = (size as f32 / 2.0) / 64.0;
    let origin = size as f32 / 2.0 - 64.0 * unit;
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5 - origin) / unit;
            let v = (y as f32 + 0.5 - origin) / unit;
            let distance = ((u - 64.0).powi(2) + (v - 64.0).powi(2)).sqrt();
            let coverage = if polished {
                1.0
            } else {
                ((64.0 - distance) * unit + 0.5).clamp(0.0, 1.0)
            };
            if coverage <= 0.0 {
                continue;
            }
            let mut colour = if polished { [5.0, 6.0, 9.0] } else { GREEN };
            if polished {
                for (index, (bar_colour, top)) in BARS.iter().enumerate() {
                    let left = index as f32 * 11.7;
                    let right = left + 11.0;
                    let center_x = (left + right) / 2.0;
                    let half_x = (right - left) / 2.0;
                    let center_y = (top + 110.0) / 2.0;
                    let half_y = (110.0 - top) / 2.0;
                    let radius = 5.5_f32.min(half_x);
                    let qx = (u - center_x).abs() - (half_x - radius);
                    let qy = (v - center_y).abs() - (half_y - radius);
                    let sd = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                    let fade_start = PUDDLE_TOPS[index] - 2.0;
                    let submerged = ((v - fade_start) / 7.0).clamp(0.0, 1.0);
                    let bar_opacity = 1.0 - submerged;
                    let bar_coverage = (0.5 - sd * unit).clamp(0.0, 1.0) * bar_opacity;
                    colour = mix(colour, *bar_colour, bar_coverage);
                }

                let puddle_x = (u - 64.0) / 69.0;
                let puddle_y = (v - 108.0) / 28.0;
                let puddle_radius = puddle_x * puddle_x + puddle_y * puddle_y;
                if puddle_radius < 1.0 {
                    let position =
                        (u / 128.0 * (BARS.len() - 1) as f32).clamp(0.0, (BARS.len() - 1) as f32);
                    let lower = position.floor() as usize;
                    let upper = (lower + 1).min(BARS.len() - 1);
                    let puddle_colour = mix(BARS[lower].0, BARS[upper].0, position - lower as f32);
                    let puddle_alpha = 0.42 * (1.0 - puddle_radius).powf(1.5);
                    colour = mix(colour, puddle_colour, puddle_alpha);
                }

                if (0.0..=128.0).contains(&u) {
                    let position = ((u - 5.5) / 11.7).clamp(0.0, (PUDDLE_TOPS.len() - 1) as f32);
                    let segment = (position.floor() as usize).min(PUDDLE_TOPS.len() - 2);
                    let fraction = position - segment as f32;
                    let eased = fraction * fraction * (3.0 - 2.0 * fraction);
                    let top =
                        PUDDLE_TOPS[segment] * (1.0 - eased) + PUDDLE_TOPS[segment + 1] * eased;
                    if v >= top && puddle_radius < 1.0 {
                        let position = (u / 128.0 * (BARS.len() - 1) as f32)
                            .clamp(0.0, (BARS.len() - 1) as f32);
                        let lower = position.floor() as usize;
                        let upper = (lower + 1).min(BARS.len() - 1);
                        let puddle_colour =
                            mix(BARS[lower].0, BARS[upper].0, position - lower as f32);
                        let body_alpha = 0.78 * (1.0 - puddle_radius).powf(0.8);
                        colour = mix(colour, puddle_colour, body_alpha);
                    }
                }

                if puddle_radius < 1.0 {
                    let pool_weight = (1.0 - puddle_radius).sqrt();
                    let wave_one =
                        114.0 - 3.5 * ((u - 14.0) * 0.13).sin() + 0.7 * ((u - 14.0) * 0.27).sin();
                    let wave_one_coverage =
                        ((1.9 - (v - wave_one).abs()) * unit + 0.5).clamp(0.0, 1.0) * pool_weight;
                    let base_position =
                        (u / 128.0 * BARS.len() as f32).clamp(0.0, BARS.len() as f32 - 0.001);
                    let flow_one_position = (base_position + 3.0).rem_euclid(BARS.len() as f32);
                    let flow_one_lower = flow_one_position.floor() as usize;
                    let flow_one_upper = (flow_one_lower + 1) % BARS.len();
                    let flow_one_colour = mix(
                        BARS[flow_one_lower].0,
                        BARS[flow_one_upper].0,
                        flow_one_position.fract(),
                    );
                    colour = mix(colour, flow_one_colour, wave_one_coverage * 0.32);

                    let wave_two = 117.0 + 2.5 * ((u - 14.0) * 0.13 + 1.5).sin();
                    let wave_two_coverage =
                        ((1.25 - (v - wave_two).abs()) * unit + 0.5).clamp(0.0, 1.0) * pool_weight;
                    let flow_two_position = (base_position + 6.0).rem_euclid(BARS.len() as f32);
                    let flow_two_lower = flow_two_position.floor() as usize;
                    let flow_two_upper = (flow_two_lower + 1) % BARS.len();
                    let flow_two_colour = mix(
                        BARS[flow_two_lower].0,
                        BARS[flow_two_upper].0,
                        flow_two_position.fract(),
                    );
                    colour = mix(colour, flow_two_colour, wave_two_coverage * 0.26);
                }

                let shadow = triangle_distance((u, v), (56.5, 51.5), (56.5, 77.5), (80.0, 64.5));
                let shadow_coverage = ((1.4 - shadow) * unit + 0.5).clamp(0.0, 1.0);
                colour = mix(colour, [17.0, 18.0, 24.0], shadow_coverage);
                let triangle = triangle_distance((u, v), (59.5, 55.5), (59.5, 73.5), (75.8, 64.5));
                let glyph = ((1.0 - triangle) * unit + 0.5).clamp(0.0, 1.0);
                colour = mix(colour, [248.0, 247.0, 250.0], glyph);
            } else {
                let triangle = triangle_distance((u, v), (49.2, 43.5), (49.2, 84.5), (86.1, 64.0));
                let glyph = ((5.0 - triangle) * unit + 0.5).clamp(0.0, 1.0);
                colour = mix(colour, [0.0, 0.0, 0.0], glyph);
            }
            let index = (y * size + x) * 4;
            rgba[index] = colour[0].round() as u8;
            rgba[index + 1] = colour[1].round() as u8;
            rgba[index + 2] = colour[2].round() as u8;
            rgba[index + 3] = if polished {
                255
            } else {
                (coverage * 255.0) as u8
            };
        }
    }
    rgba
}

pub fn greeting(locale: Locale) -> Cow<'static, str> {
    match local_hour() {
        5..=11 => gettext(locale, "Good morning"),
        12..=17 => gettext(locale, "Good afternoon"),
        _ => gettext(locale, "Good evening"),
    }
}

fn local_hour() -> u8 {
    jiff::Zoned::now().hour() as u8
}

/// Strips the HTML Spotify embeds in playlist descriptions.
pub fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for character in text.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// The temporary files a replacement (fastframe-file) left in `dir`, which
/// a finished or failed one never does.
#[cfg(test)]
pub(crate) fn temporary_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".tmp"))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let index = (y * size + x) * 4;
        [
            rgba[index],
            rgba[index + 1],
            rgba[index + 2],
            rgba[index + 3],
        ]
    }

    /// The icon fills its frame with a black canvas, edge-to-edge spectrum
    /// bars, a blended puddle and a clear foreground play glyph.
    #[test]
    fn the_icon_has_a_full_bleed_spectrum_and_foreground_play_glyph() {
        let (large, small) = (app_icon_rgba(128), app_icon_rgba(32));
        let background = pixel(&large, 128, 1, 1);
        assert_eq!(background, [5, 6, 9, 255]);
        let red_bar = pixel(&large, 128, 5, 70);
        assert!(
            red_bar[0] > red_bar[1] * 2,
            "spectrum bar missing: {red_bar:?}"
        );
        let broad_spectrum = pixel(&large, 128, 63, 40);
        assert!(
            broad_spectrum[1] > broad_spectrum[0] * 2,
            "spectrum should remain visible behind the small play mark: {broad_spectrum:?}"
        );
        let play = pixel(&large, 128, 68, 64);
        assert!(
            play[0] > 220 && play[1] > 220,
            "play glyph missing: {play:?}"
        );
        assert!(pixel(&small, 32, 1, 16)[0] > 180);
        assert!(
            pixel(&large, 128, 0, 64)[3] == 255,
            "black background should reach the edges"
        );
        let template = tray_template_rgba(44);
        assert_eq!(pixel(&template, 44, 24, 22)[3], 0);
        assert_eq!(pixel(&template, 44, 8, 22), [0, 0, 0, 255]);
    }

    #[test]
    fn the_icon_is_polished_at_every_size() {
        // #given the icon at a dock size and at a tray size
        let (large, small) = (app_icon_rgba(128), app_icon_rgba(32));

        // #then both have the black backing and near-touching edge bars
        assert_eq!(pixel(&large, 128, 1, 1), [5, 6, 9, 255]);
        assert!(pixel(&small, 32, 1, 16)[0] > 180);
        // #and its spectrum and play glyph remain visible at tray size
        let low = pixel(&large, 128, 64, 108);
        assert!(low[1] > low[0]);

        // #and both carry the bright triangle, a little right of centre
        for (icon, size) in [(&large, 128), (&small, 32)] {
            let centre = pixel(icon, size, size / 2 + size / 32, size / 2);
            assert!(centre[0] > 180, "triangle missing at {size}: {centre:?}");
        }

        // #and the menu-bar template remains the disc with the triangle cut out
        let template = tray_template_rgba(44);
        assert_eq!(pixel(&template, 44, 24, 22)[3], 0);
        assert_eq!(pixel(&template, 44, 8, 22), [0, 0, 0, 255]);
    }

    #[test]
    fn radio_stations_map_to_their_seeds_and_back() {
        for kind in ["track", "playlist", "album", "artist"] {
            let seed = format!("spotify:{kind}:4uLU6hMCjMI75M1A2tKUQC");
            let station = station_uri(&seed).expect("a station");
            assert_eq!(
                station,
                format!("spotify:station:{kind}:4uLU6hMCjMI75M1A2tKUQC")
            );
            assert_eq!(station_seed(&station).as_deref(), Some(seed.as_str()));
        }
        for seed in [
            "spotify:show:abc",
            "spotify:episode:abc",
            "spotify:user:me:collection",
            "spotify:track:",
            "track:abc",
        ] {
            assert_eq!(station_uri(seed), None, "{seed}");
        }
        assert_eq!(station_seed("spotify:playlist:abc"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration_ms(225_000), "3:45");
        assert_eq!(format_duration_ms(3_723_000), "1:02:03");
        assert_eq!(format_total_ms(Locale::English, 7_980_000), "2 hr 13 min");
        assert_eq!(format_total_ms(Locale::English, 2_712_000), "45 min 12 sec");
        assert_eq!(format_episode_ms(Locale::English, 4_320_000), "1 hr 12 min");
    }

    #[test]
    fn counts_and_dates() {
        assert_eq!(format_count(1_234_567), "1,234,567");
        assert_eq!(format_count(12), "12");
        assert_eq!(
            format_date(Locale::English, "2024-01-05T10:00:00Z"),
            "Jan 5, 2024"
        );
        assert_eq!(format_date(Locale::English, "2024-03"), "Mar 2024");
        assert_eq!(format_date(Locale::English, "2024"), "2024");
    }

    #[test]
    fn recent_dates_are_relative_for_the_first_month() {
        let now: jiff::Timestamp = "2026-08-31T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_relative_date(Locale::English, "2026-08-31T11:59:30Z", now),
            "30 seconds ago"
        );
        assert_eq!(
            format_relative_date(Locale::English, "2026-08-31T11:59:00Z", now),
            "1 minute ago"
        );
        assert_eq!(
            format_relative_date(Locale::English, "2026-08-31T11:00:00Z", now),
            "1 hour ago"
        );
        assert_eq!(
            format_relative_date(Locale::English, "2026-08-30T12:00:00Z", now),
            "1 day ago"
        );
        assert_eq!(
            format_relative_date(Locale::English, "2026-08-17T12:00:00Z", now),
            "2 weeks ago"
        );
        assert_eq!(
            format_relative_date(Locale::English, "2026-08-01T12:00:00Z", now),
            "Aug 1, 2026"
        );
        assert!(is_relative_date("2026-08-31T11:59:30Z", now));
        assert!(is_relative_date("2026-08-31T11:00:00Z", now));
        assert!(is_relative_date("2026-08-30T12:00:00Z", now));
        assert!(is_relative_date("2026-08-17T12:00:00Z", now));
        assert!(!is_relative_date("2026-08-01T12:00:00Z", now));
        assert!(!is_relative_date("2026-09-01T12:00:00Z", now));
        assert!(!is_relative_date("not-a-date", now));

        // Relative date detection is independent of locale wording.
        let pt = format_relative_date(Locale::PortugueseBrazil, "2026-08-31T11:59:30Z", now);
        assert!(!pt.ends_with(" ago"));
        assert!(is_relative_date("2026-08-31T11:59:30Z", now));
    }

    #[test]
    fn dates_and_lengths_follow_the_interface_language() {
        let now: jiff::Timestamp = "2026-08-31T12:00:00Z".parse().unwrap();
        assert_eq!(format_date(Locale::Spanish, "2024-01-05"), "5 ene 2024");
        assert_eq!(format_date(Locale::Spanish, "2024-09"), "sept 2024");
        assert_eq!(format_total_ms(Locale::Spanish, 7_980_000), "2 h 13 min");
        assert_eq!(format_episode_ms(Locale::Spanish, 2_280_000), "38 min");
        for (added, expected) in [
            ("2026-08-31T11:59:59Z", "hace 1 segundo"),
            ("2026-08-31T11:58:00Z", "hace 2 minutos"),
            ("2026-08-30T12:00:00Z", "hace 1 día"),
            ("2026-08-17T12:00:00Z", "hace 2 semanas"),
        ] {
            assert_eq!(format_relative_date(Locale::Spanish, added, now), expected);
        }
        // Each language orders the date its own way.
        assert_eq!(format_date(Locale::Swedish, "2024-01-05"), "5 jan. 2024");
        assert_eq!(format_date(Locale::English, "2024-01-05"), "Jan 5, 2024");
        assert_eq!(
            format_relative_date(Locale::Swedish, "2026-08-30T12:00:00Z", now),
            "för 1 dag sedan"
        );
        assert_eq!(format_date(Locale::Turkish, "2024-01-05"), "5 Oca 2024");
        assert_eq!(format_date(Locale::Turkish, "2024-09"), "Eyl 2024");
        assert_eq!(format_total_ms(Locale::Turkish, 7_980_000), "2 sa 13 dk");
        assert_eq!(format_episode_ms(Locale::Turkish, 2_280_000), "38 dk");
        for (added, expected) in [
            ("2026-08-31T11:59:59Z", "1 saniye önce"),
            ("2026-08-31T11:58:00Z", "2 dakika önce"),
            ("2026-08-30T12:00:00Z", "1 gün önce"),
            ("2026-08-17T12:00:00Z", "2 hafta önce"),
        ] {
            assert_eq!(format_relative_date(Locale::Turkish, added, now), expected);
        }
        assert_eq!(
            format_date(Locale::Ukrainian, "2024-01-05"),
            "5 січ. 2024 р."
        );
        assert_eq!(format_date(Locale::Ukrainian, "2024-09"), "вер. 2024 р.");
        assert_eq!(format_total_ms(Locale::Ukrainian, 7_980_000), "2 год 13 хв");
        assert_eq!(format_episode_ms(Locale::Ukrainian, 2_280_000), "38 хв");
        for (added, expected) in [
            ("2026-08-31T11:59:59Z", "1 секунду тому"),
            ("2026-08-31T11:58:00Z", "2 хвилини тому"),
            ("2026-08-31T07:00:00Z", "5 годин тому"),
            ("2026-08-30T12:00:00Z", "1 день тому"),
            ("2026-08-17T12:00:00Z", "2 тижні тому"),
        ] {
            assert_eq!(
                format_relative_date(Locale::Ukrainian, added, now),
                expected
            );
        }
        assert_eq!(format_date(Locale::Korean, "2024-01-05"), "2024년 1월 5일");
        assert_eq!(format_date(Locale::Korean, "2024-09"), "2024년 9월");
        assert_eq!(format_total_ms(Locale::Korean, 7_980_000), "2시간 13분");
        assert_eq!(format_episode_ms(Locale::Korean, 2_280_000), "38분");
        for (added, expected) in [
            ("2026-08-31T11:59:59Z", "1초 전"),
            ("2026-08-31T11:58:00Z", "2분 전"),
            ("2026-08-31T07:00:00Z", "5시간 전"),
            ("2026-08-30T12:00:00Z", "1일 전"),
            ("2026-08-17T12:00:00Z", "2주 전"),
        ] {
            assert_eq!(format_relative_date(Locale::Korean, added, now), expected);
        }
    }

    #[test]
    fn relative_dates_fall_back_for_future_and_invalid_timestamps() {
        let now: jiff::Timestamp = "2026-08-31T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_relative_date(Locale::English, "2026-09-01T12:00:00Z", now),
            "Sep 1, 2026"
        );
        assert_eq!(
            format_relative_date(Locale::English, "not-a-date", now),
            "not-a-date"
        );
    }

    #[test]
    fn uris() {
        assert_eq!(uri_id("spotify:track:abc"), Some("abc"));
        assert_eq!(uri_kind("spotify:playlist:x"), Some("playlist"));
        assert_eq!(
            open_spotify_url("spotify:album:z").as_deref(),
            Some("https://open.spotify.com/album/z")
        );
    }

    #[test]
    fn html_is_stripped() {
        assert_eq!(
            strip_html("Hi <a href=\"x\">there</a> &amp; you"),
            "Hi there & you"
        );
        assert_eq!(strip_html("ONE&#x2F;TWO&#x2F;THREE"), "ONE/TWO/THREE");
    }
}
